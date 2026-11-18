use crate::app_state::{AppState, FileTransferCancellation, FileTransferRegistry};
use mrd_ipc::{
    CapabilityStatus, DirectoryList, FileEntry, FileEntryKind, FileTransferConflictPolicy,
    FileTransferEntry, FileTransferProviderDescriptor, FileTransferProviderHandoffHint,
    FileTransferStartRequest, FileTransferStatus, FileTransferTaskSnapshot, IpcResponse,
};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::UNIX_EPOCH;
use tokio::sync::{watch, Mutex};

const LOCAL_FILE_TRANSFER_PROVIDER: &str = "mrd-local";
const LOCAL_FILE_TRANSFER_CAPABILITY: &str = "service.file_transfer.local";
const RFILE_FILE_TRANSFER_PROVIDER: &str = "r-file";
const EXTERNAL_FILE_TRANSFER_BRIDGE_CAPABILITY: &str = "service.file_transfer.external_bridge";
const EXTERNAL_FILE_TRANSFER_BRIDGE_REASON: &str =
    "reserved handoff to R-File; MRD keeps local copy/list/cancel as the active path";
const RFILE_BRIDGE_CONTROL_ENDPOINT: &str = "http://127.0.0.1:18100";
const RFILE_WATCH_DATA_ENDPOINT: &str = "http://127.0.0.1:18080";
const COPY_CHUNK_BYTES: usize = 256 * 1024;

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
    let transfer = FileTransferTaskSnapshot {
        transfer_id,
        status: FileTransferStatus::Queued,
        source_device_id: request.source_device_id.clone(),
        target_device_id: request.target_device_id.clone(),
        transport_kind,
        provider_kind: LOCAL_FILE_TRANSFER_PROVIDER.to_string(),
        provider_capabilities: vec![LOCAL_FILE_TRANSFER_CAPABILITY.to_string()],
        total_entries: request.entries.len(),
        copied_entries: 0,
        total_bytes: None,
        copied_bytes: 0,
        error: None,
        entries: Vec::new(),
    };

    let registry = app_state.file_transfers();
    let cancelled = Arc::new(AtomicBool::new(false));
    let (finished, completion) = watch::channel(None);
    registry
        .lock()
        .await
        .begin(transfer.clone(), cancelled.clone(), completion.clone());

    // The service owns the worker and its terminal update even if the IPC caller disconnects.
    tokio::spawn(async move {
        let worker_registry = registry.clone();
        let mut fallback = transfer.clone();
        let transfer = tokio::task::spawn_blocking(move || {
            run_file_transfer(request, &worker_registry, &cancelled, transfer)
        })
        .await
        .unwrap_or_else(|error| {
            fallback.status = FileTransferStatus::Failed;
            fallback.error = Some(format!("file transfer task failed: {error}"));
            fallback
        });

        registry.lock().await.finish(transfer.clone());
        finished.send_replace(Some(transfer));
    });

    transfer_response(completion, |transfer| IpcResponse::FileTransferStarted {
        transfer,
    })
    .await
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
    let cancellation = app_state.file_transfers.lock().await.cancel(&transfer_id);
    match cancellation {
        Some(FileTransferCancellation::Pending(completion)) => {
            transfer_response(completion, |transfer| IpcResponse::FileTransferCancelled {
                transfer,
            })
            .await
        }
        Some(FileTransferCancellation::Finished(transfer)) => {
            IpcResponse::FileTransferCancelled { transfer }
        }
        None => IpcResponse::Error {
            code: "E_FILE_TRANSFER_NOT_FOUND".to_string(),
            message: format!("file transfer not found: {transfer_id}"),
        },
    }
}

