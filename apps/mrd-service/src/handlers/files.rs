use crate::app_state::AppState;
use mrd_ipc::{
    CapabilityStatus, DirectoryList, FileEntry, FileEntryKind, FileTransferConflictPolicy,
    FileTransferEntry, FileTransferProviderDescriptor, FileTransferProviderHandoffHint,
    FileTransferStartRequest, FileTransferStatus, FileTransferTaskSnapshot, IpcResponse,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

const LOCAL_FILE_TRANSFER_PROVIDER: &str = "mrd-local";
const LOCAL_FILE_TRANSFER_CAPABILITY: &str = "service.file_transfer.local";
const RFILE_FILE_TRANSFER_PROVIDER: &str = "r-file";
const EXTERNAL_FILE_TRANSFER_BRIDGE_CAPABILITY: &str = "service.file_transfer.external_bridge";
const EXTERNAL_FILE_TRANSFER_BRIDGE_REASON: &str =
    "reserved handoff to R-File; MRD keeps local copy/list/cancel as the active path";
const RFILE_BRIDGE_CONTROL_ENDPOINT: &str = "http://127.0.0.1:18100";
const RFILE_WATCH_DATA_ENDPOINT: &str = "http://127.0.0.1:18080";

pub fn list_directory(path: Option<String>) -> IpcResponse {
    match read_directory(path) {
        Ok(listing) => IpcResponse::DirectoryList { listing },
        Err(error) => IpcResponse::Error {
            code: "E_LIST_DIRECTORY".to_string(),
            message: error,
        },
    }
}

pub async fn start_file_transfer(
    app_state: &Arc<AppState>,
    request: FileTransferStartRequest,
) -> IpcResponse {
    if request.entries.is_empty() {
        return IpcResponse::Error {
            code: "E_FILE_TRANSFER_EMPTY".to_string(),
            message: "file transfer request is empty".to_string(),
        };
    }
    if let Some(provider_hint) = unsupported_file_transfer_provider_hint(&request.provider_hint) {
        return IpcResponse::Error {
            code: "E_FILE_TRANSFER_PROVIDER_UNAVAILABLE".to_string(),
            message: format!(
                "file transfer provider hint `{provider_hint}` is reserved for R-File handoff via rfile-bridge ({RFILE_BRIDGE_CONTROL_ENDPOINT}); {EXTERNAL_FILE_TRANSFER_BRIDGE_CAPABILITY} is not implemented in MRD"
            ),
        };
    }

    let transfer_id = {
        let mut registry = app_state.file_transfers.lock().await;
        registry.allocate_transfer_id()
    };
    let transport_kind = request
        .transport_hint
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("local")
        .to_string();
    let total_bytes = estimate_transfer_bytes(&request.entries).ok();
    let mut transfer = FileTransferTaskSnapshot {
        transfer_id,
        status: FileTransferStatus::Queued,
        source_device_id: request.source_device_id.clone(),
        target_device_id: request.target_device_id.clone(),
        transport_kind,
        provider_kind: LOCAL_FILE_TRANSFER_PROVIDER.to_string(),
        provider_capabilities: vec![LOCAL_FILE_TRANSFER_CAPABILITY.to_string()],
        total_entries: request.entries.len(),
        copied_entries: 0,
        total_bytes,
        copied_bytes: 0,
        error: None,
        entries: Vec::new(),
    };

    {
        let mut registry = app_state.file_transfers.lock().await;
        registry.upsert(transfer.clone());
    }

    transfer.status = FileTransferStatus::Running;
    {
        let mut registry = app_state.file_transfers.lock().await;
        registry.upsert(transfer.clone());
    }

    let result = tokio::task::spawn_blocking(move || execute_file_transfer(request)).await;
    match result {
        Ok(Ok(result)) => {
            transfer.status = FileTransferStatus::Completed;
            transfer.copied_entries = result.copied_entries;
            transfer.copied_bytes = result.copied_bytes;
            transfer.entries = result.entries;
        }
        Ok(Err(error)) => {
            transfer.status = FileTransferStatus::Failed;
            transfer.error = Some(error);
        }
        Err(error) => {
            transfer.status = FileTransferStatus::Failed;
            transfer.error = Some(format!("file transfer task failed: {error}"));
        }
    }

    {
        let mut registry = app_state.file_transfers.lock().await;
        registry.upsert(transfer.clone());
    }

    IpcResponse::FileTransferStarted { transfer }
}

pub async fn list_file_transfers(app_state: &Arc<AppState>) -> IpcResponse {
    IpcResponse::FileTransferList {
        transfers: app_state.file_transfers.lock().await.list(),
    }
}

pub fn list_file_transfer_providers() -> IpcResponse {
    IpcResponse::FileTransferProviderList {
        providers: file_transfer_provider_descriptors(),
    }
}

pub async fn cancel_file_transfer(app_state: &Arc<AppState>, transfer_id: String) -> IpcResponse {
    match app_state.file_transfers.lock().await.cancel(&transfer_id) {
        Some(transfer) => IpcResponse::FileTransferCancelled { transfer },
        None => IpcResponse::Error {
            code: "E_FILE_TRANSFER_NOT_FOUND".to_string(),
            message: format!("file transfer not found: {transfer_id}"),
        },
    }
}

fn file_transfer_provider_descriptors() -> Vec<FileTransferProviderDescriptor> {
    vec![
        FileTransferProviderDescriptor {
            provider_kind: LOCAL_FILE_TRANSFER_PROVIDER.to_string(),
            display_name: "MRD local file transfer".to_string(),
            status: CapabilityStatus::Available,
            capabilities: vec![LOCAL_FILE_TRANSFER_CAPABILITY.to_string()],
            reason: None,
            handoff_hint: None,
        },
        FileTransferProviderDescriptor {
            provider_kind: RFILE_FILE_TRANSFER_PROVIDER.to_string(),
            display_name: "R-File external bridge".to_string(),
            status: CapabilityStatus::Unimplemented,
            capabilities: vec![EXTERNAL_FILE_TRANSFER_BRIDGE_CAPABILITY.to_string()],
            reason: Some(EXTERNAL_FILE_TRANSFER_BRIDGE_REASON.to_string()),
            handoff_hint: Some(FileTransferProviderHandoffHint {
                external_app: "R-File".to_string(),
                bridge_service: "rfile-bridge".to_string(),
                control_endpoint: Some(RFILE_BRIDGE_CONTROL_ENDPOINT.to_string()),
                data_endpoint: Some(RFILE_WATCH_DATA_ENDPOINT.to_string()),
                capabilities: vec![
                    "rfile.bridge.session_v1".to_string(),
                    "rfile.watch.http_v1".to_string(),
                    "rfile.remote_mount.v1".to_string(),
                    "rfile.transfer_history.v1".to_string(),
                ],
            }),
        },
    ]
}

fn unsupported_file_transfer_provider_hint(provider_hint: &Option<String>) -> Option<String> {
    let hint = provider_hint.as_deref()?.trim();
    if hint.is_empty() {
        return None;
    }
    let normalized = hint.to_ascii_lowercase();
    match normalized.as_str() {
        LOCAL_FILE_TRANSFER_PROVIDER | "local" | LOCAL_FILE_TRANSFER_CAPABILITY => None,
        _ => Some(hint.to_string()),
    }
}

struct FileTransferCopyResult {
    copied_entries: usize,
    copied_bytes: u64,
    entries: Vec<FileEntry>,
}

fn execute_file_transfer(
    request: FileTransferStartRequest,
) -> Result<FileTransferCopyResult, String> {
    let target_dir = PathBuf::from(&request.target_path)
        .canonicalize()
        .map_err(|error| format!("resolve transfer target failed: {error}"))?;
    if !target_dir.is_dir() {
        return Err(format!(
            "transfer target is not a directory: {}",
            target_dir.display()
        ));
    }

    let mut copied_entries = 0usize;
    let mut copied_bytes = 0u64;
    let mut entries = Vec::with_capacity(request.entries.len());

    for entry in request.entries {
        let copied = copy_transfer_entry(&entry, &target_dir, request.conflict_policy)?;
        copied_bytes = copied_bytes.saturating_add(copied.size_bytes.unwrap_or(0));
        copied_entries = copied_entries.saturating_add(1);
        entries.push(copied);
    }

    Ok(FileTransferCopyResult {
        copied_entries,
        copied_bytes,
        entries,
    })
}

fn copy_transfer_entry(
    entry: &FileTransferEntry,
    target_dir: &Path,
    conflict_policy: FileTransferConflictPolicy,
) -> Result<FileEntry, String> {
    let source = PathBuf::from(&entry.source_path)
        .canonicalize()
        .map_err(|error| format!("resolve transfer source failed: {error}"))?;
    let metadata = fs::metadata(&source)
        .map_err(|error| format!("read transfer source metadata failed: {error}"))?;
    let file_name = transfer_file_name(entry, &source)?;
    let output_path = resolve_transfer_output_path(target_dir, &file_name, conflict_policy)?;

    match entry.kind {
        FileEntryKind::File => {
            if !metadata.is_file() {
                return Err(format!(
                    "transfer source is not a file: {}",
                    source.display()
                ));
            }
            copy_file_entry(&source, &output_path, conflict_policy)?;
        }
        FileEntryKind::Directory => {
            if !metadata.is_dir() {
                return Err(format!(
                    "transfer source is not a directory: {}",
                    source.display()
                ));
            }
            copy_directory_entry(&source, &output_path, conflict_policy)?;
        }
        FileEntryKind::Symlink | FileEntryKind::Other => {
            return Err(format!(
                "unsupported transfer entry kind for {}",
                source.display()
            ));
        }
    }

    let copied_metadata = fs::metadata(&output_path)
        .map_err(|error| format!("read copied entry metadata failed: {error}"))?;
    file_entry_from_metadata(output_path, copied_metadata)
}

fn copy_file_entry(
    source: &Path,
    output_path: &Path,
    conflict_policy: FileTransferConflictPolicy,
) -> Result<(), String> {
    if same_existing_path(source, output_path) {
        return Err(format!(
            "transfer source and target are the same file: {}",
            source.display()
        ));
    }
    remove_existing_for_replace(output_path, conflict_policy)?;
    fs::copy(source, output_path).map_err(|error| format!("copy file failed: {error}"))?;
    Ok(())
}

fn copy_directory_entry(
    source: &Path,
    output_path: &Path,
    conflict_policy: FileTransferConflictPolicy,
) -> Result<(), String> {
    if same_existing_path(source, output_path) {
        return Err(format!(
            "transfer source and target are the same directory: {}",
            source.display()
        ));
    }
    remove_existing_for_replace(output_path, conflict_policy)?;
    copy_directory_recursive(source, output_path)
}

fn copy_directory_recursive(source: &Path, output_path: &Path) -> Result<(), String> {
    fs::create_dir_all(output_path).map_err(|error| format!("create directory failed: {error}"))?;
    for entry in fs::read_dir(source).map_err(|error| format!("read directory failed: {error}"))? {
        let entry = entry.map_err(|error| format!("read directory entry failed: {error}"))?;
        let child_source = entry.path();
        let child_output = output_path.join(entry.file_name());
        let metadata = entry
            .metadata()
            .map_err(|error| format!("read directory entry metadata failed: {error}"))?;
        if metadata.is_dir() {
            copy_directory_recursive(&child_source, &child_output)?;
        } else if metadata.is_file() {
            fs::copy(&child_source, &child_output)
                .map_err(|error| format!("copy directory file failed: {error}"))?;
        } else {
            return Err(format!(
                "unsupported directory transfer entry: {}",
                child_source.display()
            ));
        }
    }
    Ok(())
}

fn transfer_file_name(entry: &FileTransferEntry, source: &Path) -> Result<String, String> {
    let file_name = entry
        .file_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            source
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::to_string)
        })
        .ok_or_else(|| format!("transfer source has no file name: {}", source.display()))?;
    validate_transfer_file_name(&file_name)?;
    Ok(file_name)
}

