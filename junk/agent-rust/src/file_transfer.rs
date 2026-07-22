use anyhow::{Result, anyhow};
use std::collections::{BTreeMap, HashMap};

pub const OP_BEGIN: u8 = 1;
pub const OP_COMPLETE: u8 = 2;
pub const OP_CANCEL: u8 = 3;

#[derive(Debug, Default)]
pub struct FileTransferManager {
    inflight: HashMap<u64, InflightTransfer>,
}

#[derive(Debug)]
struct InflightTransfer {
    total_chunks: u32,
    chunks: BTreeMap<u32, Vec<u8>>,
    checksum: [u8; 16],
}

#[derive(Debug, Clone)]
pub struct CompletedTransfer {
    pub transfer_id: u64,
    pub bytes: Vec<u8>,
}

impl FileTransferManager {
    pub fn handle_control(
        &mut self,
        op: u8,
        transfer_id: u64,
        arg0: u64,
        _arg1: u64,
    ) -> Result<Option<CompletedTransfer>> {
        match op {
            OP_BEGIN => {
                let total_chunks = arg0 as u32;
                self.inflight.insert(
                    transfer_id,
                    InflightTransfer {
                        total_chunks,
                        chunks: BTreeMap::new(),
                        checksum: [0; 16],
                    },
                );
                Ok(None)
            }
            OP_COMPLETE => self.complete_transfer(transfer_id),
            OP_CANCEL => {
                self.inflight.remove(&transfer_id);
                Ok(None)
            }
            _ => Err(anyhow!("unsupported file control op={op}")),
        }
    }

    pub fn handle_chunk(
        &mut self,
        transfer_id: u64,
        chunk_idx: u32,
        total_chunks: u32,
        sha256_16: [u8; 16],
        payload: Vec<u8>,
    ) -> Result<()> {
        let transfer = self
            .inflight
            .get_mut(&transfer_id)
            .ok_or_else(|| anyhow!("unknown transfer_id={transfer_id}"))?;
        if transfer.total_chunks != total_chunks {
            return Err(anyhow!(
                "chunk total mismatch transfer_id={transfer_id}, expected={}, got={total_chunks}",
                transfer.total_chunks
            ));
        }
        if chunk_idx >= total_chunks {
            return Err(anyhow!(
                "chunk index out of range idx={chunk_idx} total={total_chunks}"
            ));
        }
        transfer.checksum = sha256_16;
        transfer.chunks.insert(chunk_idx, payload);
        Ok(())
    }

    fn complete_transfer(&mut self, transfer_id: u64) -> Result<Option<CompletedTransfer>> {
        let transfer = match self.inflight.remove(&transfer_id) {
            Some(v) => v,
            None => return Ok(None),
        };
        if transfer.chunks.len() != transfer.total_chunks as usize {
            return Err(anyhow!(
                "incomplete transfer transfer_id={transfer_id}, expected={} got={}",
                transfer.total_chunks,
                transfer.chunks.len()
            ));
        }
        let mut bytes = Vec::new();
        for idx in 0..transfer.total_chunks {
            let chunk = transfer
                .chunks
                .get(&idx)
                .ok_or_else(|| anyhow!("missing chunk idx={idx}"))?;
            bytes.extend_from_slice(chunk);
        }
        Ok(Some(CompletedTransfer { transfer_id, bytes }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_transfer() {
        let mut m = FileTransferManager::default();
        m.handle_control(OP_BEGIN, 7, 2, 0).expect("begin");
        m.handle_chunk(7, 0, 2, [0xAA; 16], vec![1, 2])
            .expect("chunk0");
        m.handle_chunk(7, 1, 2, [0xAA; 16], vec![3, 4])
            .expect("chunk1");
        let done = m
            .handle_control(OP_COMPLETE, 7, 0, 0)
            .expect("complete")
            .expect("result");
        assert_eq!(done.transfer_id, 7);
        assert_eq!(done.bytes, vec![1, 2, 3, 4]);
    }
}
