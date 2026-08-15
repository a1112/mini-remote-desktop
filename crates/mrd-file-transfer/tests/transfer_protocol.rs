use mrd_file_transfer::{
    chunking::{chunk_bytes, verify_chunk},
    paths::{validate_relative_path, PathValidationError},
    protocol::{FileDirection, FileTransferManifest, TransferProvider},
    resume::{atomic_commit, ResumeState},
};

#[test]
fn manifest_round_trip_preserves_remote_direction_and_provider() {
    let manifest = FileTransferManifest::new(
        "transfer-1",
        "session-1",
        FileDirection::Upload,
        "docs/report.txt",
        11,
        4,
        "a".repeat(64),
    )
    .expect("manifest");
    assert_eq!(manifest.provider, TransferProvider::Remote);
    let encoded = serde_json::to_vec(&manifest).expect("encode");
    let decoded: FileTransferManifest = serde_json::from_slice(&encoded).expect("decode");
    assert_eq!(decoded, manifest);
}

#[test]
fn chunks_are_bounded_and_hash_verified() {
    let chunks = chunk_bytes(b"abcdefghij", 4).expect("chunk");
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].offset, 0);
    assert_eq!(chunks[1].offset, 4);
    assert!(verify_chunk(&chunks[0]).is_ok());
    let mut tampered = chunks[0].clone();
    tampered.payload[0] = b'X';
    assert!(verify_chunk(&tampered).is_err());
}

#[test]
fn paths_reject_escape_alternate_stream_and_reserved_names() {
    for path in [
        "../secret.txt",
        "nested/../../secret.txt",
        "report.txt:secret",
        "CON",
    ] {
        assert!(matches!(
            validate_relative_path(path),
            Err(PathValidationError::Rejected(_))
        ));
    }
    assert_eq!(
        validate_relative_path("nested/report.txt").unwrap(),
        "nested/report.txt"
    );
}

#[test]
fn resume_state_requires_contiguous_offsets_and_manifest_hash() {
    let manifest = FileTransferManifest::new(
        "transfer-2",
        "session-1",
        FileDirection::Download,
        "report.txt",
        8,
        4,
        "b".repeat(64),
    )
    .expect("manifest");
    let mut state = ResumeState::new(&manifest);
    assert_eq!(state.accept_offset(0, 4, &manifest).unwrap(), 4);
    assert!(state.accept_offset(8, 0, &manifest).is_err());
    assert!(state.accept_offset(4, 4, &manifest).is_ok());
    assert_eq!(state.completed_bytes(), 8);
}

#[test]
fn atomic_commit_does_not_replace_on_hash_failure() {
    let root = std::env::temp_dir().join(format!("mrd-file-transfer-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("root");
    let temp = root.join("payload.part");
    let dest = root.join("payload.txt");
    std::fs::write(&temp, b"payload").expect("temp");
    std::fs::write(&dest, b"old").expect("dest");
    assert!(atomic_commit(&temp, &dest, "0".repeat(64).as_str()).is_err());
    assert_eq!(std::fs::read(&dest).unwrap(), b"old");
    std::fs::remove_dir_all(root).expect("cleanup");
}
