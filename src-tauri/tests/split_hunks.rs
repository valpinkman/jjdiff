//! Hunk-level split and squash, end to end: a real `jj split` and a real
//! `jj squash` driven by the real jjdiff binary acting as their diff editor.
//!
//! Everything below the surface is unit-tested elsewhere — the arithmetic in
//! `jjdiff-diff::apply`, the directory edits in `split.rs`, the argv in
//! `cli.rs`. What none of those can show is that the three fit together
//! through jj's diff-editor protocol, which is a contract with another program
//! and cannot be checked by reading either side of it. So these run the whole
//! loop: plan → `jj <verb> --tool` → jj re-enters the binary → the commits move.

use std::path::Path;
use std::process::Command;

/// Concurrent `jj git init` is flaky, and the crates' own tests take the same
/// precaution. This is a separate process from those, so the guard is local.
static JJ_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn jj_available() -> bool {
    Command::new("jj").arg("--version").output().is_ok()
}

fn jj(dir: &Path, args: &[&str]) {
    let output = Command::new("jj")
        .args(["--config", "signing.behavior=drop"])
        .args(args)
        .current_dir(dir)
        .env("JJ_USER", "Test")
        .env("JJ_EMAIL", "test@example.com")
        .output()
        .expect("jj runs");
    assert!(output.status.success(), "jj {args:?}: {}", String::from_utf8_lossy(&output.stderr));
}

