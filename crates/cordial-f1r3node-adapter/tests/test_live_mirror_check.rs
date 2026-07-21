//! Harness-level tests for `live_mirror_check`'s ordered-file write/compare
//! behavior.
//!
//! The ordering seam itself (`ordered_output::latest_finalized_output`) is
//! already covered by unit tests colocated in `ordered_output.rs`. What
//! isn't covered anywhere is the CLI-facing `--write-ordered-file` /
//! `--compare-ordered-file` path in the harness: `write_ordered_hashes` and
//! `compare_ordered_hashes` are pure file+JSON logic with no gRPC/HTTP
//! dependency, so they're cheap to exercise directly here without a live
//! node.
//!
//! ## Why `#[path]` instead of the public library API
//!
//! `write_ordered_hashes`, `compare_ordered_hashes`, and `OrderedComparison`
//! are harness-internal (`pub(crate)`) implementation details of the
//! `live_mirror_check` binary, not part of `cordial-f1r3node-adapter`'s
//! public surface — they have no reason to be. Including the binary's
//! source directly lets this test reach them without widening the crate's
//! public API just to make them testable.
#[path = "../src/bin/live_mirror_check.rs"]
#[allow(dead_code)]
mod live_mirror_check;

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use live_mirror_check::{compare_ordered_hashes, write_ordered_hashes};

/// Build a unique path under the system temp dir so tests can run
/// concurrently (or be retried) without clobbering each other's fixture
/// files.
fn temp_path(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "live_mirror_check_test_{label}_{}_{}.json",
        std::process::id(),
        nanos
    ))
}

fn sample_hashes(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("hash-{i:04}")).collect()
}

/// Deletes its path on drop so a failing assertion doesn't leak fixture
/// files into the system temp dir.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
fn write_then_compare_identical_hashes_is_a_match() {
    let path = temp_path("identical");
    let _guard = TempFile(path.clone());

    let hashes = sample_hashes(5);
    write_ordered_hashes(&path, &hashes).expect("write_ordered_hashes should succeed");

    let comparison =
        compare_ordered_hashes(&path, &hashes).expect("compare_ordered_hashes should succeed");

    assert_eq!(comparison.status, "MATCH");
    assert_eq!(comparison.prefix_relation, "equal");
    assert!(comparison.first_mismatch.is_none());
}

#[test]
fn compare_detects_previous_is_prefix_of_current() {
    // The normal append-only case: the mirror produced more finalized
    // blocks since the file was written, but everything already written
    // still agrees with the new run.
    let path = temp_path("previous_prefix");
    let _guard = TempFile(path.clone());

    let previous = sample_hashes(3);
    write_ordered_hashes(&path, &previous).expect("write_ordered_hashes should succeed");

    let mut current = previous.clone();
    current.push("hash-0003".to_string());
    current.push("hash-0004".to_string());

    let comparison =
        compare_ordered_hashes(&path, &current).expect("compare_ordered_hashes should succeed");

    assert_eq!(comparison.status, "MISMATCH");
    assert_eq!(comparison.prefix_relation, "previous-is-prefix");
    assert_eq!(
        comparison.first_mismatch.as_deref(),
        Some("prev=<end> current=hash-0003")
    );
}

#[test]
fn compare_detects_current_is_prefix_of_previous() {
    // A mirror that rolled back or was rebuilt with fewer finalized blocks
    // than a previously written run.
    let path = temp_path("current_prefix");
    let _guard = TempFile(path.clone());

    let previous = sample_hashes(5);
    write_ordered_hashes(&path, &previous).expect("write_ordered_hashes should succeed");

    let current: Vec<String> = previous.iter().take(3).cloned().collect();

    let comparison =
        compare_ordered_hashes(&path, &current).expect("compare_ordered_hashes should succeed");

    assert_eq!(comparison.status, "MISMATCH");
    assert_eq!(comparison.prefix_relation, "current-is-prefix");
    assert_eq!(
        comparison.first_mismatch.as_deref(),
        Some("prev=hash-0003 current=<end>")
    );
}

#[test]
fn compare_detects_genuine_divergence() {
    // Same length on both sides, but the ordering itself disagrees partway
    // through. This is the case that actually matters for catching a real
    // ordering regression, as opposed to the file simply being stale.
    let path = temp_path("diverged");
    let _guard = TempFile(path.clone());

    let previous = vec![
        "hash-0000".to_string(),
        "hash-0001".to_string(),
        "hash-0002".to_string(),
    ];
    write_ordered_hashes(&path, &previous).expect("write_ordered_hashes should succeed");

    let current = vec![
        "hash-0000".to_string(),
        "hash-0001".to_string(),
        "hash-9999".to_string(),
    ];

    let comparison =
        compare_ordered_hashes(&path, &current).expect("compare_ordered_hashes should succeed");

    assert_eq!(comparison.status, "MISMATCH");
    assert_eq!(comparison.prefix_relation, "diverged");
    assert_eq!(
        comparison.first_mismatch.as_deref(),
        Some("prev=hash-0002 current=hash-9999")
    );
}

#[test]
fn write_ordered_hashes_produces_a_valid_json_array() {
    let path = temp_path("json_shape");
    let _guard = TempFile(path.clone());

    let hashes = sample_hashes(2);
    write_ordered_hashes(&path, &hashes).expect("write_ordered_hashes should succeed");

    let body = fs::read_to_string(&path).expect("written file should be readable");
    let parsed: Vec<String> =
        serde_json::from_str(&body).expect("written file should be valid JSON");
    assert_eq!(parsed, hashes);
}

#[test]
fn compare_against_a_missing_file_is_an_error() {
    // Exercises the "no previous run yet" path — `--compare-ordered-file`
    // pointed at a file that was never written should surface a clear
    // error, not panic or silently treat it as an empty baseline.
    let path = temp_path("missing");
    let result = compare_ordered_hashes(&path, &sample_hashes(1));
    assert!(result.is_err());
}
