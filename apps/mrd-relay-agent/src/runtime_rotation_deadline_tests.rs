use super::*;
use crate::{
    backend::{
        DesiredNodeState, EnrollmentStatus, NodeCertificate, PickupRequest,
        RelayBackendClientFactoryPort, RelayNodeState, RenewalRequest, SecretCommitRequest,
        SecretRotationStatus, SecretRotationStatusRequest, SecretUploadRequest,
    },
    identity::StoredIdentity,
    metrics::{CoturnMetrics, MetricsError},
    process::{CoturnSnapshot, LiveAllocationEvidence},
};
use rcgen::{
    date_time_ymd, BasicConstraints, CertificateParams, CertificateSigningRequestParams,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose, PKCS_ED25519,
};
use secrecy::ExposeSecret;
use std::sync::atomic::AtomicUsize;

const NOW: i64 = 1_800_000_000;
const CHALLENGE: &str = "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI";

#[derive(Default)]
struct IdentityStore(Mutex<Option<StoredIdentity>>);

impl IdentityFsPort for IdentityStore {
    fn load(&self) -> Result<Option<StoredIdentity>, RuntimeError> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn atomic_replace(&self, identity: &StoredIdentity) -> Result<(), RuntimeError> {
        *self.0.lock().unwrap() = Some(identity.clone());
        Ok(())
    }

    fn enforce_strict_permissions(&self) -> Result<(), RuntimeError> {
        Ok(())
    }
}

#[derive(Default)]
struct StateStore(Mutex<RuntimeStateSnapshot>);

impl RuntimeStateStorePort for StateStore {
    fn load(&self) -> Result<RuntimeStateSnapshot, RuntimeError> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn atomic_store(&self, state: &RuntimeStateSnapshot) -> Result<(), RuntimeError> {
        *self.0.lock().unwrap() = state.clone();
        Ok(())
    }
}

struct Clock(tokio::time::Instant);

impl ClockPort for Clock {
    fn monotonic_ms(&self) -> u64 {
        self.0.elapsed().as_millis().try_into().unwrap()
    }

    fn unix_seconds(&self) -> i64 {
        NOW + i64::try_from(self.0.elapsed().as_secs()).unwrap()
    }
}

struct Sleeper;

#[async_trait]
impl SleeperPort for Sleeper {
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

struct Jitter;

impl JitterPort for Jitter {
    fn jitter_ms(&self, _upper_exclusive: u64) -> u64 {
        0
    }
}

struct NativeState {
    stopped: AtomicBool,
    warmed: AtomicBool,
    non_evidence: AtomicBool,
    remain_cold: AtomicBool,
    changed_generation: AtomicBool,
    active_allocations: AtomicUsize,
}

struct Metrics(Arc<NativeState>);

#[async_trait]
impl MetricsPort for Metrics {
    async fn collect(&self) -> Result<CoturnMetrics, MetricsError> {
        if self.0.stopped.load(Ordering::SeqCst) || !self.0.warmed.load(Ordering::SeqCst) {
            return Err(MetricsError::Unavailable);
        }
        Ok(CoturnMetrics::default())
    }
}

struct Coturn {
    probe_delay: Option<Duration>,
    applied: Mutex<Vec<u64>>,
    native: Arc<NativeState>,
    drain_error: Mutex<Option<ProcessError>>,
    apply_error: Mutex<Option<ProcessError>>,
    drain_checks: AtomicUsize,
    probes: AtomicUsize,
}

#[async_trait]
impl CoturnRuntimePort for Coturn {
    async fn snapshot(&self) -> Result<CoturnSnapshot, ProcessError> {
        let mut snapshot = CoturnSnapshot::healthy(0, 0).with_generation(7, 2);
        if self.native.stopped.load(Ordering::SeqCst) {
            snapshot.generation = 6;
            snapshot.applied_secret_version = 1;
            snapshot.health = ProcessHealth::Degraded;
        } else if !self.native.warmed.load(Ordering::SeqCst) {
            snapshot.health = ProcessHealth::Failed;
        }
        if self.native.changed_generation.load(Ordering::SeqCst)
            && self.probes.load(Ordering::SeqCst) > 0
        {
            snapshot.generation += 1;
        }
        snapshot.active_allocations = self
            .native
            .active_allocations
            .load(Ordering::SeqCst)
            .try_into()
            .unwrap();
        Ok(snapshot)
    }