fn diff(dir: &Path, revset: &str) -> String {
    let output = Command::new("jj")
        .args(["--ignore-working-copy", "--color=never", "diff", "--git", "-r", revset])
        .current_dir(dir)
        .output()
        .expect("jj runs");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Two independent edits to one file; the reviewer takes the first only.
#[test]
fn a_selected_hunk_becomes_its_own_change() {
    let _guard = JJ_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if !jj_available() {
        eprintln!("skipping: jj not installed");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    jj(root, &["git", "init", "--colocate", "."]);
    std::fs::write(root.join("f.txt"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
    jj(root, &["commit", "-m", "base"]);
    std::fs::write(root.join("f.txt"), "ONE\ntwo\nthree\nfour\nFIVE\n").unwrap();
    jj(root, &["describe", "-m", "both edits"]);

    // The plan the frontend would send: hunk 0 in, hunk 1 out.
    let plan = serde_json::json!({
        "files": [{
            "path": "f.txt",
            "oldPath": null,
            "select": "hunks",
            "hunks": [
                { "selected": true, "oldStart": 1, "oldLines": 2, "lines": [
                    { "kind": "removed", "text": "one" },
                    { "kind": "added", "text": "ONE" },
                    { "kind": "context", "text": "two" }
                ]},
                { "selected": false, "oldStart": 4, "oldLines": 2, "lines": [
                    { "kind": "context", "text": "four" },
                    { "kind": "removed", "text": "five" },
                    { "kind": "added", "text": "FIVE" }
                ]}
            ]
        }]
    });
    let plan_path = root.join("plan.json");
    std::fs::write(&plan_path, plan.to_string()).unwrap();

    // Exactly what `split_hunks` builds, with the test binary in place of the
    // bundle's.
    let binary = env!("CARGO_BIN_EXE_jjdiff-app");
    let program = format!("merge-tools.jjdiff-split.program=\"{binary}\"");
    let edit_args = format!(
        "merge-tools.jjdiff-split.edit-args=[\"--apply-split-plan\",\"{}\",\"$left\",\"$right\"]",
        plan_path.display()
    );
    jj(
        root,
        &[
            "--config", &program,
            "--config", &edit_args,
            "split", "-r", "@", "--tool", "jjdiff-split", "-m", "first edit only",
        ],
    );

    let selected = diff(root, "@-");
    assert!(selected.contains("+ONE"), "the selected hunk moved out: {selected}");
    assert!(!selected.contains("+FIVE"), "the unselected one did not: {selected}");

    let remainder = diff(root, "@");
    assert!(remainder.contains("+FIVE"), "the remainder keeps the rest: {remainder}");
    assert!(!remainder.contains("+ONE"), "and not the part that left: {remainder}");

    // A split rearranges history, it does not touch the tree. `plan.json` is in
    // the working copy too, so read the file rather than diffing.
    assert_eq!(
        std::fs::read_to_string(root.join("f.txt")).unwrap(),
        "ONE\ntwo\nthree\nfour\nFIVE\n"
    );
}

/// The same split against a file with CRLF endings. The plan carries no `\r` —
/// the diff it was built from was parsed with `str::lines`, which drops it —
/// while the directories jj hands the tool hold the file's bytes as they are,
/// so this is the case where the two only line up if the arithmetic normalises
/// them and writes the terminators back.
#[test]
fn a_crlf_file_keeps_its_terminators_through_a_split() {
    let _guard = JJ_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if !jj_available() {
        eprintln!("skipping: jj not installed");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    jj(root, &["git", "init", "--colocate", "."]);
    std::fs::write(root.join("f.txt"), "one\r\ntwo\r\nthree\r\nfour\r\nfive\r\n").unwrap();
    jj(root, &["commit", "-m", "base"]);
    std::fs::write(root.join("f.txt"), "ONE\r\ntwo\r\nthree\r\nfour\r\nFIVE\r\n").unwrap();
    jj(root, &["describe", "-m", "both edits"]);

    // Byte for byte the plan of the first test: no producer puts a `\r` in a
    // plan line, whatever the file's endings are.
    let plan = serde_json::json!({
        "files": [{
            "path": "f.txt",
            "oldPath": null,
            "select": "hunks",
            "hunks": [
                { "selected": true, "oldStart": 1, "oldLines": 2, "lines": [
                    { "kind": "removed", "text": "one" },
                    { "kind": "added", "text": "ONE" },
                    { "kind": "context", "text": "two" }
                ]},
                { "selected": false, "oldStart": 4, "oldLines": 2, "lines": [
                    { "kind": "context", "text": "four" },
                    { "kind": "removed", "text": "five" },
                    { "kind": "added", "text": "FIVE" }
                ]}
            ]
        }]
    });
    let plan_path = root.join("plan.json");
    std::fs::write(&plan_path, plan.to_string()).unwrap();

    let binary = env!("CARGO_BIN_EXE_jjdiff-app");
    let program = format!("merge-tools.jjdiff-split.program=\"{binary}\"");
    let edit_args = format!(
        "merge-tools.jjdiff-split.edit-args=[\"--apply-split-plan\",\"{}\",\"$left\",\"$right\"]",
        plan_path.display()
    );
    jj(
        root,
        &[
            "--config", &program,
            "--config", &edit_args,
            "split", "-r", "@", "--tool", "jjdiff-split", "-m", "first edit only",
        ],
    );

    // The `\r` in the diff's own output is the file's: a split that rewrote the
    // terminators would show every line as changed instead of one.
    let selected = diff(root, "@-");
    assert!(selected.contains("+ONE\r\n"), "the selected hunk moved out, CRLF intact: {selected:?}");
    assert!(!selected.contains("+FIVE"), "the unselected one did not: {selected:?}");
    assert!(!selected.contains("-two"), "and no line nobody touched moved with it: {selected:?}");

    let remainder = diff(root, "@");
    assert!(remainder.contains("+FIVE\r\n"), "the remainder keeps the rest: {remainder:?}");
    assert!(!remainder.contains("+ONE"), "and not the part that left: {remainder:?}");
}

/// The same plan, the same protocol, the other verb.
///
/// What this proves that the split test cannot is the assumption the whole
/// feature stands on: the two directories `jj squash -i` hands its editor are
/// the *source's own* diff, parent on the left and source on the right. The
/// frontend builds its plan from that diff as drawn on screen, so if jj laid
/// out anything else here — the destination's tree, or a combined one — the
/// ticked hunks would select the wrong lines and nothing in the unit tests
/// would notice.
#[test]
fn a_selected_hunk_moves_into_the_destination_and_the_rest_stays() {
    let _guard = JJ_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if !jj_available() {
        eprintln!("skipping: jj not installed");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    jj(root, &["git", "init", "--colocate", "."]);
    std::fs::write(root.join("f.txt"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
    jj(root, &["commit", "-m", "destination"]);
    std::fs::write(root.join("f.txt"), "ONE\ntwo\nthree\nfour\nFIVE\n").unwrap();
    jj(root, &["describe", "-m", "source"]);

    let plan = serde_json::json!({
        "files": [{
            "path": "f.txt",
            "oldPath": null,
            "select": "hunks",
            "hunks": [
                { "selected": true, "oldStart": 1, "oldLines": 2, "lines": [
                    { "kind": "removed", "text": "one" },
                    { "kind": "added", "text": "ONE" },
                    { "kind": "context", "text": "two" }
                ]},
                { "selected": false, "oldStart": 4, "oldLines": 2, "lines": [
                    { "kind": "context", "text": "four" },
                    { "kind": "removed", "text": "five" },
                    { "kind": "added", "text": "FIVE" }
                ]}
            ]
        }]
    });
    let plan_path = root.join("plan.json");
    std::fs::write(&plan_path, plan.to_string()).unwrap();

    // Exactly what `squash_hunks` builds, with the test binary in place of the
    // bundle's.
    let binary = env!("CARGO_BIN_EXE_jjdiff-app");
    let program = format!("merge-tools.jjdiff-squash.program=\"{binary}\"");
    let edit_args = format!(
        "merge-tools.jjdiff-squash.edit-args=[\"--apply-split-plan\",\"{}\",\"$left\",\"$right\"]",
        plan_path.display()
    );
    jj(
        root,
        &[
            "--config", &program,
            "--config", &edit_args,
            "squash", "--from", "@", "--into", "@-",
            "--use-destination-message", "--tool", "jjdiff-squash",
        ],
    );

    let destination = diff(root, "@-");
    assert!(destination.contains("+ONE"), "the selected hunk moved in: {destination}");
    assert!(!destination.contains("+FIVE"), "the unselected one did not: {destination}");

    let source = diff(root, "@");
    assert!(source.contains("+FIVE"), "the source keeps the rest: {source}");
    assert!(!source.contains("+ONE"), "and not the part that left: {source}");

    // `--use-destination-message` is what keeps this non-interactive: without
    // it jj combines the two descriptions through `$EDITOR`, which in a
    // GUI-spawned process hangs rather than failing.
    let description = String::from_utf8_lossy(
        &Command::new("jj")
            .args(["--ignore-working-copy", "log", "--no-graph", "-r", "@-", "-T", "description"])
            .current_dir(root)
            .output()
            .unwrap()
            .stdout,
    )
    .into_owned();
    assert_eq!(description.trim(), "destination");
}

/// A plan that no longer describes the change must take the split down with it.
/// This is the failure that matters: the alternative to aborting is patching
/// someone's commit at offsets that mean nothing.
#[test]
fn a_stale_plan_aborts_the_split_and_leaves_one_change() {
    let _guard = JJ_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if !jj_available() {
        eprintln!("skipping: jj not installed");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    jj(root, &["git", "init", "--colocate", "."]);
    std::fs::write(root.join("f.txt"), "one\ntwo\n").unwrap();
    jj(root, &["commit", "-m", "base"]);
    std::fs::write(root.join("f.txt"), "ONE\ntwo\n").unwrap();
    jj(root, &["describe", "-m", "an edit"]);
    let before = String::from_utf8_lossy(
        &Command::new("jj")
            .args(["--ignore-working-copy", "log", "--no-graph", "-r", "all()", "-T", "change_id"])
            .current_dir(root)
            .output()
            .unwrap()
            .stdout,
    )
    .into_owned();

    // Describes a file that does not look like this.
    let plan = serde_json::json!({
        "files": [{
            "path": "f.txt",
            "oldPath": null,
            "select": "hunks",
            "hunks": [
                { "selected": true, "oldStart": 1, "oldLines": 1, "lines": [
                    { "kind": "removed", "text": "something else entirely" },
                    { "kind": "added", "text": "ONE" }
                ]},
                { "selected": false, "oldStart": 2, "oldLines": 1, "lines": [
                    { "kind": "context", "text": "two" }
                ]}
            ]
        }]
    });
    let plan_path = root.join("plan.json");
    std::fs::write(&plan_path, plan.to_string()).unwrap();

    let binary = env!("CARGO_BIN_EXE_jjdiff-app");
    let output = Command::new("jj")
        .args(["--config", "signing.behavior=drop"])
        .args(["--config", &format!("merge-tools.jjdiff-split.program=\"{binary}\"")])
        .args([
            "--config",
            &format!(
                "merge-tools.jjdiff-split.edit-args=[\"--apply-split-plan\",\"{}\",\"$left\",\"$right\"]",
                plan_path.display()
            ),
        ])
        .args(["split", "-r", "@", "--tool", "jjdiff-split", "-m", "should not happen"])
        .current_dir(root)
        .env("JJ_USER", "Test")
        .env("JJ_EMAIL", "test@example.com")
        .output()
        .expect("jj runs");

    assert!(!output.status.success(), "jj should have failed: {output:?}");
    let after = String::from_utf8_lossy(
        &Command::new("jj")
            .args(["--ignore-working-copy", "log", "--no-graph", "-r", "all()", "-T", "change_id"])
            .current_dir(root)
            .output()
            .unwrap()
            .stdout,
    )
    .into_owned();
    assert_eq!(before, after, "no commit was created or rewritten");
}
