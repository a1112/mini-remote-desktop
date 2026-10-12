//! Allowlisted diagnostics for verified target installation failures.

use mrd_ipc::{RemoteFailure, RemoteReasonCode};
use mrd_proto::SessionId;
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TargetInstallStage {
    TargetGenerationZeroBinding,
    TargetGenerationZeroOfferWait,
    TargetGenerationZeroPeerOpen,
    TargetGenerationZeroAnswerGather,
    TargetGenerationZeroConnectivity,
    TargetGenerationZeroRouteProof,
    TargetGenerationZeroInstall,
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
    BindingMismatch,
    CandidateManifestMismatch,
    CandidateRoleMismatch,
    RouteEvidenceMismatch,
    RuntimeInstallFailed,
    GuardRejected,
    DirectoryNotYetValid,
    BackendUnavailable,
    BackendRejected,
    SignalingUnavailable,
    SignalingRejected,
    ConsentUnavailable,
    ConsentRejected,
    BackendBindingMismatch,
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
        test_records().lock().unwrap().push(record);
    }
}

#[cfg(test)]
fn test_records() -> &'static std::sync::Mutex<Vec<String>> {
    static RECORDS: std::sync::OnceLock<std::sync::Mutex<Vec<String>>> = std::sync::OnceLock::new();
    RECORDS.get_or_init(Default::default)
}

#[cfg(test)]
pub(crate) fn take_test_records(session_id: &SessionId) -> Vec<String> {
    let mut records = test_records().lock().unwrap();
    let mut matching = Vec::new();
    records.retain(|record| {
        if serde_json::from_str::<serde_json::Value>(record).unwrap()["session_id"] == session_id.0
        {
            matching.push(record.clone());
            false
        } else {
            true
        }
    });
    matching
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
    fn postapproval_diagnostic_records_keep_only_finite_public_fields() {
        use TargetInstallCode as Code;
        use TargetInstallStage as Stage;
        let session = SessionId("725a2774-b72e-4203-94a5-1929190550d4".into());
        for stage in [
            Stage::TargetGenerationZeroBinding,
            Stage::TargetGenerationZeroOfferWait,
            Stage::TargetGenerationZeroPeerOpen,
            Stage::TargetGenerationZeroAnswerGather,
            Stage::TargetGenerationZeroConnectivity,
            Stage::TargetGenerationZeroRouteProof,
            Stage::TargetGenerationZeroInstall,
        ] {
            for code in [
                Code::BindingMismatch,
                Code::CandidateManifestMismatch,
                Code::CandidateRoleMismatch,
                Code::RouteEvidenceMismatch,
                Code::RuntimeInstallFailed,
                Code::Timeout,
            ] {
                let record = TargetInstallDiagnostic::new(stage, code)
                    .public_record(31500, &session)
                    .unwrap();
                let value: serde_json::Value = serde_json::from_str(&record).unwrap();
                assert_eq!(value.as_object().unwrap().len(), 5);
                assert!(record.len() <= 512);
                assert_eq!(value["session_id"], session.0);
                assert_eq!(value["service_pid"], 31500);
                assert_eq!(value["event"], "wan_target_install_failed");
                assert!(value["stage"]
                    .as_str()
                    .unwrap()
                    .starts_with("target_generation_zero_"));
                for forbidden in ["password", "credential", "token", "sdp", "private"] {
                    assert!(!record.contains(forbidden));
                }
            }
        }
    }

    #[test]
    fn approval_diagnostic_public_codes_have_only_the_fixed_five_fields() {
        use TargetInstallCode as Code;
        let session = SessionId("3b100535-939e-48e7-af24-45cdd6e82d11".into());
        for (code, name) in [
            (Code::DirectoryNotYetValid, "directory_not_yet_valid"),
            (Code::BackendUnavailable, "backend_unavailable"),
            (Code::BackendRejected, "backend_rejected"),
            (Code::SignalingUnavailable, "signaling_unavailable"),
            (Code::SignalingRejected, "signaling_rejected"),
            (Code::ConsentUnavailable, "consent_unavailable"),
            (Code::ConsentRejected, "consent_rejected"),
            (Code::BackendBindingMismatch, "backend_binding_mismatch"),
        ] {
            let diagnostic =
                TargetInstallDiagnostic::new(TargetInstallStage::TargetBackendApprove, code);
            let record = diagnostic.public_record(31500, &session).unwrap();
            assert!(record.len() <= 512);
            let value: serde_json::Value = serde_json::from_str(&record).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "event": "wan_target_install_failed",
                    "service_pid": 31500,
                    "session_id": session.0,
                    "stage": "target_backend_approve",
                    "code": name,
                })
            );
        }
    }

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
