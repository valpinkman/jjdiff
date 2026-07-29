//! Conflict resolution: jjdiff as jj's merge tool.
//!
//! The third thing built on the seam `split.rs` opened, and the one that
//! finally removes "resolve it in a terminal" from the conflict banner.
//! `jj resolve` materializes a conflict's sides into files, runs the configured
//! merge tool, and takes whatever is at `$output` when it exits — so a tool
//! that already knows the answer has nothing to do but write it there.
//!
//! [`Repo::resolve_with_merge_tool`] registers this binary for one invocation
//! and jj re-enters it as `jjdiff --apply-resolution <file> $output`. The
//! resolved text was worked out in the UI, region by region, which is the part
//! a merge editor would otherwise need a terminal for.
//!
//! [`Repo::resolve_with_merge_tool`]: jjdiff_vcs::Repo::resolve_with_merge_tool
//!
//! The check on the way through is the one that matters: jj does not re-read
//! conflict markers in a merge tool's output unless told to, so text that still
//! has fences in it would be committed *as* the resolution — seven angle
//! brackets in the file and a conflict jj considers settled. A non-zero exit
//! makes jj leave the conflict alone instead.

use std::path::Path;

use jjdiff_diff::has_conflict_markers;

/// Copy a prepared resolution to where jj will read it.
///
/// Runs inside the process jj spawned as its merge tool; the exit status is
/// what jj reads, so an error here has to abort the resolution rather than
/// leave `$output` half written.
pub fn apply_resolution(resolution: &Path, output: &Path) -> Result<(), String> {
    let content = std::fs::read_to_string(resolution).map_err(|error| {
        format!("cannot read the resolution at {}: {error}", resolution.display())
    })?;
    if has_conflict_markers(&content) {
        return Err(
            "the resolution still contains conflict markers — jj would write them into the file and call the conflict resolved"
                .into(),
        );
    }
    std::fs::write(output, content)
        .map_err(|error| format!("cannot write the resolution to {}: {error}", output.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(resolution: &str) -> Result<(String, tempfile::TempDir), String> {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("resolution.txt");
        let output = tmp.path().join("output.txt");
        std::fs::write(&source, resolution).unwrap();
        std::fs::write(&output, "<<<<<<< Conflict 1 of 1\nleft\n>>>>>>> ends\n").unwrap();
        apply_resolution(&source, &output)?;
        Ok((std::fs::read_to_string(&output).unwrap(), tmp))
    }

    #[test]
    fn a_resolution_replaces_whatever_jj_left_at_the_output() {
        let (written, _tmp) = run("resolved\ncontent\n").unwrap();
        assert_eq!(written, "resolved\ncontent\n");
    }

    /// The load-bearing refusal. Without it jj takes the markers at face value
    /// and the "resolved" file keeps its fences.
    #[test]
    fn a_resolution_that_still_has_fences_is_refused() {
        let error = run("kept\n<<<<<<< Conflict 1 of 1\nboth\n>>>>>>> ends\n").unwrap_err();
        assert!(error.contains("conflict markers"), "{error}");
    }

    #[test]
    fn an_empty_resolution_is_a_legitimate_answer() {
        let (written, _tmp) = run("").unwrap();
        assert_eq!(written, "", "deleting the conflicted region entirely is a resolution");
    }
}
