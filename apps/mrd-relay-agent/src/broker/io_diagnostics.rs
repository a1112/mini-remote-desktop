//! Bounded system-call categories for privileged helper failures.

use std::io;

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrokerIoStage {
    RuntimeBuild,
    SocketRequestTimeout,
    WslRequestTimeout,
    WslResponseWrite,
    WslResponseFlush,
    SocketNonblocking,
    SocketFromStd,
    SocketResponseWrite,
    SocketResponseShutdown,
    PrivateMetadata,
    PrivateUnlink,
    DirectoryOpen,
    DirectorySync,
    MonotonicRead,
    MonotonicInvalid,
    MonotonicOverflow,
}

impl BrokerIoStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::RuntimeBuild => "runtime_build",
            Self::SocketRequestTimeout => "socket_request_timeout",
            Self::WslRequestTimeout => "wsl_request_timeout",
            Self::WslResponseWrite => "wsl_response_write",
            Self::WslResponseFlush => "wsl_response_flush",
            Self::SocketNonblocking => "socket_nonblocking",
            Self::SocketFromStd => "socket_from_std",
            Self::SocketResponseWrite => "socket_response_write",
            Self::SocketResponseShutdown => "socket_response_shutdown",
            Self::PrivateMetadata => "private_metadata",
            Self::PrivateUnlink => "private_unlink",
            Self::DirectoryOpen => "directory_open",
            Self::DirectorySync => "directory_sync",
            Self::MonotonicRead => "monotonic_read",
            Self::MonotonicInvalid => "monotonic_invalid",
            Self::MonotonicOverflow => "monotonic_overflow",
        }
    }
}

fn public_record(pid: u32, stage: BrokerIoStage, error: Option<&io::Error>) -> Option<String> {
    let errno = error
        .and_then(io::Error::raw_os_error)
        .filter(|errno| (0..=4095).contains(errno))
        .map_or_else(|| "null".to_owned(), |errno| errno.to_string());
    // Only a finite name, the actual process ID, and a bounded integer can
    // enter the record. Never display the I/O error or any request/path.
    let record = format!(
        "{{\"event\":\"relay_broker_io_failed\",\"pid\":{pid},\"stage\":\"{}\",\"errno\":{errno}}}",
        stage.as_str()
    );
    (record.len() <= 512).then_some(record)
}

fn emit_with_sink(
    stage: BrokerIoStage,
    error: Option<&io::Error>,
    sink: impl FnOnce(&str) -> io::Result<()>,
) {
    if let Some(record) = public_record(std::process::id(), stage, error) {
        let _ = sink(&record);
    }
}

#[doc(hidden)]
pub fn report_broker_io_failure(stage: BrokerIoStage, error: Option<&io::Error>) {
    emit_with_sink(stage, error, write_record);
}

#[cfg(not(test))]
fn write_record(record: &str) -> io::Result<()> {
    use io::Write as _;
    // A broken stderr pipe cannot replace the original failure or exit code.
    writeln!(io::stderr().lock(), "{record}")
}