    async fn verify_completed_drain(&self, expected: &CoturnSnapshot) -> Result<(), ProcessError> {
        self.drain_checks.fetch_add(1, Ordering::SeqCst);
        assert_eq!(expected.generation, 6);
        assert_eq!(expected.applied_secret_version, 1);
        if let Some(error) = self.drain_error.lock().unwrap().clone() {
            return Err(error);
        }
        Ok(())
    }

    async fn restart(&self) -> Result<(), ProcessError> {
        panic!("rotation must not restart coturn through the supervisor")
    }

    async fn apply_secret(&self, version: u64, _secret: SecretBytes) -> Result<(), ProcessError> {
        self.applied.lock().unwrap().push(version);
        if let Some(error) = self.apply_error.lock().unwrap().clone() {
            return Err(error);
        }
        self.native.stopped.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn set_draining(&self, draining: bool) -> Result<(), ProcessError> {
        assert!(draining);
        Ok(())
    }

    async fn probe_local_allocation(&self) -> Result<AllocationProbeEvidence, ProcessError> {
        if let Some(delay) = self.probe_delay {
            tokio::time::sleep(delay).await;
        } else {
            return std::future::pending().await;
        }
        self.probes.fetch_add(1, Ordering::SeqCst);
        if !self.native.remain_cold.load(Ordering::SeqCst) {
            self.native.warmed.store(true, Ordering::SeqCst);
        }
        if self.native.non_evidence.load(Ordering::SeqCst) {
            return Ok(AllocationProbeEvidence::NonEvidence);
        }
        // This crate-private test constructor leaves the production evidence
        // boundary intact; the worker still checks generation and secret version.
        Ok(AllocationProbeEvidence::Live(
            LiveAllocationEvidence::from_verified_roundtrip([0x77; 32]),
        ))
    }
}

struct Backend {
    heartbeat_successes: AtomicUsize,
    heartbeats: Mutex<Vec<SignedHeartbeat>>,
    commits: Mutex<Vec<SecretCommitRequest>>,
    statuses: Mutex<Vec<SecretRotationStatusRequest>>,
    uploads: Mutex<Vec<SecretUploadRequest>>,
    upload_error: Mutex<Option<BackendError>>,
    hang_commit: bool,
}

#[async_trait]
impl RelayBackendPort for Backend {
    async fn enroll(&self, _request: EnrollmentRequest) -> Result<EnrollmentStatus, BackendError> {
        panic!("an active identity must not enroll")
    }

    async fn pickup(
        &self,
        _request: PickupRequest,
    ) -> Result<Option<NodeCertificate>, BackendError> {
        panic!("an active identity must not pick up")
    }

    async fn renew(&self, _request: RenewalRequest) -> Result<NodeCertificate, BackendError> {
        panic!("this certificate is not due for renewal")
    }

    async fn heartbeat(&self, heartbeat: SignedHeartbeat) -> Result<NodeDirective, BackendError> {
        self.heartbeats.lock().unwrap().push(heartbeat);
        if self
            .heartbeat_successes
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(1)
            })
            .is_err()
        {
            // Stop the otherwise infinite production loop after observing the
            // next advertised version; this is independent of rotation success.
            return Err(BackendError::ProtocolInvalid);
        }
        Ok(NodeDirective {
            identity_epoch: 1,
            sequence: 2,
            state: RelayNodeState::Draining,
            desired: DesiredNodeState {
                draining: true,
                secret_version: 2,
                not_before_unix_seconds: Some(NOW),
                old_credential_deadline_unix_seconds: Some(NOW),
                rotation_challenge: Some(CHALLENGE.into()),
            },
            secret_update: None,
        })
    }

