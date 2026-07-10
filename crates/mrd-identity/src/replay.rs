#![allow(missing_docs)]

use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReplayError {
    #[error("duplicate nonce")]
    DuplicateNonce,
    #[error("counter rollback")]
    CounterRollback,
}

pub struct ReplayWindow {
    width: u64,
    highest: Option<u64>,
    nonces: BTreeSet<[u8; 16]>,
}

impl ReplayWindow {
    pub fn new(width: u64) -> Self { Self { width: width.max(1), highest: None, nonces: BTreeSet::new() } }
    pub fn accept(&mut self, counter: u64, nonce: [u8; 16]) -> Result<(), ReplayError> {
        if self.nonces.contains(&nonce) { return Err(ReplayError::DuplicateNonce); }
        if self.highest.is_some_and(|highest| counter < highest.saturating_sub(self.width)) {
            return Err(ReplayError::CounterRollback);
        }
        if self.highest.is_some_and(|highest| counter < highest) { return Err(ReplayError::CounterRollback); }
        self.highest = Some(counter);
        self.nonces.insert(nonce);
        Ok(())
    }
}