fn validate_transfer_file_name(file_name: &str) -> Result<(), String> {
    if file_name.trim().is_empty()
        || file_name == "."
        || file_name == ".."
        || file_name.contains('/')
        || file_name.contains('\\')
    {
        return Err(format!("invalid transfer file name: {file_name}"));
    }
    Ok(())
}

fn resolve_transfer_output_path(
    target_dir: &Path,
    file_name: &str,
    conflict_policy: FileTransferConflictPolicy,
) -> Result<PathBuf, String> {
    let preferred = target_dir.join(file_name);
    match conflict_policy {
        FileTransferConflictPolicy::Reject if preferred.exists() => Err(format!(
            "transfer target already exists: {}",
            preferred.display()
        )),
        FileTransferConflictPolicy::Reject | FileTransferConflictPolicy::Replace => Ok(preferred),
        FileTransferConflictPolicy::Rename => Ok(unique_transfer_output_path(&preferred)),
    }
}

fn unique_transfer_output_path(preferred: &Path) -> PathBuf {
    if !preferred.exists() {
        return preferred.to_path_buf();
    }
    let parent = preferred.parent().unwrap_or_else(|| Path::new(""));
    let stem = preferred
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("copy");
    let extension = preferred.extension().and_then(|value| value.to_str());
    for index in 1..10_000 {
        let name = match extension {
            Some(extension) if !extension.is_empty() => format!("{stem} ({index}).{extension}"),
            _ => format!("{stem} ({index})"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    preferred.to_path_buf()
}

fn remove_existing_for_replace(
    output_path: &Path,
    conflict_policy: FileTransferConflictPolicy,
) -> Result<(), String> {
    if conflict_policy != FileTransferConflictPolicy::Replace || !output_path.exists() {
        return Ok(());
    }
    let metadata = fs::metadata(output_path)
        .map_err(|error| format!("read replace target metadata failed: {error}"))?;
    if metadata.is_dir() {
        fs::remove_dir_all(output_path)
            .map_err(|error| format!("replace target directory failed: {error}"))?;
    } else {
        fs::remove_file(output_path)
            .map_err(|error| format!("replace target file failed: {error}"))?;
    }
    Ok(())
}

fn same_existing_path(left: &Path, right: &Path) -> bool {
    let Ok(left) = left.canonicalize() else {
        return false;
    };
    let Ok(right) = right.canonicalize() else {
        return false;
    };
    left == right
}

fn estimate_transfer_bytes(entries: &[FileTransferEntry]) -> Result<u64, String> {
    let mut total = 0u64;
    for entry in entries {
        let path = PathBuf::from(&entry.source_path)
            .canonicalize()
            .map_err(|error| format!("resolve transfer source failed: {error}"))?;
        total = total.saturating_add(estimate_path_bytes(&path)?);
    }
    Ok(total)
}

fn estimate_path_bytes(path: &Path) -> Result<u64, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("read transfer source metadata failed: {error}"))?;
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Ok(0);
    }
    let mut total = 0u64;
    for entry in fs::read_dir(path).map_err(|error| format!("read directory failed: {error}"))? {
        let entry = entry.map_err(|error| format!("read directory entry failed: {error}"))?;
        total = total.saturating_add(estimate_path_bytes(&entry.path())?);
    }
    Ok(total)
}

