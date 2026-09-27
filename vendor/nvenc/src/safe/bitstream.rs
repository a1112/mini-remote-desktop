use std::{ffi::c_void, sync::Arc};

use crate::{safe::encoder::EncoderInternal, sys::result::NVencError};

pub struct BitStream {
    pub(crate) buffer: *mut c_void,
    pub(crate) encoder: Arc<EncoderInternal>,
}

impl Drop for BitStream {
    fn drop(&mut self) {
        finish_cleanup(
            "destroy bitstream",
            self.encoder.destroy_bitstream_buffer(self.buffer),
        );
    }
}

unsafe impl Send for BitStream {}

impl BitStream {
    /// Attempts to lock the bit stream, if `wait` is true it will wait
    /// otherwise a `LockBusy` Error may be returned, in which case the
    /// client should retry in a few milliseconds
    pub fn try_lock(&self, wait: bool) -> Result<BitStreamLockGuard<'_>, NVencError> {
        let lock = self.encoder.lock_bit_stream_buffer(self.buffer, wait)?;
        Ok(BitStreamLockGuard {
            buffer: self,
            data_ptr: lock.bitstream_buffer,
            data_len: lock.bitstream_size_in_bytes,
            release: ReleaseOnce::default(),
        })
    }
}

/// Holds a reference to the `BitStream` and holds the data and associated fields
pub struct BitStreamLockGuard<'a> {
    buffer: &'a BitStream,
    data_ptr: *mut c_void,
    data_len: u32,
    release: ReleaseOnce,
}

impl BitStreamLockGuard<'_> {
    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.data_ptr as _, self.data_len as _) }
    }

    /// Unlock explicitly so callers can retain native resources on failure.
    pub fn unlock(mut self) -> Result<(), NVencError> {
        self.release.run(|| {
            self.buffer
                .encoder
                .unlock_bit_stream_buffer(self.buffer.buffer)
        })
    }
}

impl<'a> Drop for BitStreamLockGuard<'a> {
    fn drop(&mut self) {
        let result = self.release.run(|| {
            self.buffer
                .encoder
                .unlock_bit_stream_buffer(self.buffer.buffer)
        });
        finish_cleanup("unlock bitstream", result);
    }
}

#[derive(Default)]
struct ReleaseOnce {
    attempted: bool,
}

impl ReleaseOnce {
    fn run(&mut self, release: impl FnOnce() -> Result<(), NVencError>) -> Result<(), NVencError> {
        if self.attempted {
            return Ok(());
        }
        // A failing native unlock must be reported to the owner, not retried
        // implicitly when the guard is dropped during error propagation.
        self.attempted = true;
        release()
    }
}

fn finish_cleanup(operation: &str, result: Result<(), NVencError>) {
    if let Err(error) = result {
        // Cleanup must not cause a second panic while another error unwinds.
        use std::io::Write;
        let _ = writeln!(
            std::io::stderr(),
            "NVENC {operation} failed during cleanup: {error:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_explicit_unlock_is_reported_and_not_retried_by_drop() {
        let mut release = ReleaseOnce::default();
        let calls = std::cell::Cell::new(0);
        assert_eq!(
            release.run(|| {
                calls.set(calls.get() + 1);
                Err(NVencError::InvalidDevice)
            }),
            Err(NVencError::InvalidDevice)
        );
        release
            .run(|| {
                calls.set(calls.get() + 1);
                Ok(())
            })
            .unwrap();
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn native_cleanup_failure_does_not_panic_during_drop() {
        assert!(
            std::panic::catch_unwind(|| {
                finish_cleanup("destroy bitstream", Err(NVencError::InvalidDevice));
            })
            .is_ok()
        );
    }
}
