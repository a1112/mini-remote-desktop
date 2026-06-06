use mrd_ipc::{DirectoryList, FileEntry, FileEntryKind, IpcResponse};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub fn list_directory(path: Option<String>) -> IpcResponse {
    match read_directory(path) {
        Ok(listing) => IpcResponse::DirectoryList { listing },
        Err(error) => IpcResponse::Error {
            code: "E_LIST_DIRECTORY".to_string(),
            message: error,
        },
    }
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
}
