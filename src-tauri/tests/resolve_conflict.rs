//! Conflict resolution, end to end: a real `jj resolve` driven by the real
//! jjdiff binary acting as its merge tool.
//!
//! The parser and the writer are unit-tested on their own
//! (`jjdiff-diff::conflict`, `resolve.rs`). What neither can show is that jj
//! accepts what jjdiff hands back — the merge-tool protocol is a contract with
//! another program, and the only way to check a contract is to keep it. So this
//! builds a genuine conflict the way one actually arises, resolves it, and asks
//! jj whether it agrees the conflict is gone.

use std::path::Path;
use std::process::Command;

/// Concurrent `jj git init` is flaky; the same guard as the other suites, local
/// to this process.
static JJ_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn jj_available() -> bool {
    Command::new("jj").arg("--version").output().is_ok()
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("jj")
        .args(["--config", "signing.behavior=drop"])
        .args(args)
        .current_dir(dir)
        .env("JJ_USER", "Test")
        .env("JJ_EMAIL", "test@example.com")
        .output()
        .expect("jj runs")
}

fn jj(dir: &Path, args: &[&str]) {
    let output = run(dir, args);
    assert!(output.status.success(), "jj {args:?}: {}", String::from_utf8_lossy(&output.stderr));
}

fn read(dir: &Path, args: &[&str]) -> String {
    let output = run(dir, args);
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Two siblings editing one line, merged. The ordinary way to get a conflict,
/// and the only way to get one jj will actually store.
fn conflicted_repo(root: &Path) {
    jj(root, &["git", "init", "--colocate", "."]);
    std::fs::write(root.join("f.txt"), "keep\nbase\ntail\n").unwrap();
    jj(root, &["commit", "-m", "base"]);
    jj(root, &["bookmark", "create", "base", "-r", "@-"]);

    std::fs::write(root.join("f.txt"), "keep\nleft\ntail\n").unwrap();
    jj(root, &["describe", "-m", "left"]);
    jj(root, &["bookmark", "create", "left", "-r", "@"]);

    jj(root, &["new", "base"]);
    std::fs::write(root.join("f.txt"), "keep\nright\ntail\n").unwrap();
    jj(root, &["describe", "-m", "right"]);
    jj(root, &["bookmark", "create", "right", "-r", "@"]);

    jj(root, &["new", "left", "right", "-m", "merge"]);
}

#[test]
fn a_resolution_written_as_a_merge_tool_settles_the_conflict() {
    let _guard = JJ_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if !jj_available() {
        eprintln!("skipping: jj not installed");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    conflicted_repo(root);

    // The premise: jj really did store a conflict, and really does materialize
    // it as the marker text the parser is written against.
    let listed = read(root, &["--ignore-working-copy", "resolve", "--list"]);
    assert!(listed.contains("f.txt"), "expected a conflict to resolve: {listed}");
    let materialized = read(root, &["--ignore-working-copy", "file", "show", "-r", "@", "f.txt"]);
    assert!(materialized.contains("<<<<<<<"), "jj materializes markers: {materialized}");
    assert!(
        jjdiff_diff::parse_conflicts(&materialized).conflict_count() >= 1,
        "the parser finds the region jj wrote: {materialized}"
    );

    // What the UI would assemble after the reviewer settled the one region.
    let resolution = root.join("resolution.txt");
    std::fs::write(&resolution, "keep\nleft\ntail\n").unwrap();

    // Exactly what `resolve_conflict` builds, with the test binary in place of
    // the bundle's.
    let binary = env!("CARGO_BIN_EXE_jjdiff-app");
    jj(
        root,
        &[
            "--config",
            &format!("merge-tools.jjdiff-resolve.program=\"{binary}\""),
            "--config",
            &format!(
                "merge-tools.jjdiff-resolve.merge-args=[\"--apply-resolution\",\"{}\",\"$output\"]",
                resolution.display()
            ),
            "resolve",
            "-r",
            "@",
            "--tool",
            "jjdiff-resolve",
            "--",
            "f.txt",
        ],
    );

    let settled = read(root, &["--ignore-working-copy", "file", "show", "-r", "@", "f.txt"]);
    assert_eq!(settled, "keep\nleft\ntail\n", "jj committed the resolution verbatim");

    // jj's own verdict, which is the only one that counts: a file that merely
    // looks resolved but is still a conflict in the tree would pass every
    // assertion above.
    let after = run(root, &["--ignore-working-copy", "resolve", "--list"]);
    let remaining = String::from_utf8_lossy(&after.stdout);
    assert!(
        !remaining.contains("f.txt"),
        "jj still considers it conflicted: {remaining}{}",
        String::from_utf8_lossy(&after.stderr)
    );
}

/// A resolution that still holds fences must fail the tool rather than be
/// written. jj takes a merge tool's output at face value, so the alternative is
/// a file with seven angle brackets in it that jj calls resolved.
#[test]
fn a_resolution_with_fences_left_in_is_refused_and_the_conflict_survives() {
    let _guard = JJ_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if !jj_available() {
        eprintln!("skipping: jj not installed");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    conflicted_repo(root);

    let resolution = root.join("resolution.txt");
    std::fs::write(&resolution, "keep\n<<<<<<< Conflict 1 of 1\nleft\n>>>>>>> ends\ntail\n").unwrap();

    let binary = env!("CARGO_BIN_EXE_jjdiff-app");
    let output = run(
        root,
        &[
            "--config",
            &format!("merge-tools.jjdiff-resolve.program=\"{binary}\""),
            "--config",
            &format!(
                "merge-tools.jjdiff-resolve.merge-args=[\"--apply-resolution\",\"{}\",\"$output\"]",
                resolution.display()
            ),
            "resolve",
            "-r",
            "@",
            "--tool",
            "jjdiff-resolve",
            "--",
            "f.txt",
        ],
    );
    assert!(!output.status.success(), "jj should have aborted the resolution");

    let still = read(root, &["--ignore-working-copy", "resolve", "--list"]);
    assert!(still.contains("f.txt"), "the conflict is left alone: {still}");
}
