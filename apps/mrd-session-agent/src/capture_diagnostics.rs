//! Finite capture failure records. No authority or media is accepted here.

use mrd_proto::SessionId;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptureFailureCause {
    InitializationFailed,
    EncoderInitializationFailed,
    FrameFailed,
    SourceBoundsFailed,
    SourceDimensionsInvalid,
    ProfileFrameInvalid,
    EncodeFailed,
    EncodedCodecInvalid,
    EncodedUnitOversized,
    SequenceExhausted,
    EncodedUnitInvalid,
    EncodedUnitRejected,
    BindingRejected,
    IpcUnitInvalid,
    OutboundUnavailable,
}

impl CaptureFailureCause {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::InitializationFailed => "capture_initialization_failed",
            Self::EncoderInitializationFailed => "capture_encoder_initialization_failed",
            Self::FrameFailed => "capture_frame_failed",
            Self::SourceBoundsFailed => "capture_source_bounds_failed",
            Self::SourceDimensionsInvalid => "capture_source_dimensions_invalid",
            Self::ProfileFrameInvalid => "capture_profile_frame_invalid",
            Self::EncodeFailed => "capture_encode_failed",
            Self::EncodedCodecInvalid => "capture_encoded_codec_invalid",
            Self::EncodedUnitOversized => "capture_encoded_unit_oversized",
            Self::SequenceExhausted => "capture_sequence_exhausted",
            Self::EncodedUnitInvalid => "capture_encoded_unit_invalid",
            Self::EncodedUnitRejected => "capture_encoded_unit_rejected",
            Self::BindingRejected => "capture_binding_rejected",
            Self::IpcUnitInvalid => "capture_ipc_unit_invalid",
            Self::OutboundUnavailable => "capture_outbound_unavailable",
        }
    }
}

fn public_record(
    agent_pid: u32,
    session_id: &SessionId,
    cause: CaptureFailureCause,
) -> Option<String> {
    let session = &session_id.0;
    if session.len() != 36
        || !session.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
    {
        return None;
    }
    // All string fields are finite names or a validated lowercase UUID. No
    // free-form value, error, authority, or media enters this JSON boundary.
    let record = format!(
        "{{\"event\":\"agent_capture_failed\",\"agent_pid\":{agent_pid},\"session_id\":\"{session}\",\"cause\":\"{}\"}}",
        cause.as_str()
    );
    (record.len() <= 512).then_some(record)
}

fn emit_with_sink(
    session_id: &SessionId,
    cause: CaptureFailureCause,
    stop: Option<&AtomicBool>,
    sink: impl FnOnce(&str) -> Result<(), ()>,
) {
    if stop.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        return;
    }
    if let Some(record) = public_record(std::process::id(), session_id, cause) {
        if stop.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return;
        }
        let _ = sink(&record);
    }
}

pub(crate) fn report_failure(session_id: &SessionId, cause: CaptureFailureCause) {
    emit_with_sink(session_id, cause, None, write_event);
}

pub(crate) fn report_worker_failure(
    session_id: &SessionId,
    cause: CaptureFailureCause,
    stop: &AtomicBool,
) {
    emit_with_sink(session_id, cause, Some(stop), write_event);
}

#[cfg(test)]
fn write_event(record: &str) -> Result<(), ()> {
    test_records()
        .lock()
        .map_err(|_| ())?
        .push(record.to_owned());
    Ok(())
}

#[cfg(all(not(windows), not(test)))]
fn write_event(record: &str) -> Result<(), ()> {
    tracing::warn!(public_capture_failure = record);
    Ok(())
}

#[cfg(all(windows, not(test)))]
fn write_event(record: &str) -> Result<(), ()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::System::EventLog::{
            DeregisterEventSource, RegisterEventSourceW, ReportEventW, EVENTLOG_WARNING_TYPE,
        },
    };
    let source: Vec<u16> = std::ffi::OsStr::new("MiniRemoteDesktop")
        .encode_wide()
        .chain(Some(0))
        .collect();
    let message: Vec<u16> = std::ffi::OsStr::new(record)
        .encode_wide()
        .chain(Some(0))
        .collect();
    let handle =
        unsafe { RegisterEventSourceW(PCWSTR::null(), PCWSTR(source.as_ptr())) }.map_err(|_| ())?;
    let strings = [PCWSTR(message.as_ptr())];
    // Event 1/2 remain the service's existing events. This best-effort Event 3
    // carries only the bounded public record and cannot change Agent outcomes.
    let result = unsafe {
        ReportEventW(
            handle,
            EVENTLOG_WARNING_TYPE,
            0,
            3,
            None,
            0,
            Some(&strings),
            None,
        )
    };
    let _ = unsafe { DeregisterEventSource(handle) };
    result.map_err(|_| ())
}