fn read_directory(path: Option<String>) -> Result<DirectoryList, String> {
    let requested = path
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_file_browser_root);
    let directory = requested
        .canonicalize()
        .map_err(|error| format!("resolve directory failed: {error}"))?;
    if !directory.is_dir() {
        return Err(format!("not a directory: {}", directory.display()));
    }

    let mut entries = Vec::new();
    for entry in
        std::fs::read_dir(&directory).map_err(|error| format!("read directory failed: {error}"))?
    {
        let entry = entry.map_err(|error| format!("read directory entry failed: {error}"))?;
        let metadata = entry
            .metadata()
            .map_err(|error| format!("read entry metadata failed: {error}"))?;
        entries.push(file_entry_from_metadata(entry.path(), metadata)?);
    }
    entries.sort_by(|left, right| {
        file_entry_sort_rank(&left.kind)
            .cmp(&file_entry_sort_rank(&right.kind))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.name.cmp(&right.name))
    });

    Ok(DirectoryList {
        path: directory.display().to_string(),
        parent_path: directory
            .parent()
            .map(|parent| parent.display().to_string()),
        entries,
    })
}

fn file_entry_from_metadata(
    path: PathBuf,
    metadata: std::fs::Metadata,
) -> Result<FileEntry, String> {
    let file_type = metadata.file_type();
    let kind = if file_type.is_dir() {
        FileEntryKind::Directory
    } else if file_type.is_file() {
        FileEntryKind::File
    } else if file_type.is_symlink() {
        FileEntryKind::Symlink
    } else {
        FileEntryKind::Other
    };
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("invalid file name: {}", path.display()))?
        .to_string();
    let modified_ms = metadata.modified().ok().and_then(|modified| {
        modified
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| u64::try_from(duration.as_millis()).ok())
    });
    Ok(FileEntry {
        name,
        path: path.display().to_string(),
        kind,
        size_bytes: metadata.is_file().then_some(metadata.len()),
        modified_ms,
        readonly: metadata.permissions().readonly(),
    })
}

