use mrd_ipc::{FileTransferStatus, FileTransferTaskSnapshot};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::watch;

type TransferCompletion = watch::Receiver<Option<FileTransferTaskSnapshot>>;

#[derive(Debug)]
struct ActiveFileTransfer {
    cancelled: Arc<AtomicBool>,
    completion: TransferCompletion,
}

pub enum FileTransferCancellation {
    Pending(TransferCompletion),
    Finished(FileTransferTaskSnapshot),
}

/// Service-owned file transfer task snapshots.
#[derive(Debug, Default)]
pub struct FileTransferRegistry {
    next_transfer_seq: u64,
    transfers: Vec<FileTransferTaskSnapshot>,
    active: HashMap<String, ActiveFileTransfer>,
}

impl FileTransferRegistry {
    pub fn allocate_transfer_id(&mut self) -> String {
        self.next_transfer_seq = self.next_transfer_seq.saturating_add(1);
        format!("file-transfer-{}", self.next_transfer_seq)
    }

    pub fn upsert(&mut self, transfer: FileTransferTaskSnapshot) {
        if let Some(existing) = self
            .transfers
            .iter_mut()
            .find(|candidate| candidate.transfer_id == transfer.transfer_id)
        {
            if matches!(
                existing.status,
                FileTransferStatus::Queued | FileTransferStatus::Running
            ) {
                *existing = transfer;
            }
            return;
        }
        self.transfers.push(transfer);
    }

    pub fn begin(
        &mut self,
        transfer: FileTransferTaskSnapshot,
        cancelled: Arc<AtomicBool>,
        completion: TransferCompletion,
    ) {
        self.active.insert(
            transfer.transfer_id.clone(),
            ActiveFileTransfer {
                cancelled,
                completion,
            },
        );
        self.upsert(transfer);
    }

    pub fn finish(&mut self, transfer: FileTransferTaskSnapshot) {
        self.active.remove(&transfer.transfer_id);
        self.upsert(transfer);
    }

    pub fn list(&self) -> Vec<FileTransferTaskSnapshot> {
        self.transfers.clone()
    }

    pub fn cancel(&self, transfer_id: &str) -> Option<FileTransferCancellation> {
        if let Some(active) = self.active.get(transfer_id) {
            active.cancelled.store(true, Ordering::Release);
            return Some(FileTransferCancellation::Pending(active.completion.clone()));
        }
        let transfer = self
            .transfers
            .iter()
            .find(|candidate| candidate.transfer_id == transfer_id)?;
        Some(FileTransferCancellation::Finished(transfer.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transfer(transfer_id: String, status: FileTransferStatus) -> FileTransferTaskSnapshot {
        FileTransferTaskSnapshot {
            transfer_id,
            status,
            source_device_id: None,
            target_device_id: None,
            transport_kind: "local".to_string(),
            provider_kind: "mrd-local".to_string(),
            provider_capabilities: vec!["service.file_transfer.local".to_string()],
            total_entries: 0,
            copied_entries: 0,
            total_bytes: None,
            copied_bytes: 0,
            error: None,
            entries: Vec::new(),
        }
    }

    #[test]
    fn upsert_preserves_the_terminal_snapshot_after_late_progress() {
        let mut registry = FileTransferRegistry::default();
        let transfer_id = registry.allocate_transfer_id();
        registry.upsert(transfer(transfer_id.clone(), FileTransferStatus::Running));
        registry.upsert(transfer(transfer_id.clone(), FileTransferStatus::Completed));
        registry.upsert(transfer(transfer_id.clone(), FileTransferStatus::Running));

        let transfers = registry.list();

        assert_eq!(transfers.len(), 1);
        assert_eq!(transfers[0].transfer_id, transfer_id);
        assert_eq!(transfers[0].status, FileTransferStatus::Completed);
    }

    #[test]
    fn cancel_signals_the_worker_without_publishing_a_terminal_status() {
        let mut registry = FileTransferRegistry::default();
        let transfer_id = registry.allocate_transfer_id();
        let cancelled = Arc::new(AtomicBool::new(false));
        let (_completion, receiver) = watch::channel(None);
        registry.begin(
            transfer(transfer_id.clone(), FileTransferStatus::Running),
            cancelled.clone(),
            receiver,
        );

        assert!(matches!(
            registry.cancel(&transfer_id),
            Some(FileTransferCancellation::Pending(_))
        ));
        assert!(cancelled.load(Ordering::Acquire));
        assert_eq!(registry.list()[0].status, FileTransferStatus::Running);

        registry.finish(transfer(transfer_id.clone(), FileTransferStatus::Completed));
        let Some(FileTransferCancellation::Finished(snapshot)) = registry.cancel(&transfer_id)
        else {
            panic!("expected the actual terminal snapshot");
        };
        assert_eq!(snapshot.status, FileTransferStatus::Completed);
    }
}