async fn transfer_response(
    mut completion: watch::Receiver<Option<FileTransferTaskSnapshot>>,
    response: impl FnOnce(FileTransferTaskSnapshot) -> IpcResponse,
) -> IpcResponse {
    match completion.wait_for(Option::is_some).await {
        Ok(transfer) => response(transfer.clone().expect("transfer finished")),
        Err(_) => IpcResponse::Error {
            code: "E_FILE_TRANSFER_WORKER".to_string(),
            message: "file transfer worker stopped without a terminal result".to_string(),
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

#[derive(Debug)]
enum CopyError {
    Cancelled,
    Failed(String),
}

impl From<String> for CopyError {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

fn check_transfer_cancelled(cancelled: &AtomicBool) -> Result<(), CopyError> {
    if cancelled.load(Ordering::Acquire) {
        Err(CopyError::Cancelled)
    } else {
        Ok(())
    }
}

fn run_file_transfer(
    request: FileTransferStartRequest,
    registry: &Mutex<FileTransferRegistry>,
    cancelled: &AtomicBool,
    mut transfer: FileTransferTaskSnapshot,
) -> FileTransferTaskSnapshot {
    transfer.status = FileTransferStatus::Running;
    registry.blocking_lock().upsert(transfer.clone());
    transfer.status = match execute_file_transfer(request, registry, cancelled, &mut transfer) {
        Ok(()) => FileTransferStatus::Completed,
        Err(CopyError::Cancelled) => FileTransferStatus::Cancelled,
        Err(CopyError::Failed(error)) => {
            transfer.error = Some(error);
            FileTransferStatus::Failed
        }
    };
    transfer
}

fn execute_file_transfer(
    request: FileTransferStartRequest,
    registry: &Mutex<FileTransferRegistry>,
    cancelled: &AtomicBool,
    transfer: &mut FileTransferTaskSnapshot,
) -> Result<(), CopyError> {
    check_transfer_cancelled(cancelled)?;
    let target_dir = PathBuf::from(&request.target_path)
        .canonicalize()
        .map_err(|error| format!("resolve transfer target failed: {error}"))?;
    if !target_dir.is_dir() {
        return Err(format!(
            "transfer target is not a directory: {}",
            target_dir.display()
        )
        .into());
    }
    transfer.total_bytes = estimate_transfer_bytes(&request.entries, cancelled).ok();
    check_transfer_cancelled(cancelled)?;
    registry.blocking_lock().upsert(transfer.clone());

    for entry in request.entries {
        let (copied, bytes) = copy_transfer_entry(
            &entry,
            &target_dir,
            request.conflict_policy,
            cancelled,
            registry,
        )?;
        transfer.copied_bytes = transfer.copied_bytes.saturating_add(bytes);
        transfer.copied_entries += 1;
        transfer.entries.push(copied);
    }
    Ok(())
}

fn copy_transfer_entry(
    entry: &FileTransferEntry,
    target_dir: &Path,
    conflict_policy: FileTransferConflictPolicy,
    cancelled: &AtomicBool,
    registry: &Mutex<FileTransferRegistry>,
) -> Result<(FileEntry, u64), CopyError> {
    check_transfer_cancelled(cancelled)?;
    let source = PathBuf::from(&entry.source_path)
        .canonicalize()
        .map_err(|error| format!("resolve transfer source failed: {error}"))?;
    let metadata = fs::metadata(&source)
        .map_err(|error| format!("read transfer source metadata failed: {error}"))?;
    let file_name = transfer_file_name(entry, &source)?;
    let output_path = resolve_transfer_output_path(target_dir, &file_name, conflict_policy)?;
    if same_existing_path(&source, &output_path)
        || (metadata.is_dir() && target_dir.starts_with(&source))
    {
        return Err(format!(
            "cannot copy the transfer source into itself: {}",
            source.display()
        )
        .into());
    }
    let mut builder = tempfile::Builder::new();
    builder.prefix(".mrd-transfer-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o700));
    }
    let staging = builder
        .tempdir_in(target_dir)
        .map_err(|error| format!("create transfer staging directory failed: {error}"))?;
    let staged_path = staging.path().join("payload");
    #[cfg(windows)]
    let _staged_permissions = StagedPayloadPermissions(&staged_path);

    let copied_bytes = match entry.kind {
        FileEntryKind::File if metadata.is_file() => {
            copy_file_entry(&source, &staged_path, cancelled)?
        }
        FileEntryKind::Directory if metadata.is_dir() => {
            copy_directory_recursive(&source, &staged_path, cancelled)?
        }
        _ => {
            return Err(format!(
                "transfer entry kind does not match a supported source: {}",
                source.display()
            )
            .into())
        }
    };
    let copied_metadata = fs::metadata(&staged_path)
        .map_err(|error| format!("read copied entry metadata failed: {error}"))?;

    // Serialize publication with cancellation and with other transfers choosing the same destination.
    let commit = registry.blocking_lock();
    check_transfer_cancelled(cancelled)?;
    let output_path = resolve_transfer_output_path(target_dir, &file_name, conflict_policy)?;
    let copied_entry = file_entry_from_metadata(output_path.clone(), copied_metadata)?;
    let previous = staging.path().join("previous");
    let replace = conflict_policy == FileTransferConflictPolicy::Replace
        && transfer_path_exists(&output_path)?;
    if replace {
        fs::rename(&output_path, &previous)
            .map_err(|error| format!("stage existing target failed: {error}"))?;
    }
    if let Err(error) = fs::rename(&staged_path, &output_path) {
        let restore = if replace {
            fs::rename(&previous, &output_path)
        } else {
            Ok(())
        };
        if let Err(rollback) = restore {
            let kept = staging.keep();
            return Err(format!(
                "publish transfer failed: {error}; restore failed: {rollback}; previous target kept at {}",
                kept.join("previous").display()
            ).into());
        }
        return Err(format!("publish transfer failed: {error}").into());
    }
    drop(commit);
    if let Err(error) = staging.close() {
        tracing::warn!(%error, "could not remove file transfer staging directory");
    }
    Ok((copied_entry, copied_bytes))
}

// Windows can refuse to delete read-only staged files. Only adjust the new payload.
#[cfg(windows)]
struct StagedPayloadPermissions<'a>(&'a Path);

#[cfg(windows)]
impl Drop for StagedPayloadPermissions<'_> {
    fn drop(&mut self) {
        fn make_removable(path: &Path) {
            let Ok(metadata) = path.symlink_metadata() else {
                return;
            };
            if metadata.is_dir() {
                if let Ok(entries) = fs::read_dir(path) {
                    for entry in entries.flatten() {
                        make_removable(&entry.path());
                    }
                }
            } else if metadata.is_file() && metadata.permissions().readonly() {
                let mut permissions = metadata.permissions();
                permissions.set_readonly(false);
                let _ = fs::set_permissions(path, permissions);
            }
        }
        make_removable(self.0);
    }
}

fn copy_file_entry(
    source: &Path,
    output_path: &Path,
    cancelled: &AtomicBool,
) -> Result<u64, CopyError> {
    check_transfer_cancelled(cancelled)?;
    let mut input =
        fs::File::open(source).map_err(|error| format!("open source file failed: {error}"))?;
    let permissions = input
        .metadata()
        .map_err(|error| format!("read source permissions failed: {error}"))?
        .permissions();
    let mut output = fs::File::create(output_path)
        .map_err(|error| format!("create staged file failed: {error}"))?;
    let copied = copy_file_chunks(&mut input, &mut output, cancelled)?;
    output
        .set_permissions(permissions)
        .map_err(|error| format!("preserve file permissions failed: {error}"))?;
    Ok(copied)
}

fn copy_file_chunks(
    input: &mut impl Read,
    output: &mut impl Write,
    cancelled: &AtomicBool,
) -> Result<u64, CopyError> {
    let mut buffer = vec![0; COPY_CHUNK_BYTES];
    let mut copied = 0u64;
    loop {
        check_transfer_cancelled(cancelled)?;
        let read = match input.read(&mut buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result.map_err(|error| format!("read transfer file failed: {error}"))?,
        };
        if read == 0 {
            return Ok(copied);
        }
        check_transfer_cancelled(cancelled)?;
        output
            .write_all(&buffer[..read])
            .map_err(|error| format!("write staged file failed: {error}"))?;
        copied += read as u64;
    }
}

fn copy_directory_recursive(
    source: &Path,
    output_path: &Path,
    cancelled: &AtomicBool,
) -> Result<u64, CopyError> {
    check_transfer_cancelled(cancelled)?;
    fs::create_dir(output_path)
        .map_err(|error| format!("create staged directory failed: {error}"))?;
    let mut copied = 0u64;
    for entry in fs::read_dir(source).map_err(|error| format!("read directory failed: {error}"))? {
        check_transfer_cancelled(cancelled)?;
        let entry = entry.map_err(|error| format!("read directory entry failed: {error}"))?;
        let child_source = entry.path();
        let child_output = output_path.join(entry.file_name());
        let metadata = entry
            .metadata()
            .map_err(|error| format!("read directory entry metadata failed: {error}"))?;
        if metadata.is_dir() {
            copied = copied.saturating_add(copy_directory_recursive(
                &child_source,
                &child_output,
                cancelled,
            )?);
        } else if metadata.is_file() {
            copied =
                copied.saturating_add(copy_file_entry(&child_source, &child_output, cancelled)?);
        } else {
            return Err(format!(
                "unsupported directory transfer entry: {}",
                child_source.display()
            )
            .into());
        }
    }
    Ok(copied)
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
        || (cfg!(windows) && file_name.contains(':'))
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
        FileTransferConflictPolicy::Reject if transfer_path_exists(&preferred)? => Err(format!(
            "transfer target already exists: {}",
            preferred.display()
        )),
        FileTransferConflictPolicy::Reject | FileTransferConflictPolicy::Replace => Ok(preferred),
        FileTransferConflictPolicy::Rename => unique_transfer_output_path(&preferred),
    }
}

fn unique_transfer_output_path(preferred: &Path) -> Result<PathBuf, String> {
    if !transfer_path_exists(preferred)? {
        return Ok(preferred.to_path_buf());
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
        if !transfer_path_exists(&candidate)? {
            return Ok(candidate);
        }
    }
    Err(format!(
        "no available transfer name for {}",
        preferred.display()
    ))
}

fn transfer_path_exists(path: &Path) -> Result<bool, String> {
    match path.symlink_metadata() {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("read transfer target metadata failed: {error}")),
    }
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

fn estimate_transfer_bytes(
    entries: &[FileTransferEntry],
    cancelled: &AtomicBool,
) -> Result<u64, CopyError> {
    let mut total = 0u64;
    for entry in entries {
        check_transfer_cancelled(cancelled)?;
        let path = PathBuf::from(&entry.source_path)
            .canonicalize()
            .map_err(|error| format!("resolve transfer source failed: {error}"))?;
        total = total.saturating_add(estimate_path_bytes(&path, cancelled)?);
    }
    Ok(total)
}

fn estimate_path_bytes(path: &Path, cancelled: &AtomicBool) -> Result<u64, CopyError> {
    check_transfer_cancelled(cancelled)?;
    let metadata = fs::symlink_metadata(path)
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
        total = total.saturating_add(estimate_path_bytes(&entry.path(), cancelled)?);
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
    use std::time::Duration;

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

    #[test]
    fn cancellation_stops_the_copy_before_the_next_chunk() {
        struct CancelAfterWrite<'a> {
            cancelled: &'a AtomicBool,
            written: usize,
        }

        impl Write for CancelAfterWrite<'_> {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.written += bytes.len();
                self.cancelled.store(true, Ordering::Release);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let cancelled = AtomicBool::new(false);
        let bytes = vec![7; 2 * COPY_CHUNK_BYTES];
        let mut output = CancelAfterWrite {
            cancelled: &cancelled,
            written: 0,
        };
        assert!(matches!(
            copy_file_chunks(&mut bytes.as_slice(), &mut output, &cancelled),
            Err(CopyError::Cancelled)
        ));
        assert_eq!(output.written, COPY_CHUNK_BYTES);
    }

    #[tokio::test]
    async fn cancellation_before_publish_preserves_the_existing_directory() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let target = root.path().join("target");
        let destination = target.join("data");
        fs::create_dir(&source).unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(source.join("new.txt"), "new").unwrap();
        fs::write(destination.join("old.txt"), "original").unwrap();

        let cancelled = Arc::new(AtomicBool::new(false));
        let registry = Arc::new(Mutex::new(FileTransferRegistry::default()));
        let block_publication = registry.lock().await;
        let worker = tokio::task::spawn_blocking({
            let cancelled = cancelled.clone();
            let registry = registry.clone();
            let target = target.clone();
            move || {
                copy_transfer_entry(
                    &FileTransferEntry {
                        source_path: source.display().to_string(),
                        file_name: Some("data".to_string()),
                        kind: FileEntryKind::Directory,
                    },
                    &target,
                    FileTransferConflictPolicy::Replace,
                    &cancelled,
                    &registry,
                )
            }
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while !fs::read_dir(&target).unwrap().flatten().any(|entry| {
                fs::read(entry.path().join("payload/new.txt")).is_ok_and(|bytes| bytes == b"new")
            }) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("wait for staged copy");

        cancelled.store(true, Ordering::Release);
        drop(block_publication);
        assert!(matches!(worker.await.unwrap(), Err(CopyError::Cancelled)));
        assert_eq!(fs::read(destination.join("old.txt")).unwrap(), b"original");
        assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
    }

    #[test]
    fn cancel_waits_for_the_service_owned_worker_after_start_caller_leaves() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()
            .unwrap();
        runtime.block_on(async {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("source.txt");
            let target = root.path().join("target");
            fs::write(&source, b"source").unwrap();
            fs::create_dir(&target).unwrap();
            let app_state = Arc::new(AppState::new());
            let (release, blocked) = std::sync::mpsc::channel::<()>();
            let _blocker = tokio::task::spawn_blocking(move || blocked.recv());
            let started = tokio::spawn({
                let state = app_state.clone();
                let target = target.clone();
                async move {
                    start_file_transfer(
                        &state,
                        FileTransferStartRequest {
                            source_device_id: None,
                            target_device_id: None,
                            entries: vec![FileTransferEntry {
                                source_path: source.display().to_string(),
                                file_name: None,
                                kind: FileEntryKind::File,
                            }],
                            target_path: target.display().to_string(),
                            conflict_policy: FileTransferConflictPolicy::Reject,
                            transport_hint: None,
                            provider_hint: None,
                        },
                    )
                    .await
                }
            });
            let id = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if let Some(snapshot) = app_state.file_transfers.lock().await.list().pop() {
                        assert_eq!(snapshot.status, FileTransferStatus::Queued);
                        break snapshot.transfer_id;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("wait for queued transfer");
            started.abort();
            assert!(started.await.unwrap_err().is_cancelled());

            let mut cancellation = std::pin::pin!(cancel_file_transfer(&app_state, id.clone()));
            assert!(futures_util::poll!(&mut cancellation).is_pending());
            assert_eq!(
                app_state.file_transfers.lock().await.list()[0].status,
                FileTransferStatus::Queued
            );
            release.send(()).unwrap();
            let response = tokio::time::timeout(Duration::from_secs(5), cancellation)
                .await
                .unwrap();
            let IpcResponse::FileTransferCancelled { transfer } = response else {
                panic!("expected confirmed cancellation, got {response:?}");
            };
            assert_eq!(transfer.status, FileTransferStatus::Cancelled);
            assert_eq!(transfer.copied_entries, 0);
            assert_eq!(app_state.file_transfers.lock().await.list(), vec![transfer]);
            assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
        });
    }

    #[cfg(windows)]
    #[test]
    fn transfer_name_cannot_select_a_different_drive_or_an_alternate_stream() {
        assert!(validate_transfer_file_name("C:outside.txt").is_err());
        assert!(validate_transfer_file_name("target.txt:stream").is_err());
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