fn file_entry_sort_rank(kind: &FileEntryKind) -> u8 {
    match kind {
        FileEntryKind::Directory => 0,
        FileEntryKind::Symlink => 1,
        FileEntryKind::File => 2,
        FileEntryKind::Other => 3,
    }
}

fn default_file_browser_root() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| Path::new(".").to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use mrd_ipc::{
        FileTransferConflictPolicy, FileTransferEntry, FileTransferStartRequest, FileTransferStatus,
    };
    use mrd_proto::DeviceId;
    use std::sync::Arc;

    #[test]
    fn list_directory_returns_sorted_entries_from_service_host() {
        let root =
            std::env::temp_dir().join(format!("mrd-service-list-directory-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("folder")).expect("create folder");
        std::fs::write(root.join("file.txt"), b"hello").expect("write file");

        let response = list_directory(Some(root.display().to_string()));
        let IpcResponse::DirectoryList { listing } = response else {
            panic!("expected directory list response");
        };

        assert_eq!(
            listing.path,
            root.canonicalize().unwrap().display().to_string()
        );
        assert_eq!(listing.entries[0].name, "folder");
        assert_eq!(listing.entries[0].kind, FileEntryKind::Directory);
        assert_eq!(listing.entries[1].name, "file.txt");
        assert_eq!(listing.entries[1].kind, FileEntryKind::File);
        assert_eq!(listing.entries[1].size_bytes, Some(5));

        std::fs::remove_dir_all(root).expect("remove temp directory");
    }

    #[test]
    fn list_directory_rejects_files() {
        let path = std::env::temp_dir().join(format!(
            "mrd-service-list-directory-file-{}.txt",
            std::process::id()
        ));
        std::fs::write(&path, b"hello").expect("write file");

        let response = list_directory(Some(path.display().to_string()));

        match response {
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "E_LIST_DIRECTORY");
                assert!(message.contains("not a directory"));
            }
            other => panic!("expected error response, got {other:?}"),
        }

        std::fs::remove_file(path).expect("remove temp file");
    }

    #[tokio::test]
    async fn start_file_transfer_copies_file_and_records_completed_task() {
        let app_state = Arc::new(AppState::new());
        let root =
            std::env::temp_dir().join(format!("mrd-service-file-transfer-{}", std::process::id()));
        let source_dir = root.join("source");
        let target_dir = root.join("target");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&source_dir).expect("create source");
        std::fs::create_dir_all(&target_dir).expect("create target");
        let source_file = source_dir.join("payload.txt");
        std::fs::write(&source_file, b"hello").expect("write source");

        let response = start_file_transfer(
            &app_state,
            FileTransferStartRequest {
                source_device_id: Some(DeviceId("local-source".to_string())),
                target_device_id: Some(DeviceId("local-target".to_string())),
                entries: vec![FileTransferEntry {
                    source_path: source_file.display().to_string(),
                    file_name: Some("payload.txt".to_string()),
                    kind: FileEntryKind::File,
                }],
                target_path: target_dir.display().to_string(),
                conflict_policy: FileTransferConflictPolicy::Rename,
                transport_hint: Some("local".to_string()),
                provider_hint: None,
            },
        )
        .await;

        let IpcResponse::FileTransferStarted { transfer } = response else {
            panic!("expected file transfer response");
        };
        assert_eq!(transfer.status, FileTransferStatus::Completed);
        assert_eq!(transfer.transport_kind, "local");
        assert_eq!(transfer.provider_kind, "mrd-local");
        assert_eq!(
            transfer.provider_capabilities,
            vec!["service.file_transfer.local".to_string()]
        );
        assert_eq!(transfer.total_entries, 1);
        assert_eq!(transfer.copied_entries, 1);
        assert_eq!(transfer.total_bytes, Some(5));
        assert_eq!(transfer.copied_bytes, 5);
        assert_eq!(transfer.entries[0].name, "payload.txt");
        assert_eq!(
            std::fs::read(target_dir.join("payload.txt")).unwrap(),
            b"hello"
        );

        let list_response = list_file_transfers(&app_state).await;
        let IpcResponse::FileTransferList { transfers } = list_response else {
            panic!("expected file transfer list");
        };
        assert_eq!(transfers.len(), 1);
        assert_eq!(transfers[0].transfer_id, transfer.transfer_id);

        std::fs::remove_dir_all(root).expect("remove temp directory");
    }

    #[tokio::test]
    async fn start_file_transfer_renames_conflicting_targets() {
        let app_state = Arc::new(AppState::new());
        let root = std::env::temp_dir().join(format!(
            "mrd-service-file-transfer-rename-{}",
            std::process::id()
        ));
        let source_dir = root.join("source");
        let target_dir = root.join("target");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&source_dir).expect("create source");
        std::fs::create_dir_all(&target_dir).expect("create target");
        let source_file = source_dir.join("payload.txt");
        std::fs::write(&source_file, b"new").expect("write source");
        std::fs::write(target_dir.join("payload.txt"), b"existing").expect("write existing");

        let response = start_file_transfer(
            &app_state,
            FileTransferStartRequest {
                source_device_id: None,
                target_device_id: None,
                entries: vec![FileTransferEntry {
                    source_path: source_file.display().to_string(),
                    file_name: Some("payload.txt".to_string()),
                    kind: FileEntryKind::File,
                }],
                target_path: target_dir.display().to_string(),
                conflict_policy: FileTransferConflictPolicy::Rename,
                transport_hint: None,
                provider_hint: None,
            },
        )
        .await;

        let IpcResponse::FileTransferStarted { transfer } = response else {
            panic!("expected file transfer response");
        };
        assert_eq!(transfer.status, FileTransferStatus::Completed);
        assert_eq!(
            std::fs::read(target_dir.join("payload.txt")).unwrap(),
            b"existing"
        );
        assert_eq!(
            std::fs::read(target_dir.join("payload (1).txt")).unwrap(),
            b"new"
        );

        std::fs::remove_dir_all(root).expect("remove temp directory");
    }

    #[tokio::test]
    async fn start_file_transfer_rejects_reserved_external_provider_hint() {
        let app_state = Arc::new(AppState::new());
        let root = std::env::temp_dir().join(format!(
            "mrd-service-file-transfer-provider-hint-{}",
            std::process::id()
        ));
        let source_dir = root.join("source");
        let target_dir = root.join("target");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&source_dir).expect("create source");
        std::fs::create_dir_all(&target_dir).expect("create target");
        let source_file = source_dir.join("payload.txt");
        std::fs::write(&source_file, b"new").expect("write source");

        let response = start_file_transfer(
            &app_state,
            FileTransferStartRequest {
                source_device_id: None,
                target_device_id: None,
                entries: vec![FileTransferEntry {
                    source_path: source_file.display().to_string(),
                    file_name: Some("payload.txt".to_string()),
                    kind: FileEntryKind::File,
                }],
                target_path: target_dir.display().to_string(),
                conflict_policy: FileTransferConflictPolicy::Rename,
                transport_hint: None,
                provider_hint: Some("r-file".to_string()),
            },
        )
        .await;

        match response {
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "E_FILE_TRANSFER_PROVIDER_UNAVAILABLE");
                assert!(message.contains("r-file"));
                assert!(message.contains("service.file_transfer.external_bridge"));
            }
            other => panic!("expected provider unavailable error, got {other:?}"),
        }
        assert!(!target_dir.join("payload.txt").exists());

        let IpcResponse::FileTransferList { transfers } = list_file_transfers(&app_state).await
        else {
            panic!("expected file transfer list");
        };
        assert!(transfers.is_empty());

        std::fs::remove_dir_all(root).expect("remove temp directory");
    }

    #[test]
    fn list_file_transfer_providers_exposes_local_and_reserved_external_bridge() {
        let response = list_file_transfer_providers();

        let IpcResponse::FileTransferProviderList { providers } = response else {
            panic!("expected file transfer provider list");
        };
        assert_eq!(providers.len(), 2);
        assert_eq!(providers[0].provider_kind, "mrd-local");
        assert_eq!(providers[0].status, mrd_ipc::CapabilityStatus::Available);
        assert_eq!(
            providers[0].capabilities,
            vec!["service.file_transfer.local".to_string()]
        );
        assert_eq!(providers[1].provider_kind, "r-file");
        assert_eq!(
            providers[1].status,
            mrd_ipc::CapabilityStatus::Unimplemented
        );
        assert_eq!(
            providers[1].capabilities,
            vec!["service.file_transfer.external_bridge".to_string()]
        );
        assert!(providers[1]
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("reserved"));
        let handoff = providers[1]
            .handoff_hint
            .as_ref()
            .expect("reserved R-File handoff hint");
        assert_eq!(handoff.external_app, "R-File");
        assert_eq!(handoff.bridge_service, "rfile-bridge");
        assert_eq!(
            handoff.control_endpoint.as_deref(),
            Some("http://127.0.0.1:18100")
        );
        assert!(handoff
            .capabilities
            .contains(&"rfile.remote_mount.v1".to_string()));
    }
}
