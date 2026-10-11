//! One-time, offline recovery of an existing relay identity. The operator must
//! stop and isolate the Agent before running this example as its service user.
//! This example never enrolls a node or changes normal Agent validation.

#[cfg(any(target_os = "linux", test))]
mod recovery {
    use std::sync::Arc;

    use mrd_relay_agent::{
        identity::{load_or_create_identity, CertificateState, IdentityFsPort, StoredIdentity},
        runtime::{ClockPort, RuntimeError},
        secure_store::AtomicEnvelopeFile,
    };
    use serde::{Deserialize, Serialize};
    use sha2::{Digest as _, Sha256};
    use zeroize::{Zeroize as _, Zeroizing};

    const MAX_SEALED_BYTES: usize = 512 * 1024;

    #[derive(Debug, PartialEq, Eq)]
    enum RecoveryError {
        InvalidArguments,
        MissingIdentity,
        InvalidIdentity,
        InvalidCertificate,
        RenewalDue,
        BackupMismatch,
        IdentityChanged,
        WriteFailed,
        ProtectedInput,
    }

    impl RecoveryError {
        fn code(&self) -> &'static str {
            match self {
                Self::InvalidArguments => "recovery_invalid_arguments",
                Self::MissingIdentity => "recovery_identity_missing",
                Self::InvalidIdentity => "recovery_identity_invalid",
                Self::InvalidCertificate => "recovery_certificate_invalid",
                Self::RenewalDue => "recovery_renewal_due",
                Self::BackupMismatch => "recovery_backup_mismatch",
                Self::IdentityChanged => "recovery_identity_changed",
                Self::WriteFailed => "recovery_write_failed",
                Self::ProtectedInput => "recovery_protected_input_invalid",
            }
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Command {
        Inspect {
            config: String,
        },
        Install {
            config: String,
            trusted_ca: String,
            certificate_json: String,
            expected_sealed_sha256: String,
            backup_file: String,
        },
    }

    fn parse_command(args: Vec<String>) -> Result<Command, RecoveryError> {
        let operation = args.first().ok_or(RecoveryError::InvalidArguments)?;
        let required: &[&str] = match operation.as_str() {
            "inspect" => &["--config"],
            "install" => &[
                "--config",
                "--trusted-ca",
                "--certificate-json",
                "--expected-sealed-sha256",
                "--backup-file",
            ],
            _ => return Err(RecoveryError::InvalidArguments),
        };
        if args.len() != 1 + 2 * required.len() {
            return Err(RecoveryError::InvalidArguments);
        }
        let mut options = std::collections::BTreeMap::new();
        for pair in args[1..].chunks_exact(2) {
            if !required.contains(&pair[0].as_str())
                || pair[1].is_empty()
                || pair[1].starts_with("--")
                || options.insert(pair[0].as_str(), pair[1].clone()).is_some()
            {
                return Err(RecoveryError::InvalidArguments);
            }
        }
        let mut take = |name| options.remove(name).ok_or(RecoveryError::InvalidArguments);
        match operation.as_str() {
            "inspect" => Ok(Command::Inspect {
                config: take("--config")?,
            }),
            "install" => Ok(Command::Install {
                config: take("--config")?,
                trusted_ca: take("--trusted-ca")?,
                certificate_json: take("--certificate-json")?,
                expected_sealed_sha256: take("--expected-sealed-sha256")?,
                backup_file: take("--backup-file")?,
            }),
            _ => Err(RecoveryError::InvalidArguments),
        }
    }

    #[derive(Debug, Serialize)]
    struct Inspection {
        node_id: String,
        csr_pem: String,
        public_key_b64: String,
        identity_epoch: u64,
        request_sequence: u64,
        sealed_sha256: String,
        active_certificate_present: bool,
        active_certificate_expires_at_unix_seconds: Option<i64>,
    }

    #[derive(Clone, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct CertificateDocument {
        certificate_pem: String,
        ca_certificate_pem: String,
        expires_at_unix_seconds: i64,
        renew_at_unix_seconds: Option<i64>,
    }