#[cfg(test)]
fn test_records() -> &'static std::sync::Mutex<Vec<String>> {
    static RECORDS: std::sync::OnceLock<std::sync::Mutex<Vec<String>>> = std::sync::OnceLock::new();
    RECORDS.get_or_init(Default::default)
}

#[cfg(test)]
pub(crate) fn take_test_records(session_id: &SessionId) -> Vec<String> {
    let mut records = test_records().lock().unwrap();
    let mut selected = Vec::new();
    records.retain(|record| {
        if record.contains(&format!("\"session_id\":\"{}\"", session_id.0)) {
            selected.push(record.clone());
            false
        } else {
            true
        }
    });
    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> SessionId {
        SessionId("c4b10cca-eac1-4bbf-980a-9338bb578427".into())
    }

    #[test]
    fn capture_diagnostic_all_causes_have_exact_four_public_fields() {
        use CaptureFailureCause::*;
        for (cause, name) in [
            (InitializationFailed, "capture_initialization_failed"),
            (
                EncoderInitializationFailed,
                "capture_encoder_initialization_failed",
            ),
            (FrameFailed, "capture_frame_failed"),
            (SourceBoundsFailed, "capture_source_bounds_failed"),
            (SourceDimensionsInvalid, "capture_source_dimensions_invalid"),
            (ProfileFrameInvalid, "capture_profile_frame_invalid"),
            (EncodeFailed, "capture_encode_failed"),
            (EncodedCodecInvalid, "capture_encoded_codec_invalid"),
            (EncodedUnitOversized, "capture_encoded_unit_oversized"),
            (SequenceExhausted, "capture_sequence_exhausted"),
            (EncodedUnitInvalid, "capture_encoded_unit_invalid"),
            (EncodedUnitRejected, "capture_encoded_unit_rejected"),
            (BindingRejected, "capture_binding_rejected"),
            (IpcUnitInvalid, "capture_ipc_unit_invalid"),
            (OutboundUnavailable, "capture_outbound_unavailable"),
        ] {
            let record =
                public_record(42, &session(), cause).expect("finite cause must be observable");
            assert_eq!(record, format!(
                "{{\"event\":\"agent_capture_failed\",\"agent_pid\":42,\"session_id\":\"{}\",\"cause\":\"{name}\"}}",
                session().0
            ));
            assert!(record.len() < 256);
            for secret in [
                "token",
                "password",
                "payload",
                "signature",
                "claims",
                "authority",
            ] {
                assert!(!record.contains(secret));
            }
        }
    }

    #[test]
    fn capture_diagnostic_rejects_noncanonical_uuid_without_logging_it() {
        for invalid in [
            "secret-password-token",
            "C4B10CCA-EAC1-4BBF-980A-9338BB578427",
            "c4b10ccaeac14bbf980a9338bb578427",
            "c4b10cca-eac1-4bbf-980a-9338bb57842g",
            "c4b10cca-eac1-4bbf-980a-9338bb578427\n",
            "c4b10cca-eac1-4bbf-980a-9338bb578427\"payload\":1",
        ] {
            assert!(public_record(
                42,
                &SessionId(invalid.into()),
                CaptureFailureCause::FrameFailed
            )
            .is_none());
            emit_with_sink(
                &SessionId(invalid.into()),
                CaptureFailureCause::FrameFailed,
                None,
                |_| panic!("untrusted session text must never reach the sink"),
            );
        }
    }

    #[test]
    fn capture_diagnostic_worker_stop_is_silent_and_live_failure_uses_actual_pid() {
        let session = SessionId("d4b10cca-eac1-4bbf-980a-9338bb578427".into());
        let stop = AtomicBool::new(true);
        report_worker_failure(&session, CaptureFailureCause::EncodeFailed, &stop);
        assert!(take_test_records(&session).is_empty());
        stop.store(false, std::sync::atomic::Ordering::Release);
        report_worker_failure(&session, CaptureFailureCause::EncodeFailed, &stop);
        let records = take_test_records(&session);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0], format!(
            "{{\"event\":\"agent_capture_failed\",\"agent_pid\":{},\"session_id\":\"{}\",\"cause\":\"capture_encode_failed\"}}",
            std::process::id(), session.0
        ));
    }

    #[test]
    fn capture_diagnostic_sink_failure_is_best_effort() {
        let mut calls = 0;
        let caller_result: Result<(), &'static str> = {
            emit_with_sink(&session(), CaptureFailureCause::FrameFailed, None, |_| {
                calls += 1;
                Err(())
            });
            Err("original_capture_error")
        };
        assert_eq!(calls, 1);
        assert_eq!(caller_result, Err("original_capture_error"));
    }
}
