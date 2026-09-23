//! Keeps the recorded upstream base in sync with the vendored snapshot and its docs.

use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn crate_version_is_exposed() {
    assert_eq!(perldantic_core::VERSION, env!("CARGO_PKG_VERSION"));
}

#[test]
fn upstream_commit_is_a_full_sha() {
    let sha = perldantic_core::UPSTREAM_COMMIT;
    assert_eq!(sha.len(), 40);
    assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn upstream_version_matches_vendored_snapshot() {
    let manifest =
        fs::read_to_string(repo_root().join("upstream/pydantic-core/Cargo.toml")).unwrap();
    let expected = format!("version = \"{}\"", perldantic_core::UPSTREAM_VERSION);
    assert!(
        manifest.contains(&expected),
        "snapshot Cargo.toml lacks {expected}"
    );
}

#[test]
fn upstream_doc_records_commit_and_version() {
    let doc = fs::read_to_string(repo_root().join("upstream/UPSTREAM.md")).unwrap();
    assert!(doc.contains(perldantic_core::UPSTREAM_COMMIT));
    assert!(doc.contains(perldantic_core::UPSTREAM_VERSION));
}

#[test]
fn notice_attributes_pydantic_core() {
    let notice = fs::read_to_string(repo_root().join("NOTICE")).unwrap();
    assert!(notice.contains("pydantic-core"));
    assert!(notice.contains("MIT"));
    assert!(notice.contains("pydantic/json_schema.py"));
}

#[test]
fn conformance_recording_pins_the_upstream_release() {
    let requirements =
        fs::read_to_string(repo_root().join("tools/requirements-record.txt")).unwrap();
    let pin = format!("pydantic-core=={}", perldantic_core::UPSTREAM_VERSION);
    assert!(
        requirements.lines().any(|line| line.trim() == pin),
        "tools/requirements-record.txt must pin {pin}"
    );
}

#[test]
fn json_schema_recording_pins_pydantic_at_the_upstream_commit() {
    let requirements =
        fs::read_to_string(repo_root().join("tools/requirements-record.txt")).unwrap();
    let pin = format!(
        "pydantic @ git+https://github.com/pydantic/pydantic@{}",
        perldantic_core::UPSTREAM_COMMIT
    );
    assert!(
        requirements.lines().any(|line| line.trim() == pin),
        "tools/requirements-record.txt must pin {pin}"
    );
}
