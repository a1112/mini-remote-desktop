use mrd_ipc::{FileTransferStatus, FileTransferTaskSnapshot};

/// Service-owned file transfer task snapshots.
#[derive(Debug, Default)]
pub struct FileTransferRegistry {
    next_transfer_seq: u64,
    transfers: Vec<FileTransferTaskSnapshot>,
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
            *existing = transfer;
            return;
        }
        self.transfers.push(transfer);
    }

    pub fn list(&self) -> Vec<FileTransferTaskSnapshot> {
        self.transfers.clone()
    }

    pub fn cancel(&mut self, transfer_id: &str) -> Option<FileTransferTaskSnapshot> {
        let transfer = self
            .transfers
            .iter_mut()
            .find(|candidate| candidate.transfer_id == transfer_id)?;
        if matches!(
            transfer.status,
            FileTransferStatus::Queued | FileTransferStatus::Running
        ) {
            transfer.status = FileTransferStatus::Cancelled;
        }
        Some(transfer.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_ipc::FileTransferTaskSnapshot;

    fn transfer(transfer_id: String, status: FileTransferStatus) -> FileTransferTaskSnapshot {
        FileTransferTaskSnapshot {
            transfer_id,
            status,
            source_device_id: None,
            target_device_id: None,
            transport_kind: "local".to_string(),
            total_entries: 0,
            copied_entries: 0,
            total_bytes: None,
            copied_bytes: 0,
            error: None,
            entries: Vec::new(),
        }
    }

    #[test]
    fn upsert_replaces_existing_transfer_snapshot() {
        let mut registry = FileTransferRegistry::default();
        let transfer_id = registry.allocate_transfer_id();
        registry.upsert(transfer(transfer_id.clone(), FileTransferStatus::Running));
        registry.upsert(transfer(transfer_id.clone(), FileTransferStatus::Completed));

        let transfers = registry.list();

        assert_eq!(transfers.len(), 1);
        assert_eq!(transfers[0].transfer_id, transfer_id);
        assert_eq!(transfers[0].status, FileTransferStatus::Completed);
    }

    #[test]
    fn cancel_marks_active_transfer_cancelled() {
        let mut registry = FileTransferRegistry::default();
        let transfer_id = registry.allocate_transfer_id();
        registry.upsert(transfer(transfer_id.clone(), FileTransferStatus::Running));

        let cancelled = registry.cancel(&transfer_id).expect("cancel transfer");

        assert_eq!(cancelled.status, FileTransferStatus::Cancelled);
        assert_eq!(registry.list()[0].status, FileTransferStatus::Cancelled);
    }
}