    async fn upload_secret(&self, request: SecretUploadRequest) -> Result<(), BackendError> {
        self.uploads.lock().unwrap().push(request);
        if let Some(error) = self.upload_error.lock().unwrap().clone() {
            return Err(error);
        }
        Ok(())
    }

    async fn commit_secret(&self, request: SecretCommitRequest) -> Result<(), BackendError> {
        self.commits.lock().unwrap().push(request);
        if self.hang_commit {
            return std::future::pending().await;
        }
        Ok(())
    }

    async fn rotation_status(
        &self,
        request: SecretRotationStatusRequest,
    ) -> Result<SecretRotationStatus, BackendError> {
        self.statuses.lock().unwrap().push(request);
        Ok(SecretRotationStatus::CommittedExact {
            active_secret_version: 2,
        })
    }
}

struct Factory(Arc<Backend>);

impl RelayBackendClientFactoryPort for Factory {
    fn build_mtls(
        &self,
        _certificate: &NodeCertificate,
        _private_pkcs8: &[u8],
    ) -> Result<Arc<dyn RelayBackendPort>, BackendError> {
        Ok(self.0.clone())
    }
}

struct Fixture {
    identity: CertificateState<IdentityStore>,
    runtime: AgentRuntime<SwappableRelayBackend, Coturn, Clock, Sleeper, Jitter>,
    backend: Arc<Backend>,
    slot: Arc<SwappableRelayBackend>,
    store: Arc<StateStore>,
    sampler: HeartbeatSampler<Metrics>,
    native: Arc<NativeState>,
}

impl Fixture {
    fn new(phase: SecretRotationPhase, probe_delay: Option<Duration>, hang_commit: bool) -> Self {
        let clock = Arc::new(Clock(tokio::time::Instant::now()));
        let key = KeyPair::generate_for(&PKCS_ED25519).unwrap();
        let mut params = CertificateParams::default();
        params.not_before = date_time_ymd(2025, 1, 1);
        params.not_after = date_time_ymd(2035, 1, 1);
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
        let ca = params.self_signed(&key).unwrap();
        let fs = Arc::new(IdentityStore::default());
        let _identity =
            CertificateState::new(fs.clone(), "relay-hkg-1", &ca.pem(), clock.clone()).unwrap();
        let mut stored = serde_json::to_value(fs.load().unwrap().unwrap()).unwrap();
        let mut csr =
            CertificateSigningRequestParams::from_pem(stored["csr_pem"].as_str().unwrap()).unwrap();
        csr.params.not_before = date_time_ymd(2026, 1, 1);
        csr.params.not_after = date_time_ymd(2030, 1, 1);
        csr.params.is_ca = IsCa::ExplicitNoCa;
        csr.params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        csr.params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let leaf = csr.signed_by(&ca, &key).unwrap();
        let (_, parsed) = x509_parser::parse_x509_certificate(leaf.der()).unwrap();
        stored["certificate"] = serde_json::json!({
            "certificate_pem": leaf.pem(), "ca_certificate_pem": ca.pem(),
            "expires_at_unix_seconds": parsed.validity().not_after.timestamp(),
            "renew_at_unix_seconds": NOW + 86_400,
        });
        fs.atomic_replace(&serde_json::from_value(stored).unwrap())
            .unwrap();
        let identity = CertificateState::new(fs, "relay-hkg-1", &ca.pem(), clock.clone()).unwrap();
        assert!(identity.active_certificate().is_some());

        let mut pending =
            generate_pending_rotation(1, 1, 2, NOW, NOW, CHALLENGE.into(), NOW, 0).unwrap();
        pending.phase = phase;
        if phase == SecretRotationPhase::CommitUnknown {
            pending.applied_generation = Some(7);
            pending.probe_generation = Some(7);
            pending.probe_evidence_sha256 = Some([0x33; 32]);
        }
        let store = Arc::new(StateStore(Mutex::new(RuntimeStateSnapshot {
            identity_epoch: 1,
            last_directive_sequence: 1,
            secret_version: 1,
            secret_digest: Some([1; 32]),
            draining: true,
            pending_rotation: Some(pending),
            ..RuntimeStateSnapshot::default()
        })));
        let native = Arc::new(NativeState {
            stopped: AtomicBool::new(false),
            warmed: AtomicBool::new(true),
            non_evidence: AtomicBool::new(false),
            remain_cold: AtomicBool::new(false),
            changed_generation: AtomicBool::new(false),
            active_allocations: AtomicUsize::new(0),
        });
        let backend = Arc::new(Backend {
            heartbeat_successes: AtomicUsize::new(usize::from(
                phase != SecretRotationPhase::CommitUnknown,
            )),
            heartbeats: Mutex::default(),
            commits: Mutex::default(),
            statuses: Mutex::default(),
            uploads: Mutex::default(),
            upload_error: Mutex::default(),
            hang_commit,
        });
        let slot = Arc::new(SwappableRelayBackend::new(backend.clone()));
        let runtime = AgentRuntime::new_with_state_store(
            slot.clone(),
            Arc::new(Coturn {
                probe_delay,
                applied: Mutex::default(),
                native: native.clone(),
                drain_error: Mutex::default(),
                apply_error: Mutex::default(),
                drain_checks: AtomicUsize::new(0),
                probes: AtomicUsize::new(0),
            }),
            clock,
            Arc::new(Sleeper),
            Arc::new(Jitter),
            store.clone(),
        )
        .unwrap();
        let sampler = HeartbeatSampler::new(
            Arc::new(Metrics(native.clone())),
            vec!["turn:relay.example:3478?transport=udp".into()],
            100,
            1_000_000,
        )
        .unwrap();
        Self {
            identity,
            runtime,
            backend,
            slot,
            store,
            sampler,
            native,
        }
    }

