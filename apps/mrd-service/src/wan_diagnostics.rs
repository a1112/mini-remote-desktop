//! Allowlisted diagnostics for verified target installation failures.

use mrd_ipc::{RemoteFailure, RemoteReasonCode};
use mrd_proto::SessionId;
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TargetInstallStage {
    TargetConsentDecision,
    TargetBackendApprove,
    TargetStateSnapshot,
    TargetTrust,
    TargetExpectedGrant,
    TargetBindPeer,
    TargetPairedPolicy,
    TargetBoundGrant,
    PairedPolicy,
    PairedSnapshot,
    PairedGrant,
    PairedScope,
    PairedConsole,
    PairedAgentSelection,
    PairedAgentProtocol,
    PairedAgentRoute,
    PairedEntropy,
    PairedPeerKey,
    PairedIssuer,
    PairedAuthorityIssue,
    PairedServer,
    PairedTargetInsert,
    PairedAgentRequest,
    PairedAck,
    PairedTargetRead,
    PairedDesktopRecheck,
    PairedPolicyRecheck,
    PairedUnknown,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TargetInstallCode {
    GuardRejected,
    StateUnavailable,
    SessionUnavailable,
    Unhealthy,
    CapabilityUnavailable,
    CapabilityBindingMismatch,
    BindingRevoked,
    DesktopChanged,
    ProtocolVersionUnavailable,
    InvalidAuthority,
    Expired,
    CapacityExhausted,
    InvalidPeerKey,
    InvalidPairedAuthority,
    InvalidInput,
    InvalidConsent,
    InvalidRenderAccessUnit,
    ConnectionUnavailable,
    OutboundUnavailable,
    DuplicateRequest,
    Revoked,
    Disconnected,
    Timeout,
    AckRejected,
    UnexpectedAck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TargetInstallDiagnostic {
    pub(crate) stage: TargetInstallStage,
    pub(crate) code: TargetInstallCode,
}

impl TargetInstallDiagnostic {
    pub(crate) const fn new(stage: TargetInstallStage, code: TargetInstallCode) -> Self {
        Self { stage, code }
    }

    pub(crate) fn remote_failure(self) -> RemoteFailure {
        RemoteFailure {
            code: RemoteReasonCode::PolicyChanged,
            message: format!(
                "WAN target authorization failed (stage={}, code={}).",
                public_enum_name(self.stage),
                public_enum_name(self.code)
            ),
            suggested_action: Some("start a new secure WAN session request".into()),
        }
    }

    fn public_record(self, service_pid: u32, session_id: &SessionId) -> Option<String> {
        // Session ids at this boundary come from the verified coordinator
        // identity. Logging adds a separate canonical UUID guard so an
        // authenticated but arbitrary string can never become log payload.
        if service_pid == 0 || !is_canonical_session_id(&session_id.0) {
            return None;
        }
        #[derive(Serialize)]
        struct PublicRecord<'a> {
            event: &'static str,
            service_pid: u32,
            session_id: &'a str,
            stage: TargetInstallStage,
            code: TargetInstallCode,
        }
        serde_json::to_string(&PublicRecord {
            event: "wan_target_install_failed",
            service_pid,
            session_id: &session_id.0,
            stage: self.stage,
            code: self.code,
        })
        .ok()
    }

    pub(crate) fn emit_terminal(self, session_id: &SessionId) {
        let Some(record) = self.public_record(std::process::id(), session_id) else {
            return;
        };
        #[cfg(all(windows, not(test)))]
        write_windows_event(&record);
        #[cfg(all(not(windows), not(test)))]
        tracing::warn!(public_diagnostic = %record, "WAN target installation failed");
        #[cfg(test)]
        let _ = record;
    }
}

// This helper accepts only the two finite enums above, never a request, error
// chain, credential, or identity object.
fn public_enum_name(value: impl Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => name,
        _ => "unknown".into(),
    }
}

fn is_canonical_session_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

#[cfg(all(windows, not(test)))]
fn write_windows_event(record: &str) {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::System::EventLog::{
            DeregisterEventSource, RegisterEventSourceW, ReportEventW, EVENTLOG_WARNING_TYPE,
        },
    };
    let source: Vec<u16> = std::ffi::OsStr::new(crate::windows_service::MRD_WINDOWS_SERVICE_NAME)
        .encode_wide()
        .chain(Some(0))
        .collect();
    let message: Vec<u16> = std::ffi::OsStr::new(record)
        .encode_wide()
        .chain(Some(0))
        .collect();
    if let Ok(handle) = unsafe { RegisterEventSourceW(PCWSTR::null(), PCWSTR(source.as_ptr())) } {
        let strings = [PCWSTR(message.as_ptr())];
        // Event 1 remains the existing SCM lifecycle event. Event 2 has only
        // the bounded public JSON rendered above; sink failures are best effort
        // and never change authorization, cleanup, or session outcomes.
        let _ = unsafe {
            ReportEventW(
                handle,
                EVENTLOG_WARNING_TYPE,
                0,
                2,
                None,
                0,
                Some(&strings),
                None,
            )
        };
        let _ = unsafe { DeregisterEventSource(handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_diagnostic_public_record_contains_only_approved_fields() {
        let diagnostic = TargetInstallDiagnostic::new(
            TargetInstallStage::PairedAgentRequest,
            TargetInstallCode::Timeout,
        );
        let session = SessionId("0a478953-b5c2-4b55-a1f7-efcfb73f080d".into());
        let record = diagnostic
            .public_record(72920, &session)
            .expect("public record");
        let value: serde_json::Value = serde_json::from_str(&record).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "event": "wan_target_install_failed",
                "service_pid": 72920,
                "session_id": session.0,
                "stage": "paired_agent_request",
                "code": "timeout"
            })
        );
        assert_eq!(
            diagnostic.remote_failure().message,
            "WAN target authorization failed (stage=paired_agent_request, code=timeout)."
        );
    }

    #[test]
    fn target_diagnostic_never_accepts_noncanonical_identifiers_as_log_payload() {
        let diagnostic = TargetInstallDiagnostic::new(
            TargetInstallStage::PairedUnknown,
            TargetInstallCode::GuardRejected,
        );
        for id in [
            "password=private-value",
            "0a478953-b5c2-4b55-a1f7-efcfb73f080d\nprivate",
            "",
        ] {
            assert!(diagnostic
                .public_record(72920, &SessionId(id.into()))
                .is_none());
        }
        assert!(diagnostic
            .public_record(0, &SessionId("0a478953-b5c2-4b55-a1f7-efcfb73f080d".into()))
            .is_none());
    }
}