#[cfg(test)]
fn write_record(record: &str) -> io::Result<()> {
    TEST_RECORDS.with(|records| records.borrow_mut().push(record.to_owned()));
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn io_failed<E>(stage: BrokerIoStage, error: &io::Error, original: E) -> E {
    report_broker_io_failure(stage, Some(error));
    original
}

#[cfg(test)]
std::thread_local! {
    static TEST_RECORDS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(super) fn take_test_records() -> Vec<serde_json::Value> {
    TEST_RECORDS.with(|records| {
        std::mem::take(&mut *records.borrow_mut())
            .into_iter()
            .map(|record| serde_json::from_str(&record).unwrap())
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broker_io_diagnostic_all_stages_have_only_four_bounded_public_fields() {
        use BrokerIoStage::*;
        for (stage, name) in [
            (RuntimeBuild, "runtime_build"),
            (SocketRequestTimeout, "socket_request_timeout"),
            (WslRequestTimeout, "wsl_request_timeout"),
            (WslResponseWrite, "wsl_response_write"),
            (WslResponseFlush, "wsl_response_flush"),
            (SocketNonblocking, "socket_nonblocking"),
            (SocketFromStd, "socket_from_std"),
            (SocketResponseWrite, "socket_response_write"),
            (SocketResponseShutdown, "socket_response_shutdown"),
            (PrivateMetadata, "private_metadata"),
            (PrivateUnlink, "private_unlink"),
            (DirectoryOpen, "directory_open"),
            (DirectorySync, "directory_sync"),
            (MonotonicRead, "monotonic_read"),
            (MonotonicInvalid, "monotonic_invalid"),
            (MonotonicOverflow, "monotonic_overflow"),
        ] {
            let error = io::Error::from_raw_os_error(13);
            let record =
                public_record(u32::MAX, stage, Some(&error)).expect("IO stage must be observable");
            let value: serde_json::Value = serde_json::from_str(&record).unwrap();
            assert_eq!(value.as_object().unwrap().len(), 4);
            assert_eq!(value["event"], "relay_broker_io_failed");
            assert_eq!(value["pid"], u32::MAX);
            assert_eq!(value["stage"], name);
            assert_eq!(value["errno"], 13);
            assert!(record.len() < 256);
            for forbidden in [
                "password",
                "secret",
                "payload",
                "path",
                "arguments",
                "peer",
                "key",
                "certificate",
            ] {
                assert!(!record.contains(forbidden));
            }
        }
    }

    #[test]
    fn broker_io_diagnostic_errno_is_numeric_in_range_or_null() {
        for errno in [i32::MIN, -1, 0, 13, 4095, 4096, i32::MAX] {
            let error = io::Error::from_raw_os_error(errno);
            let record = public_record(1, BrokerIoStage::RuntimeBuild, Some(&error)).unwrap();
            let value: serde_json::Value = serde_json::from_str(&record).unwrap();
            let expected = if (0..=4095).contains(&errno) {
                serde_json::json!(errno)
            } else {
                serde_json::Value::Null
            };
            assert_eq!(value["errno"], expected);
        }
        for stage in [
            BrokerIoStage::SocketRequestTimeout,
            BrokerIoStage::WslRequestTimeout,
            BrokerIoStage::MonotonicInvalid,
            BrokerIoStage::MonotonicOverflow,
        ] {
            let record = public_record(1, stage, None).unwrap();
            assert!(serde_json::from_str::<serde_json::Value>(&record).unwrap()["errno"].is_null());
        }
    }

    #[derive(Debug)]
    struct UndisplayableSecret;
    impl std::fmt::Display for UndisplayableSecret {
        fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            panic!("native error text must never be formatted")
        }
    }
    impl std::error::Error for UndisplayableSecret {}

    #[test]
    fn broker_io_diagnostic_does_not_display_native_error_or_private_context() {
        let error = io::Error::new(io::ErrorKind::Other, UndisplayableSecret);
        let record = public_record(1, BrokerIoStage::PrivateMetadata, Some(&error)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&record).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 4);
        assert!(value["errno"].is_null());
    }

    #[test]
    fn broker_io_diagnostic_sink_failure_keeps_original_failure() {
        let mut calls = 0;
        let original = crate::broker::BrokerRuntimeError::IoFailed;
        emit_with_sink(
            BrokerIoStage::RuntimeBuild,
            Some(&io::Error::from_raw_os_error(24)),
            |_| {
                calls += 1;
                Err(io::Error::new(io::ErrorKind::Other, UndisplayableSecret))
            },
        );
        assert_eq!(calls, 1);
        assert!(matches!(
            original,
            crate::broker::BrokerRuntimeError::IoFailed
        ));
        assert_eq!(original.to_string(), "relay_broker_io_failed");
    }

    #[test]
    fn broker_io_diagnostic_actual_file_error_reports_errno_and_retains_iofailed() {
        let missing = std::env::temp_dir().join(format!(
            "mrd-never-created-private-path-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let error = std::fs::File::open(missing).expect_err("test path must not exist");
        let expected_errno = error
            .raw_os_error()
            .filter(|errno| (0..=4095).contains(errno));
        let returned = io_failed(
            BrokerIoStage::PrivateMetadata,
            &error,
            crate::broker::BrokerRuntimeError::IoFailed,
        );
        assert!(matches!(
            returned,
            crate::broker::BrokerRuntimeError::IoFailed
        ));
        assert_eq!(returned.to_string(), "relay_broker_io_failed");
        let records = take_test_records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["pid"], std::process::id());
        assert_eq!(records[0]["stage"], "private_metadata");
        assert_eq!(records[0]["errno"], serde_json::json!(expected_errno));
    }
}
