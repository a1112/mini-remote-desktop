use mrd_service::windows_service::{
    ServiceControl, ServiceEffect, ServiceLifecycle, ServiceState, SessionChange, ShutdownReason,
};

#[test]
fn windows_service_contract_start_and_stop_order_product_resources() {
    let mut lifecycle = ServiceLifecycle::new();
    assert_eq!(lifecycle.state(), ServiceState::Stopped);

    assert_eq!(
        lifecycle.apply(ServiceControl::Start),
        vec![
            ServiceEffect::InitializeProtectedProductData,
            ServiceEffect::InvalidateExecutionGrants,
            ServiceEffect::StartAgentServer,
            ServiceEffect::StartTransports,
            ServiceEffect::ReportRunning,
        ]
    );
    assert_eq!(lifecycle.state(), ServiceState::Running);

    assert_eq!(
        lifecycle.apply(ServiceControl::Stop),
        vec![
            ServiceEffect::StopAcceptingWork,
            ServiceEffect::StopTransports,
            ServiceEffect::StopAgents,
            ServiceEffect::ReportStopped,
        ]
    );
    assert_eq!(lifecycle.state(), ServiceState::Stopped);
}

#[test]
fn windows_service_contract_preshutdown_uses_the_same_clean_order() {
    let mut lifecycle = ServiceLifecycle::new();
    lifecycle.apply(ServiceControl::Start);
    assert_eq!(
        lifecycle.apply(ServiceControl::PreShutdown),
        vec![
            ServiceEffect::StopAcceptingWork,
            ServiceEffect::StopTransports,
            ServiceEffect::StopAgents,
            ServiceEffect::ReportStopped,
        ]
    );
    assert_eq!(
        lifecycle.last_shutdown_reason(),
        Some(ShutdownReason::PreShutdown)
    );
}

#[test]
fn windows_service_contract_supervises_logon_logoff_and_fast_user_switch() {
    let mut lifecycle = ServiceLifecycle::new();
    lifecycle.apply(ServiceControl::Start);

    assert_eq!(
        lifecycle.apply(ServiceControl::SessionChange(SessionChange::Logon(7))),
        vec![ServiceEffect::LaunchAgent(7)]
    );
    assert!(lifecycle.has_agent(7));
    assert!(lifecycle
        .apply(ServiceControl::SessionChange(SessionChange::Logon(7)))
        .is_empty());

    assert_eq!(
        lifecycle.apply(ServiceControl::SessionChange(SessionChange::Disconnect(7))),
        vec![ServiceEffect::RevokeAgentSession(7)]
    );
    assert!(!lifecycle.has_agent(7));
    assert_eq!(
        lifecycle.apply(ServiceControl::SessionChange(SessionChange::Logon(8))),
        vec![ServiceEffect::LaunchAgent(8)]
    );
    assert_eq!(
        lifecycle.apply(ServiceControl::SessionChange(SessionChange::Logoff(8))),
        vec![ServiceEffect::RevokeAgentSession(8)]
    );
}

#[test]
fn windows_service_contract_restart_preserves_trust_and_invalidates_grants() {
    let mut lifecycle = ServiceLifecycle::new();
    lifecycle.apply(ServiceControl::Start);
    lifecycle.apply(ServiceControl::SessionChange(SessionChange::Logon(7)));
    lifecycle.apply(ServiceControl::Stop);

    let effects = lifecycle.apply(ServiceControl::Start);
    assert!(effects.contains(&ServiceEffect::InitializeProtectedProductData));
    assert!(effects.contains(&ServiceEffect::InvalidateExecutionGrants));
    assert!(!effects.contains(&ServiceEffect::ResetTrustStore));
    assert!(!lifecycle.has_agent(7));
}
