use mrd_ipc::{
    FileTransferActionKind, FileTransferActionResult, FileTransferProviderSnapshot,
    FileTransferSnapshot, IpcResponse,
};
use std::path::{Path, PathBuf};

const RFILE_DEFAULT_HTTP_HOST: &str = "127.0.0.1";
const RFILE_DEFAULT_HTTP_PORT: u16 = 18080;
const RFILE_DEFAULT_QUIC_HOST: &str = "127.0.0.1";
const RFILE_DEFAULT_QUIC_PORT: u16 = 18081;

pub fn snapshot() -> FileTransferSnapshot {
    match detect_rfile_root() {
        Some(root) => snapshot_for_rfile_root(&root),
        None => reserved_snapshot(),
    }
}

pub fn request_action(transfer_id: String, action: FileTransferActionKind) -> IpcResponse {
    let snapshot = snapshot();
    let known_task = snapshot
        .tasks
        .iter()
        .any(|task| task.transfer_id == transfer_id);
    let (accepted, supported, message) = if known_task {
        (
            false,
            false,
            format!(
                "File transfer provider {} does not expose runtime task actions yet.",
                snapshot.provider.provider_id
            ),
        )
    } else {
        (
            false,
            false,
            format!(
                "File transfer provider {} has no active task named {}.",
                snapshot.provider.provider_id, transfer_id
            ),
        )
    };

    IpcResponse::FileTransferActionRequested {
        result: FileTransferActionResult {
            transfer_id,
            action,
            accepted,
            supported,
            message,
        },
    }
}

fn reserved_snapshot() -> FileTransferSnapshot {
    FileTransferSnapshot {
        provider: FileTransferProviderSnapshot {
            provider_id: "mrd.file_transfer.reserved".to_string(),
            display_name: "Reserved file transfer provider".to_string(),
            status: "reserved".to_string(),
            detail: Some(
                "Reserved for MRD-native or R-File provider binding; set MRD_RFILE_ROOT to expose a local R-File provider boundary."
                    .to_string(),
            ),
            capabilities: vec![
                "file.transfer.snapshot".to_string(),
                "file.transfer.external_provider".to_string(),
                "file.transfer.rfile.quic_stream".to_string(),
                "file.transfer.rfile.http_client_stats".to_string(),
                "file.transfer.rfile.remote_mount".to_string(),
                "file.transfer.perf_baseline".to_string(),
            ],
            supported_actions: vec![
                "list".to_string(),
                "compare_provider".to_string(),
                "bind_external_provider".to_string(),
            ],
        },
        tasks: Vec::new(),
        updated_at_ms: None,
    }
}

fn snapshot_for_rfile_root(root: &Path) -> FileTransferSnapshot {
    let mut capabilities = vec![
        "file.transfer.snapshot".to_string(),
        "file.transfer.external_provider".to_string(),
        "file.transfer.rfile.integration_boundary".to_string(),
    ];

    if root.join("services/rfile-watch/Cargo.toml").is_file() {
        capabilities.extend([
            "file.transfer.rfile.watch_service".to_string(),
            "file.transfer.rfile.http_fs".to_string(),
            "file.transfer.rfile.http.download_stream_1mb_buffer".to_string(),
            "file.transfer.rfile.http.upload_stream_16gb_limit".to_string(),
            "file.transfer.rfile.quic_stream".to_string(),
            "file.transfer.rfile.quic.transfer_16gb_limit".to_string(),
            "file.transfer.rfile.transfer_tasks".to_string(),
        ]);
    }

    if root.join("crates/rfile-remote-client/Cargo.toml").is_file() {
        capabilities.extend([
            "file.transfer.rfile.remote_client".to_string(),
            "file.transfer.rfile.http_client_stats".to_string(),
            "file.transfer.rfile.remote_browse".to_string(),
        ]);
    }

    if root.join("crates/rfile-mount-core/Cargo.toml").is_file() {
        capabilities.extend([
            "file.transfer.rfile.mount_core".to_string(),
            "file.transfer.rfile.remote_mount".to_string(),
            "file.transfer.rfile.staged_write".to_string(),
        ]);
    }

    capabilities.extend(rfile_runtime_endpoint_capabilities());
    capabilities.sort();
    capabilities.dedup();

    FileTransferSnapshot {
        provider: FileTransferProviderSnapshot {
            provider_id: "mrd.file_transfer.rfile".to_string(),
            display_name: "R-File provider boundary".to_string(),
            status: "available".to_string(),
            detail: Some(format!(
                "Detected R-File provider root at {}; service can map browse, transfer, perf and mount capabilities before binding runtime actions.",
                root.display()
            )),
            capabilities,
            supported_actions: vec![
                "list".to_string(),
                "compare_provider".to_string(),
                "bind_external_provider".to_string(),
                "browse_remote".to_string(),
                "send".to_string(),
                "receive".to_string(),
                "mount_remote".to_string(),
            ],
        },
        tasks: Vec::new(),
        updated_at_ms: None,
    }
}