    fn digest(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    // A serde Value would otherwise leave copies of the private key and
    // enrollment receipt in memory after the certificate-only transformation.
    struct SensitiveJson(serde_json::Value);
    impl SensitiveJson {
        fn from_identity(identity: &StoredIdentity) -> Result<Self, RecoveryError> {
            let bytes = Zeroizing::new(
                serde_json::to_vec(identity).map_err(|_| RecoveryError::InvalidIdentity)?,
            );
            serde_json::from_slice(&bytes)
                .map(Self)
                .map_err(|_| RecoveryError::InvalidIdentity)
        }
        fn to_identity(&self) -> Result<StoredIdentity, RecoveryError> {
            let bytes = Zeroizing::new(
                serde_json::to_vec(&self.0).map_err(|_| RecoveryError::InvalidIdentity)?,
            );
            serde_json::from_slice(&bytes).map_err(|_| RecoveryError::InvalidIdentity)
        }
    }
    impl Drop for SensitiveJson {
        fn drop(&mut self) {
            fn wipe(value: &mut serde_json::Value) {
                match value {
                    serde_json::Value::String(text) => text.zeroize(),
                    serde_json::Value::Array(values) => values.iter_mut().for_each(wipe),
                    serde_json::Value::Object(values) => values.values_mut().for_each(wipe),
                    _ => {}
                }
            }
            wipe(&mut self.0);
        }
    }

    struct FrozenIdentity(StoredIdentity);
    impl IdentityFsPort for FrozenIdentity {
        fn load(&self) -> Result<Option<StoredIdentity>, RuntimeError> {
            Ok(Some(self.0.clone()))
        }
        fn atomic_replace(&self, _identity: &StoredIdentity) -> Result<(), RuntimeError> {
            Err(RuntimeError::IdentityIo)
        }
        fn enforce_strict_permissions(&self) -> Result<(), RuntimeError> {
            Ok(())
        }
    }

    fn read_sealed(file: &impl AtomicEnvelopeFile) -> Result<Zeroizing<Vec<u8>>, RecoveryError> {
        file.read(MAX_SEALED_BYTES)
            .map_err(|_| RecoveryError::InvalidIdentity)?
            .ok_or(RecoveryError::MissingIdentity)
    }

    fn load_existing(
        store: &impl IdentityFsPort,
        node_id: &str,
    ) -> Result<StoredIdentity, RecoveryError> {
        store
            .enforce_strict_permissions()
            .map_err(|_| RecoveryError::InvalidIdentity)?;
        let stored = store
            .load()
            .map_err(|_| RecoveryError::InvalidIdentity)?
            .ok_or(RecoveryError::MissingIdentity)?;
        // The frozen port can never return None or write: no repair, new key,
        // receipt promotion, certificate validation bypass, or sequence update.
        load_or_create_identity(&FrozenIdentity(stored.clone()), node_id)
            .map_err(|_| RecoveryError::InvalidIdentity)?;
        Ok(stored)
    }

    fn inspect_identity(
        store: &impl IdentityFsPort,
        file: &impl AtomicEnvelopeFile,
        node_id: &str,
    ) -> Result<Inspection, RecoveryError> {
        let sealed = read_sealed(file)?;
        let stored = load_existing(store, node_id)?;
        let value = SensitiveJson::from_identity(&stored)?;
        let text = |name| {
            value.0[name]
                .as_str()
                .map(str::to_owned)
                .ok_or(RecoveryError::InvalidIdentity)
        };
        let report = Inspection {
            node_id: node_id.to_owned(),
            csr_pem: text("csr_pem")?,
            public_key_b64: text("public_key_b64")?,
            identity_epoch: value.0["identity_epoch"]
                .as_u64()
                .ok_or(RecoveryError::InvalidIdentity)?,
            request_sequence: value.0["request_sequence"]
                .as_u64()
                .ok_or(RecoveryError::InvalidIdentity)?,
            sealed_sha256: digest(&sealed),
            active_certificate_present: !value.0["certificate"].is_null(),
            active_certificate_expires_at_unix_seconds: value.0["certificate"]
                ["expires_at_unix_seconds"]
                .as_i64(),
        };
        if read_sealed(file)?.as_slice() != sealed.as_slice() {
            return Err(RecoveryError::IdentityChanged);
        }
        Ok(report)
    }

    #[allow(clippy::too_many_arguments)]
    fn install_identity(
        store: &impl IdentityFsPort,
        file: &impl AtomicEnvelopeFile,
        node_id: &str,
        certificate: &CertificateDocument,
        trusted_ca: &str,
        expected_sha256: &str,
        backup: &[u8],
        clock: Arc<dyn ClockPort>,
    ) -> Result<(), RecoveryError> {
        if expected_sha256.len() != 64
            || !expected_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
        {
            return Err(RecoveryError::InvalidArguments);
        }
        let sealed = read_sealed(file)?;
        if digest(&sealed) != expected_sha256 {
            return Err(RecoveryError::IdentityChanged);
        }
        if backup.is_empty() || backup.len() > MAX_SEALED_BYTES || digest(backup) != expected_sha256
        {
            return Err(RecoveryError::BackupMismatch);
        }
        let stored = load_existing(store, node_id)?;
        if !certificate
            .renew_at_unix_seconds
            .is_some_and(|renew_at| renew_at > clock.unix_seconds())
        {
            return Err(RecoveryError::RenewalDue);
        }
        let mut value = SensitiveJson::from_identity(&stored)?;
        value.0["certificate"] =
            serde_json::to_value(certificate).map_err(|_| RecoveryError::InvalidCertificate)?;
        let candidate = value.to_identity()?;
        // This is exactly the normal startup verifier, applied to an immutable
        // candidate. The original expired certificate is never trusted.
        CertificateState::new(
            Arc::new(FrozenIdentity(candidate.clone())),
            node_id,
            trusted_ca,
            clock.clone(),
        )
        .map_err(|_| RecoveryError::InvalidCertificate)?;
        // Offline service isolation is the operator's responsibility. Recheck
        // the exact original immediately before the existing atomic writer.
        if read_sealed(file)?.as_slice() != sealed.as_slice() {
            return Err(RecoveryError::IdentityChanged);
        }
        if !certificate
            .renew_at_unix_seconds
            .is_some_and(|renew_at| renew_at > clock.unix_seconds())
        {
            return Err(RecoveryError::RenewalDue);
        }
        store
            .atomic_replace(&candidate)
            .map_err(|_| RecoveryError::WriteFailed)
    }

    #[cfg(target_os = "linux")]
    pub fn run() -> Result<(), &'static str> {
        use std::io::Write as _;

        let args = std::env::args_os()
            .skip(1)
            .map(|arg| {
                arg.into_string()
                    .map_err(|_| RecoveryError::InvalidArguments)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.code())?;
        let command = parse_command(args).map_err(|error| error.code())?;
        let result = run_linux(command).map_err(|error| error.code())?;
        let mut output = std::io::stdout().lock();
        serde_json::to_writer(&mut output, &result).map_err(|_| "recovery_output_failed")?;
        output
            .write_all(b"\n")
            .map_err(|_| "recovery_output_failed")
    }

    #[cfg(target_os = "linux")]
    fn run_linux(command: Command) -> Result<serde_json::Value, RecoveryError> {
        use mrd_relay_agent::{
            config::ProductionAgentConfig,
            runtime::SystemClock,
            secure_store::{
                read_linux_integrity_file, HardenedAtomicFile, LinuxPlaintextProtector,
                SecureIdentityStore,
            },
        };
        use std::path::Path;

        // Atomic replacement must retain service ownership. An administrator
        // controls isolation and enrollment externally, never this worker.
        if unsafe { libc::geteuid() } == 0 {
            return Err(RecoveryError::ProtectedInput);
        }
        let config_path = match &command {
            Command::Inspect { config } | Command::Install { config, .. } => Path::new(config),
        };
        let config =
            ProductionAgentConfig::load(config_path).map_err(|_| RecoveryError::ProtectedInput)?;
        let identity_path = &config.agent().identity_path;
        let identity_root = identity_path
            .parent()
            .ok_or(RecoveryError::ProtectedInput)?
            .to_path_buf();
        let file = Arc::new(
            HardenedAtomicFile::new_linux(identity_root, identity_path.clone())
                .map_err(|_| RecoveryError::ProtectedInput)?,
        );
        let node_id = &config.agent().node_id;
        let store = SecureIdentityStore::new(
            file.clone(),
            Arc::new(LinuxPlaintextProtector::new()),
            node_id,
        )
        .map_err(|_| RecoveryError::InvalidIdentity)?;
        match command {
            Command::Inspect { .. } => {
                serde_json::to_value(inspect_identity(&store, file.as_ref(), node_id)?)
                    .map_err(|_| RecoveryError::InvalidIdentity)
            }
            Command::Install {
                trusted_ca,
                certificate_json,
                expected_sealed_sha256,
                backup_file,
                ..
            } => {
                let ca_bytes = read_linux_integrity_file(Path::new(&trusted_ca), 64 * 1024)
                    .map_err(|_| RecoveryError::ProtectedInput)?;
                let ca =
                    std::str::from_utf8(&ca_bytes).map_err(|_| RecoveryError::ProtectedInput)?;
                let certificate_bytes =
                    read_linux_integrity_file(Path::new(&certificate_json), 160 * 1024)
                        .map_err(|_| RecoveryError::ProtectedInput)?;
                let certificate: CertificateDocument =
                    serde_json::from_slice(&certificate_bytes)
                        .map_err(|_| RecoveryError::InvalidCertificate)?;
                let backup_path = Path::new(&backup_file);
                let backup_root = backup_path
                    .parent()
                    .ok_or(RecoveryError::ProtectedInput)?
                    .to_path_buf();
                let backup = HardenedAtomicFile::new_linux(backup_root, backup_path.to_path_buf())
                    .map_err(|_| RecoveryError::ProtectedInput)?
                    .read(MAX_SEALED_BYTES)
                    .map_err(|_| RecoveryError::ProtectedInput)?
                    .ok_or(RecoveryError::BackupMismatch)?;
                install_identity(
                    &store,
                    file.as_ref(),
                    node_id,
                    &certificate,
                    ca,
                    &expected_sealed_sha256,
                    &backup,
                    Arc::new(SystemClock::new()),
                )?;
                let after = inspect_identity(&store, file.as_ref(), node_id)?;
                Ok(serde_json::json!({
                    "installed": true, "node_id": after.node_id, "identity_epoch": after.identity_epoch,
                    "request_sequence": after.request_sequence, "before_sha256": expected_sealed_sha256,
                    "after_sha256": after.sealed_sha256,
                }))
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::Mutex;

        use mrd_relay_agent::{
            identity::load_or_create_identity,
            secure_store::{EnvelopeProtector, SecureIdentityStore, SecureStoreError},
        };
        use rcgen::{
            date_time_ymd, BasicConstraints, Certificate, CertificateParams,
            CertificateSigningRequestParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose,
            IsCa, KeyPair, KeyUsagePurpose, PKCS_ED25519,
        };
        use serde_json::{json, Value};
        use zeroize::Zeroizing;

        const NODE: &str = "relay-recovery-test";
        const NOW: i64 = 1_800_000_000;

        struct TestClock;
        impl ClockPort for TestClock {
            fn monotonic_ms(&self) -> u64 {
                0
            }
            fn unix_seconds(&self) -> i64 {
                NOW
            }
        }

        #[derive(Default)]
        struct MemoryFile {
            bytes: Mutex<Option<Zeroizing<Vec<u8>>>>,
            fail_write: Mutex<bool>,
            change_after_read: Mutex<Option<usize>>,
        }

        impl MemoryFile {
            fn snapshot(&self) -> Zeroizing<Vec<u8>> {
                self.bytes.lock().unwrap().as_ref().unwrap().clone()
            }
        }

        impl AtomicEnvelopeFile for MemoryFile {
            fn canonical_path(&self) -> &str {
                "/var/lib/mrd-relay-agent/identity.envelope"
            }
            fn read(&self, max: usize) -> Result<Option<Zeroizing<Vec<u8>>>, SecureStoreError> {
                let value = self.bytes.lock().unwrap().clone();
                if value.as_ref().is_some_and(|v| v.len() > max) {
                    return Err(SecureStoreError::Invalid);
                }
                let mut countdown = self.change_after_read.lock().unwrap();
                if let Some(count) = countdown.as_mut() {
                    *count -= 1;
                    if *count == 0 {
                        *self.bytes.lock().unwrap() = Some(Zeroizing::new(b"changed".to_vec()));
                        *countdown = None;
                    }
                }
                Ok(value)
            }
            fn atomic_replace(&self, value: &[u8], max: usize) -> Result<(), SecureStoreError> {
                if *self.fail_write.lock().unwrap() || value.len() > max {
                    return Err(SecureStoreError::Io);
                }
                *self.bytes.lock().unwrap() = Some(Zeroizing::new(value.to_vec()));
                Ok(())
            }
            fn enforce_strict_permissions(&self) -> Result<(), SecureStoreError> {
                Ok(())
            }
        }

        // The test-only protector keeps the public secure-store binding and
        // atomic I/O exercised on Windows too. Production uses the Linux port.
        struct TestProtector;
        impl EnvelopeProtector for TestProtector {
            fn seal(
                &self,
                binding: &[u8],
                plaintext: &[u8],
            ) -> Result<Zeroizing<Vec<u8>>, SecureStoreError> {
                let mut value = Zeroizing::new(Sha256::digest(binding).to_vec());
                value.extend_from_slice(plaintext);
                Ok(value)
            }
            fn open(
                &self,
                binding: &[u8],
                sealed: &[u8],
            ) -> Result<Zeroizing<Vec<u8>>, SecureStoreError> {
                if sealed.len() < 32 || sealed[..32] != Sha256::digest(binding)[..] {
                    return Err(SecureStoreError::Invalid);
                }
                Ok(Zeroizing::new(sealed[32..].to_vec()))
            }
        }

        type TestStore = SecureIdentityStore<MemoryFile, TestProtector>;

        struct Authority {
            key: KeyPair,
            cert: Certificate,
        }
        impl Authority {
            fn new() -> Self {
                let key = KeyPair::generate_for(&PKCS_ED25519).unwrap();
                let mut params = CertificateParams::default();
                params.not_before = date_time_ymd(2025, 1, 1);
                params.not_after = date_time_ymd(2035, 1, 1);
                params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
                params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
                let cert = params.self_signed(&key).unwrap();
                Self { key, cert }
            }
            fn issue(&self, csr: &str, profile: &str) -> CertificateDocument {
                let mut csr = CertificateSigningRequestParams::from_pem(csr).unwrap();
                csr.params.not_before = date_time_ymd(2026, 1, 1);
                csr.params.not_after = date_time_ymd(2030, 1, 1);
                csr.params.is_ca = IsCa::ExplicitNoCa;
                csr.params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
                csr.params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
                match profile {
                    "expired" => csr.params.not_after = date_time_ymd(2026, 2, 1),
                    "future" => csr.params.not_before = date_time_ymd(2029, 1, 1),
                    "wrong_cn" => {
                        let mut name = DistinguishedName::new();
                        name.push(DnType::CommonName, "relay-other");
                        csr.params.distinguished_name = name;
                    }
                    "duplicate_san" => csr
                        .params
                        .subject_alt_names
                        .push(csr.params.subject_alt_names[0].clone()),
                    "ca_leaf" => csr.params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained),
                    "server_auth" => csr
                        .params
                        .extended_key_usages
                        .push(ExtendedKeyUsagePurpose::ServerAuth),
                    _ => {}
                }
                let leaf = csr.signed_by(&self.cert, &self.key).unwrap();
                let (_, parsed) = x509_parser::parse_x509_certificate(leaf.der()).unwrap();
                CertificateDocument {
                    certificate_pem: leaf.pem(),
                    ca_certificate_pem: self.cert.pem(),
                    expires_at_unix_seconds: parsed.validity().not_after.timestamp(),
                    renew_at_unix_seconds: Some(NOW + 3600),
                }
            }
        }

        struct Fixture {
            file: Arc<MemoryFile>,
            store: TestStore,
            authority: Authority,
            csr: String,
        }
        impl Fixture {
            fn new() -> Self {
                let file = Arc::new(MemoryFile::default());
                let store = TestStore::new(file.clone(), Arc::new(TestProtector), NODE).unwrap();
                let csr = load_or_create_identity(&store, NODE)
                    .unwrap()
                    .csr_pem()
                    .to_owned();
                let authority = Authority::new();
                let pending_file = Arc::new(MemoryFile::default());
                let pending_store =
                    TestStore::new(pending_file, Arc::new(TestProtector), NODE).unwrap();
                load_or_create_identity(&pending_store, NODE).unwrap();
                let pending = serde_json::to_value(pending_store.load().unwrap().unwrap()).unwrap();
                let mut stored = serde_json::to_value(store.load().unwrap().unwrap()).unwrap();
                stored["certificate"] =
                    serde_json::to_value(authority.issue(&csr, "expired")).unwrap();
                stored["identity_epoch"] = json!(102);
                stored["request_sequence"] = json!(4);
                stored["pending_enrollment"] = json!({"enrollment_id":"enrollment-existing", "receipt":"existing-private-receipt-123"});
                stored["pending_renewal"] = json!({"renewal_id":"renewal-existing", "private_pkcs8_b64":pending["private_pkcs8_b64"], "public_key_b64":pending["public_key_b64"], "csr_pem":pending["csr_pem"], "certificate":null});
                store
                    .atomic_replace(&serde_json::from_value::<StoredIdentity>(stored).unwrap())
                    .unwrap();
                Self {
                    file,
                    store,
                    authority,
                    csr,
                }
            }
            fn install(
                &self,
                certificate: &CertificateDocument,
                backup: &[u8],
                expected: &str,
            ) -> Result<(), RecoveryError> {
                install_identity(
                    &self.store,
                    self.file.as_ref(),
                    NODE,
                    certificate,
                    &self.authority.cert.pem(),
                    expected,
                    backup,
                    Arc::new(TestClock),
                )
            }
            fn value(&self) -> Value {
                serde_json::to_value(self.store.load().unwrap().unwrap()).unwrap()
            }
            fn reject(&self, certificate: &CertificateDocument, expected_error: RecoveryError) {
                let backup = self.file.snapshot();
                assert_eq!(
                    self.install(certificate, &backup, &digest(&backup)),
                    Err(expected_error)
                );
                assert_eq!(*self.file.snapshot(), *backup);
            }
        }

        #[test]
        fn inspect_missing_identity_does_not_generate_a_key() {
            let file = Arc::new(MemoryFile::default());
            let store = TestStore::new(file.clone(), Arc::new(TestProtector), NODE).unwrap();
            assert!(matches!(
                inspect_identity(&store, file.as_ref(), NODE),
                Err(RecoveryError::MissingIdentity)
            ));
            assert!(file.bytes.lock().unwrap().is_none());
        }

        #[test]
        fn inspect_accepts_expired_identity_without_writing_or_exposing_secrets() {
            let f = Fixture::new();
            let before = f.file.snapshot();
            let report = inspect_identity(&f.store, f.file.as_ref(), NODE).unwrap();
            assert_eq!(report.csr_pem, f.csr);
            assert_eq!(report.identity_epoch, 102);
            assert_eq!(report.request_sequence, 4);
            assert_eq!(report.sealed_sha256, digest(&before));
            let output = serde_json::to_string(&report).unwrap();
            let value = f.value();
            for secret in [
                value["private_pkcs8_b64"].as_str().unwrap(),
                "existing-private-receipt-123",
                value["pending_renewal"]["private_pkcs8_b64"]
                    .as_str()
                    .unwrap(),
            ] {
                assert!(!output.contains(secret));
            }
            assert_eq!(*f.file.snapshot(), *before);
        }

        #[test]
        fn inspect_reports_only_the_active_certificate_presence_and_expiry() {
            let f = Fixture::new();
            let report = inspect_identity(&f.store, f.file.as_ref(), NODE).unwrap();
            let public = serde_json::to_value(report).unwrap();
            assert_eq!(public["active_certificate_present"], json!(true));
            assert_eq!(
                public["active_certificate_expires_at_unix_seconds"],
                f.value()["certificate"]["expires_at_unix_seconds"]
            );
            assert!(public.get("certificate_pem").is_none());
            let mut value = f.value();
            value["certificate"] = Value::Null;
            f.store
                .atomic_replace(&serde_json::from_value::<StoredIdentity>(value).unwrap())
                .unwrap();
            let report =
                serde_json::to_value(inspect_identity(&f.store, f.file.as_ref(), NODE).unwrap())
                    .unwrap();
            assert_eq!(report["active_certificate_present"], json!(false));
            assert!(report["active_certificate_expires_at_unix_seconds"].is_null());
        }

        #[test]
        fn install_preserves_every_identity_field_except_active_certificate() {
            let f = Fixture::new();
            let backup = f.file.snapshot();
            let mut before = f.value();
            let cert = f.authority.issue(&f.csr, "valid");
            f.install(&cert, &backup, &digest(&backup)).unwrap();
            let mut after = f.value();
            assert_eq!(after["certificate"], serde_json::to_value(&cert).unwrap());
            before.as_object_mut().unwrap().remove("certificate");
            after.as_object_mut().unwrap().remove("certificate");
            assert_eq!(after, before);
        }

        #[test]
        fn reject_wrong_leaf_profiles_without_writing() {
            for profile in [
                "expired",
                "future",
                "wrong_cn",
                "duplicate_san",
                "ca_leaf",
                "server_auth",
            ] {
                let f = Fixture::new();
                f.reject(
                    &f.authority.issue(&f.csr, profile),
                    RecoveryError::InvalidCertificate,
                );
            }
        }

        #[test]
        fn reject_wrong_signing_key_without_writing() {
            let f = Fixture::new();
            let other = Fixture::new();
            f.reject(
                &f.authority.issue(&other.csr, "valid"),
                RecoveryError::InvalidCertificate,
            );
        }

        #[test]
        fn reject_wrong_ca_without_writing() {
            let f = Fixture::new();
            let other = Authority::new();
            f.reject(
                &other.issue(&f.csr, "valid"),
                RecoveryError::InvalidCertificate,
            );
        }

        #[test]
        fn reject_missing_or_due_renewal_schedule_without_writing() {
            for renew_at in [None, Some(NOW - 1), Some(NOW)] {
                let f = Fixture::new();
                let mut cert = f.authority.issue(&f.csr, "valid");
                cert.renew_at_unix_seconds = renew_at;
                f.reject(&cert, RecoveryError::RenewalDue);
            }
        }

        #[test]
        fn renewal_deadline_must_still_be_future_when_validation_finishes() {
            struct AdvancingClock(std::sync::atomic::AtomicUsize);
            impl ClockPort for AdvancingClock {
                fn monotonic_ms(&self) -> u64 {
                    0
                }
                fn unix_seconds(&self) -> i64 {
                    if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                        NOW
                    } else {
                        NOW + 3600
                    }
                }
            }
            let f = Fixture::new();
            let backup = f.file.snapshot();
            let certificate = f.authority.issue(&f.csr, "valid");
            assert_eq!(
                install_identity(
                    &f.store,
                    f.file.as_ref(),
                    NODE,
                    &certificate,
                    &f.authority.cert.pem(),
                    &digest(&backup),
                    &backup,
                    Arc::new(AdvancingClock(std::sync::atomic::AtomicUsize::new(0)))
                ),
                Err(RecoveryError::RenewalDue)
            );
            assert_eq!(*f.file.snapshot(), *backup);
        }

        #[test]
        fn reject_expiry_timestamp_or_pinned_ca_mismatch_without_writing() {
            let f = Fixture::new();
            let mut cert = f.authority.issue(&f.csr, "valid");
            cert.expires_at_unix_seconds += 1;
            f.reject(&cert, RecoveryError::InvalidCertificate);
            let mut cert = f.authority.issue(&f.csr, "valid");
            cert.ca_certificate_pem = Authority::new().cert.pem();
            f.reject(&cert, RecoveryError::InvalidCertificate);
        }

        #[test]
        fn reject_missing_and_mismatched_backups_without_writing() {
            let f = Fixture::new();
            let before = f.file.snapshot();
            let cert = f.authority.issue(&f.csr, "valid");
            for backup in [b"".as_slice(), b"different".as_slice()] {
                assert_eq!(
                    f.install(&cert, backup, &digest(&before)),
                    Err(RecoveryError::BackupMismatch)
                );
                assert_eq!(*f.file.snapshot(), *before);
            }
        }

        #[test]
        fn reject_stale_expected_identity_hash_without_writing() {
            let f = Fixture::new();
            let backup = f.file.snapshot();
            let cert = f.authority.issue(&f.csr, "valid");
            assert_eq!(
                f.install(&cert, &backup, &"0".repeat(64)),
                Err(RecoveryError::IdentityChanged)
            );
            assert_eq!(*f.file.snapshot(), *backup);
        }

        #[test]
        fn reject_identity_changed_during_validation() {
            let f = Fixture::new();
            let backup = f.file.snapshot();
            let cert = f.authority.issue(&f.csr, "valid");
            *f.file.change_after_read.lock().unwrap() = Some(2);
            assert_eq!(
                f.install(&cert, &backup, &digest(&backup)),
                Err(RecoveryError::IdentityChanged)
            );
            assert_eq!(f.file.snapshot().as_slice(), b"changed");
        }

        #[test]
        fn failed_atomic_replace_leaves_original_envelope_intact() {
            let f = Fixture::new();
            let backup = f.file.snapshot();
            let cert = f.authority.issue(&f.csr, "valid");
            *f.file.fail_write.lock().unwrap() = true;
            assert_eq!(
                f.install(&cert, &backup, &digest(&backup)),
                Err(RecoveryError::WriteFailed)
            );
            assert_eq!(*f.file.snapshot(), *backup);
        }

        #[test]
        fn malformed_identity_is_not_repaired_or_rekeyed() {
            let f = Fixture::new();
            let mut value = f.value();
            value["private_pkcs8_b64"] = json!("bad-key");
            f.store
                .atomic_replace(&serde_json::from_value::<StoredIdentity>(value).unwrap())
                .unwrap();
            let before = f.file.snapshot();
            assert!(matches!(
                inspect_identity(&f.store, f.file.as_ref(), NODE),
                Err(RecoveryError::InvalidIdentity)
            ));
            assert_eq!(*f.file.snapshot(), *before);
        }

        #[test]
        fn certificate_document_rejects_extra_fields() {
            let f = Fixture::new();
            let mut value = serde_json::to_value(f.authority.issue(&f.csr, "valid")).unwrap();
            value["private_pkcs8_b64"] = json!("must-not-be-accepted");
            assert!(serde_json::from_value::<CertificateDocument>(value).is_err());
        }

        fn args(values: &[&str]) -> Vec<String> {
            values.iter().map(|value| (*value).to_owned()).collect()
        }

        #[test]
        fn cli_accepts_only_the_complete_inspect_and_install_operations() {
            assert_eq!(
                parse_command(args(&["inspect", "--config", "/protected/config"])),
                Ok(Command::Inspect {
                    config: "/protected/config".into()
                })
            );
            assert_eq!(
                parse_command(args(&[
                    "install",
                    "--config",
                    "/protected/config",
                    "--trusted-ca",
                    "/protected/ca",
                    "--certificate-json",
                    "/protected/cert",
                    "--expected-sealed-sha256",
                    "hash",
                    "--backup-file",
                    "/private/backup"
                ])),
                Ok(Command::Install {
                    config: "/protected/config".into(),
                    trusted_ca: "/protected/ca".into(),
                    certificate_json: "/protected/cert".into(),
                    expected_sealed_sha256: "hash".into(),
                    backup_file: "/private/backup".into()
                })
            );
        }

        #[test]
        fn cli_rejects_unknown_duplicate_missing_and_secret_arguments() {
            for input in [
                vec![],
                vec!["inspect"],
                vec!["inspect", "--config"],
                vec!["install", "--config", "/config"],
                vec!["inspect", "--config", "/config", "--config", "/other"],
                vec![
                    "inspect",
                    "--config",
                    "/config",
                    "--admin-password",
                    "must-not-enter-helper",
                ],
                vec![
                    "inspect",
                    "--config",
                    "/config",
                    "--enrollment-token",
                    "must-not-enter-helper",
                ],
                vec!["reset", "--config", "/config"],
            ] {
                assert_eq!(
                    parse_command(args(&input)),
                    Err(RecoveryError::InvalidArguments)
                );
            }
        }

        #[test]
        fn every_error_has_a_fixed_redacted_code() {
            for (error, code) in [
                (
                    RecoveryError::InvalidArguments,
                    "recovery_invalid_arguments",
                ),
                (RecoveryError::MissingIdentity, "recovery_identity_missing"),
                (RecoveryError::InvalidIdentity, "recovery_identity_invalid"),
                (
                    RecoveryError::InvalidCertificate,
                    "recovery_certificate_invalid",
                ),
                (RecoveryError::RenewalDue, "recovery_renewal_due"),
                (RecoveryError::BackupMismatch, "recovery_backup_mismatch"),
                (RecoveryError::IdentityChanged, "recovery_identity_changed"),
                (RecoveryError::WriteFailed, "recovery_write_failed"),
                (
                    RecoveryError::ProtectedInput,
                    "recovery_protected_input_invalid",
                ),
            ] {
                assert_eq!(error.code(), code);
            }
        }
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    if let Err(code) = recovery::run() {
        eprintln!("{code}");
        std::process::exit(1);
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("recovery_requires_linux");
        std::process::exit(1);
    }
}