    async fn run(&mut self, once: bool) -> Result<(), RuntimeError> {
        let mut lifecycle = IdentityLifecycle::new(Duration::from_secs(86_400)).unwrap();
        let factory = Factory(self.backend.clone());
        if once {
            backend_worker_once_impl(
                &mut lifecycle,
                &mut self.identity,
                self.backend.as_ref(),
                self.slot.as_ref(),
                &factory,
                None,
                &mut self.runtime,
                &self.sampler,
                ProcessHealth::Healthy,
                ProcessHealth::Healthy,
                RelayHealth::NonEvidence,
                HostPressureSnapshot::default(),
                false,
            )
            .await
            .map(|_| ())
        } else {
            run_backend_worker(
                &mut lifecycle,
                &mut self.identity,
                self.backend.clone(),
                self.slot.clone(),
                &factory,
                None,
                &mut self.runtime,
                &self.sampler,
                &SharedRelayHealth::default(),
                HostPressureSnapshot::default(),
            )
            .await
        }
    }

    fn heartbeat_versions(&self) -> Vec<u64> {
        self.backend
            .heartbeats
            .lock()
            .unwrap()
            .iter()
            .map(|heartbeat| {
                serde_json::from_slice::<HeartbeatPayload>(&heartbeat.body)
                    .unwrap()
                    .applied_secret_version
            })
            .collect()
    }

    fn assert_committed(&self) {
        let state = self.store.load().unwrap();
        assert_eq!(state.secret_version, 2);
        assert!(state.pending_rotation.is_none());
    }
}

#[tokio::test(start_paused = true)]
async fn production_slow_rotation_probe_commits_before_advertising_new_version() {
    let mut fixture = Fixture::new(
        SecretRotationPhase::Uploaded,
        Some(Duration::from_secs(4)),
        false,
    );
    assert_eq!(
        fixture.run(false).await,
        Err(RuntimeError::Backend(BackendError::ProtocolInvalid))
    );
    fixture.assert_committed();
    assert_eq!(fixture.backend.commits.lock().unwrap().len(), 1);
    assert_eq!(fixture.heartbeat_versions(), vec![2, 2]);
}