fn detect_rfile_root() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("MRD_RFILE_ROOT").map(PathBuf::from) {
        return path.is_dir().then_some(path);
    }

    let cwd = std::env::current_dir().ok()?;
    let sibling = cwd.parent()?.join("R-File");
    sibling.is_dir().then_some(sibling)
}

fn rfile_runtime_endpoint_capabilities() -> Vec<String> {
    let http_host =
        env_string("RFILE_SERVICE_HOST").unwrap_or_else(|| RFILE_DEFAULT_HTTP_HOST.to_string());
    let http_port = env_u16("RFILE_SERVICE_PORT").unwrap_or(RFILE_DEFAULT_HTTP_PORT);
    let quic_host = normalize_endpoint_host(
        &env_string("RFILE_QUIC_HOST").unwrap_or_else(|| RFILE_DEFAULT_QUIC_HOST.to_string()),
    );
    let quic_port = env_u16("RFILE_QUIC_PORT").unwrap_or(RFILE_DEFAULT_QUIC_PORT);

    vec![
        format!("file.transfer.rfile.endpoint.http://{http_host}:{http_port}"),
        format!("file.transfer.rfile.quic_endpoint.{quic_host}:{quic_port}"),
    ]
}

fn env_string(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_u16(name: &str) -> Option<u16> {
    env_string(name).and_then(|value| value.parse::<u16>().ok())
}

fn normalize_endpoint_host(host: &str) -> String {
    match host.trim() {
        "0.0.0.0" | "::" | "" => "127.0.0.1".to_string(),
        value => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfile_provider_snapshot_maps_available_capabilities() {
        let rfile_root = std::env::temp_dir().join(format!(
            "mrd-rfile-provider-direct-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&rfile_root);
        std::fs::create_dir_all(rfile_root.join("services/rfile-watch")).unwrap();
        std::fs::create_dir_all(rfile_root.join("crates/rfile-remote-client")).unwrap();
        std::fs::create_dir_all(rfile_root.join("crates/rfile-mount-core")).unwrap();
        std::fs::write(rfile_root.join("services/rfile-watch/Cargo.toml"), "").unwrap();
        std::fs::write(rfile_root.join("crates/rfile-remote-client/Cargo.toml"), "").unwrap();
        std::fs::write(rfile_root.join("crates/rfile-mount-core/Cargo.toml"), "").unwrap();

        let snapshot = snapshot_for_rfile_root(&rfile_root);

        let _ = std::fs::remove_dir_all(&rfile_root);

        assert_eq!(snapshot.provider.status, "available");
        assert!(snapshot
            .provider
            .capabilities
            .contains(&"file.transfer.rfile.watch_service".to_string()));
        assert!(snapshot
            .provider
            .capabilities
            .contains(&"file.transfer.rfile.remote_client".to_string()));
        assert!(snapshot
            .provider
            .capabilities
            .contains(&"file.transfer.rfile.mount_core".to_string()));
        assert!(snapshot
            .provider
            .supported_actions
            .contains(&"bind_external_provider".to_string()));
        assert!(snapshot
            .provider
            .capabilities
            .contains(&"file.transfer.rfile.http.download_stream_1mb_buffer".to_string()));
        assert!(snapshot
            .provider
            .capabilities
            .contains(&"file.transfer.rfile.http.upload_stream_16gb_limit".to_string()));
        assert!(snapshot
            .provider
            .capabilities
            .contains(&"file.transfer.rfile.quic.transfer_16gb_limit".to_string()));
        assert!(snapshot
            .provider
            .capabilities
            .contains(&"file.transfer.rfile.endpoint.http://127.0.0.1:18080".to_string()));
        assert!(snapshot
            .provider
            .capabilities
            .contains(&"file.transfer.rfile.quic_endpoint.127.0.0.1:18081".to_string()));
    }
}