#[tokio::test(start_paused = true)]
async fn production_slow_rotation_reconciliation_precedes_heartbeat() {
    let mut fixture = Fixture::new(
        SecretRotationPhase::CommitUnknown,
        Some(Duration::from_secs(4)),
        false,
    );
    assert_eq!(
        fixture.run(false).await,
        Err(RuntimeError::Backend(BackendError::ProtocolInvalid))
    );
    fixture.assert_committed();
    assert_eq!(fixture.backend.statuses.lock().unwrap().len(), 1);
    assert!(fixture.backend.commits.lock().unwrap().is_empty());
    assert_eq!(fixture.heartbeat_versions(), vec![2]);
}

#[tokio::test(start_paused = true)]
async fn once_slow_rotation_probe_uses_transaction_budget() {
    let mut fixture = Fixture::new(
        SecretRotationPhase::Uploaded,
        Some(Duration::from_secs(4)),
        false,
    );
    assert_eq!(fixture.run(true).await, Ok(()));
    fixture.assert_committed();
    assert_eq!(fixture.backend.commits.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn once_slow_rotation_reconciliation_uses_transaction_budget() {
    let mut fixture = Fixture::new(
        SecretRotationPhase::CommitUnknown,
        Some(Duration::from_secs(4)),
        false,
    );
    assert_eq!(
        fixture.run(true).await,
        Err(RuntimeError::Backend(BackendError::ProtocolInvalid))
    );
    fixture.assert_committed();
    assert_eq!(fixture.heartbeat_versions(), vec![2]);
}

#[tokio::test(start_paused = true)]
async fn production_hung_rotation_probe_exits_with_applied_candidate_intact() {
    let mut fixture = Fixture::new(SecretRotationPhase::Uploaded, None, false);
    let original = fixture.store.load().unwrap().pending_rotation.unwrap();
    let start = tokio::time::Instant::now();
    assert_eq!(
        fixture.run(false).await,
        Err(RuntimeError::Process(ProcessError::ProbeUnavailable))
    );
    assert_eq!(start.elapsed(), Duration::from_secs(60));
    let state = fixture.store.load().unwrap();
    assert_eq!(state.secret_version, 1);
    assert_eq!(state.secret_digest, Some([1; 32]));
    let pending = state.pending_rotation.unwrap();
    assert_eq!(pending.phase, SecretRotationPhase::Applied);
    assert_eq!(pending.rotation_id, original.rotation_id);
    assert_eq!(pending.turn_rest_secret, original.turn_rest_secret);
    assert_eq!(pending.applied_generation, Some(7));
    assert_eq!(pending.probe_evidence_sha256, None);
    assert!(fixture.backend.commits.lock().unwrap().is_empty());
    assert!(fixture.heartbeat_versions().is_empty());
}

#[tokio::test(start_paused = true)]
async fn production_hung_rotation_commit_exits_with_unknown_outcome_intact() {
    let mut fixture = Fixture::new(SecretRotationPhase::Uploaded, Some(Duration::ZERO), true);
    let original = fixture.store.load().unwrap().pending_rotation.unwrap();
    let start = tokio::time::Instant::now();
    assert_eq!(
        fixture.run(false).await,
        Err(RuntimeError::Process(ProcessError::ProbeUnavailable))
    );
    assert_eq!(start.elapsed(), Duration::from_secs(60));
    let state = fixture.store.load().unwrap();
    assert_eq!(state.secret_version, 1);
    let pending = state.pending_rotation.unwrap();
    assert_eq!(pending.phase, SecretRotationPhase::CommitUnknown);
    assert_eq!(pending.rotation_id, original.rotation_id);
    assert_eq!(pending.turn_rest_secret, original.turn_rest_secret);
    assert_eq!(pending.probe_generation, Some(7));
    assert_eq!(pending.probe_evidence_sha256, Some([0x77; 32]));
    assert_eq!(fixture.backend.commits.lock().unwrap().len(), 1);
    assert!(fixture.heartbeat_versions().is_empty());
}

#[tokio::test(start_paused = true)]
async fn production_hung_rotation_reconciliation_never_advertises_old_version() {
    let mut fixture = Fixture::new(SecretRotationPhase::CommitUnknown, None, false);
    let original = fixture.store.load().unwrap().pending_rotation.unwrap();
    let start = tokio::time::Instant::now();
    assert_eq!(
        fixture.run(false).await,
        Err(RuntimeError::Backend(BackendError::Unavailable))
    );
    assert_eq!(start.elapsed(), Duration::from_secs(60));
    let state = fixture.store.load().unwrap();
    assert_eq!(state.secret_version, 1);
    assert_eq!(state.pending_rotation.unwrap(), original);
    assert!(fixture.backend.heartbeats.lock().unwrap().is_empty());
    assert!(fixture.backend.commits.lock().unwrap().is_empty());
}

fn stopped_uploaded_fixture() -> Fixture {
    let fixture = Fixture::new(SecretRotationPhase::Uploaded, Some(Duration::ZERO), false);
    fixture.native.stopped.store(true, Ordering::SeqCst);
    fixture.native.warmed.store(false, Ordering::SeqCst);
    fixture
}

#[tokio::test(start_paused = true)]
async fn production_stopped_uploaded_rotation_resumes_before_native_metrics() {
    let mut fixture = stopped_uploaded_fixture();
    fixture
        .backend
        .heartbeat_successes
        .store(0, Ordering::SeqCst);
    let original = fixture.store.load().unwrap().pending_rotation.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(15), fixture.run(false)).await;
    assert_eq!(
        result,
        Ok(Err(RuntimeError::Backend(BackendError::ProtocolInvalid))),
        "pending Uploaded must not depend on a heartbeat whose native metrics are unavailable"
    );
    fixture.assert_committed();
    let uploads = fixture.backend.uploads.lock().unwrap();
    assert_eq!(uploads.len(), 1);
    assert_eq!(uploads[0].identity_epoch, original.identity_epoch);
    assert_eq!(uploads[0].rotation_id, original.rotation_id);
    assert_eq!(uploads[0].secret_version, original.secret_version);
    assert_eq!(
        uploads[0].turn_rest_secret.expose_secret(),
        original.turn_rest_secret.expose()
    );
    assert_eq!(
        fixture.runtime.coturn.applied.lock().unwrap().as_slice(),
        &[2]
    );
    assert_eq!(fixture.heartbeat_versions(), vec![2]);
}

#[tokio::test(start_paused = true)]
async fn once_stopped_uploaded_rotation_resumes_before_native_metrics() {
    let mut fixture = stopped_uploaded_fixture();
    assert_eq!(fixture.run(true).await, Ok(()));
    fixture.assert_committed();
    assert_eq!(fixture.heartbeat_versions(), vec![2]);
}

#[tokio::test(start_paused = true)]
async fn stopped_pending_rotation_rejects_revoked_identity_before_apply() {
    let mut fixture = stopped_uploaded_fixture();
    *fixture.backend.upload_error.lock().unwrap() = Some(BackendError::Unauthorized);
    assert_eq!(
        fixture.run(true).await,
        Err(RuntimeError::Backend(BackendError::Unauthorized))
    );
    assert!(fixture.runtime.coturn.applied.lock().unwrap().is_empty());
    assert!(fixture.backend.commits.lock().unwrap().is_empty());
    assert!(fixture.backend.heartbeats.lock().unwrap().is_empty());
    assert_eq!(fixture.store.load().unwrap().secret_version, 1);
}

#[tokio::test(start_paused = true)]
async fn stopped_pending_rotation_rejects_stale_upload_body_before_apply() {
    let mut fixture = stopped_uploaded_fixture();
    *fixture.backend.upload_error.lock().unwrap() = Some(BackendError::Conflict);
    assert_eq!(
        fixture.run(true).await,
        Err(RuntimeError::Backend(BackendError::Conflict))
    );
    assert!(fixture.runtime.coturn.applied.lock().unwrap().is_empty());
    assert!(fixture.backend.commits.lock().unwrap().is_empty());
    assert!(fixture.backend.heartbeats.lock().unwrap().is_empty());
    assert_eq!(fixture.store.load().unwrap().secret_version, 1);
}

#[tokio::test(start_paused = true)]
async fn stopped_pending_rotation_requires_verified_drain_before_physical_apply() {
    let mut fixture = stopped_uploaded_fixture();
    *fixture.runtime.coturn.drain_error.lock().unwrap() = Some(ProcessError::ProbeInvalid);
    assert_eq!(
        fixture.run(true).await,
        Err(RuntimeError::Process(ProcessError::ProbeInvalid))
    );
    assert_eq!(
        fixture.runtime.coturn.drain_checks.load(Ordering::SeqCst),
        1
    );
    assert!(fixture.runtime.coturn.applied.lock().unwrap().is_empty());
    assert!(fixture.backend.commits.lock().unwrap().is_empty());
    assert_eq!(fixture.store.load().unwrap().secret_version, 1);
}

#[tokio::test(start_paused = true)]
async fn already_bound_applied_cold_generation_warms_by_real_probe_without_apply() {
    let mut fixture = Fixture::new(SecretRotationPhase::Applied, Some(Duration::ZERO), false);
    fixture.native.warmed.store(false, Ordering::SeqCst);
    fixture
        .runtime
        .pending_rotation
        .as_mut()
        .unwrap()
        .applied_generation = Some(7);
    assert_eq!(fixture.run(true).await, Ok(()));
    fixture.assert_committed();
    assert_eq!(fixture.runtime.coturn.probes.load(Ordering::SeqCst), 1);
    assert!(fixture.runtime.coturn.applied.lock().unwrap().is_empty());
    assert_eq!(
        fixture.runtime.coturn.drain_checks.load(Ordering::SeqCst),
        0
    );
}

#[tokio::test(start_paused = true)]
async fn unbound_same_version_cold_restart_verifies_material_then_real_probe() {
    let mut fixture = Fixture::new(SecretRotationPhase::Applied, Some(Duration::ZERO), false);
    fixture.native.warmed.store(false, Ordering::SeqCst);
    assert_eq!(fixture.run(true).await, Ok(()));
    fixture.assert_committed();
    assert_eq!(
        fixture.runtime.coturn.applied.lock().unwrap().as_slice(),
        &[2]
    );
    assert_eq!(fixture.runtime.coturn.probes.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.runtime.coturn.drain_checks.load(Ordering::SeqCst),
        0
    );
}

#[tokio::test(start_paused = true)]
async fn resumed_rotation_non_evidence_never_commits_or_advertises_candidate() {
    let mut fixture = stopped_uploaded_fixture();
    fixture.native.non_evidence.store(true, Ordering::SeqCst);
    assert_eq!(
        fixture.run(true).await,
        Err(RuntimeError::Process(ProcessError::ProbeInvalid))
    );
    assert_eq!(fixture.store.load().unwrap().secret_version, 1);
    assert!(fixture.backend.commits.lock().unwrap().is_empty());
    assert!(fixture.backend.heartbeats.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn resumed_rotation_generation_change_during_probe_never_commits() {
    let mut fixture = stopped_uploaded_fixture();
    fixture
        .native
        .changed_generation
        .store(true, Ordering::SeqCst);
    assert_eq!(
        fixture.run(true).await,
        Err(RuntimeError::Process(ProcessError::ProbeInvalid))
    );
    assert_eq!(fixture.store.load().unwrap().secret_version, 1);
    assert!(fixture.backend.commits.lock().unwrap().is_empty());
    assert!(fixture.backend.heartbeats.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn waiting_rotation_does_not_turn_unavailable_metrics_into_a_heartbeat() {
    let mut fixture = stopped_uploaded_fixture();
    fixture
        .runtime
        .pending_rotation
        .as_mut()
        .unwrap()
        .old_credential_deadline_unix_seconds = NOW + 30;
    assert_eq!(
        fixture.run(true).await,
        Err(RuntimeError::MetricsUnavailable)
    );
    assert!(fixture.runtime.coturn.applied.lock().unwrap().is_empty());
    assert!(fixture.backend.heartbeats.lock().unwrap().is_empty());
    assert_eq!(
        fixture.runtime.coturn.drain_checks.load(Ordering::SeqCst),
        0
    );
    assert_eq!(fixture.store.load().unwrap().secret_version, 1);
}

#[tokio::test(start_paused = true)]
async fn live_allocations_still_delay_resumed_rotation() {
    let mut fixture = stopped_uploaded_fixture();
    fixture.native.active_allocations.store(1, Ordering::SeqCst);
    assert_eq!(
        fixture.run(true).await,
        Err(RuntimeError::MetricsUnavailable)
    );
    assert!(fixture.runtime.coturn.applied.lock().unwrap().is_empty());
    assert!(fixture.backend.heartbeats.lock().unwrap().is_empty());
    assert_eq!(
        fixture.runtime.coturn.drain_checks.load(Ordering::SeqCst),
        0
    );
}

#[tokio::test(start_paused = true)]
async fn stopped_intent_uses_existing_upload_before_resuming_rotation() {
    let mut fixture = stopped_uploaded_fixture();
    fixture.runtime.pending_rotation.as_mut().unwrap().phase = SecretRotationPhase::Intent;
    assert_eq!(fixture.run(true).await, Ok(()));
    fixture.assert_committed();
    assert_eq!(fixture.backend.uploads.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn unbound_same_version_bad_material_is_rejected_before_probe() {
    let mut fixture = Fixture::new(SecretRotationPhase::Applied, Some(Duration::ZERO), false);
    fixture.native.warmed.store(false, Ordering::SeqCst);
    *fixture.runtime.coturn.apply_error.lock().unwrap() = Some(ProcessError::SecretApplyFailed);
    assert_eq!(
        fixture.run(true).await,
        Err(RuntimeError::Process(ProcessError::SecretApplyFailed))
    );
    assert_eq!(
        fixture.runtime.coturn.applied.lock().unwrap().as_slice(),
        &[2]
    );
    assert_eq!(fixture.runtime.coturn.probes.load(Ordering::SeqCst), 0);
    assert!(fixture.backend.commits.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn real_probe_without_healthy_native_snapshot_never_commits() {
    let mut fixture = stopped_uploaded_fixture();
    fixture.native.remain_cold.store(true, Ordering::SeqCst);
    assert_eq!(
        fixture.run(true).await,
        Err(RuntimeError::Process(ProcessError::ProbeInvalid))
    );
    assert_eq!(fixture.runtime.coturn.probes.load(Ordering::SeqCst), 1);
    assert!(fixture.backend.commits.lock().unwrap().is_empty());
    assert_eq!(fixture.store.load().unwrap().secret_version, 1);
}

#[tokio::test(start_paused = true)]
async fn pending_rotation_wrong_identity_epoch_is_rejected_before_upload() {
    let mut fixture = stopped_uploaded_fixture();
    fixture
        .runtime
        .pending_rotation
        .as_mut()
        .unwrap()
        .identity_epoch = 2;
    assert_eq!(fixture.run(true).await, Err(RuntimeError::DirectiveReplay));
    assert!(fixture.backend.uploads.lock().unwrap().is_empty());
    assert!(fixture.runtime.coturn.applied.lock().unwrap().is_empty());
    assert!(fixture.backend.commits.lock().unwrap().is_empty());
}
