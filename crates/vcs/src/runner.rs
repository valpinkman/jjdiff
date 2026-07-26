use std::path::PathBuf;
use std::process::Command;

use crate::{Result, VcsError};

/// Flags applied to *every* jj invocation.
const COMMON_ARGS: &[&str] = &["--color=never", "--no-pager"];

/// Runs the `jj` CLI with jjdiff's read/mutate discipline.
#[derive(Clone)]
pub struct JjRunner {
    bin: PathBuf,
    cwd: PathBuf,
}

impl JjRunner {
    pub fn new(cwd: PathBuf) -> Self {
        let bin = std::env::var_os("JJDIFF_JJ_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("jj"));
        JjRunner { bin, cwd }
    }

    /// Read-only invocation: never snapshots, never takes the working-copy lock.
    pub fn read(&self, args: &[&str]) -> Result<String> {
        self.run(args, true)
    }

    /// Mutating invocation: lets jj snapshot the working copy as usual.
    pub fn mutate(&self, args: &[&str]) -> Result<String> {
        self.run(args, false)
    }

    /// Like [`Self::mutate`], but returns stderr on success — jj narrates what a mutation
    /// did (e.g. `absorb`'s per-target summary) on stderr.
    pub fn mutate_capturing_stderr(&self, args: &[&str]) -> Result<String> {
        let mut cmd = Command::new(&self.bin);
        cmd.current_dir(&self.cwd).args(COMMON_ARGS).args(args);
        let output = cmd.output().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                VcsError::JjNotFound { bin: self.bin.display().to_string() }
            } else {
                VcsError::Io(error)
            }
        })?;
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !output.status.success() {
            return Err(VcsError::CommandFailed {
                args: args.iter().map(|s| s.to_string()).collect(),
                stderr,
            });
        }
        Ok(stderr)
    }

    fn run(&self, args: &[&str], ignore_working_copy: bool) -> Result<String> {
        let mut cmd = Command::new(&self.bin);
        cmd.current_dir(&self.cwd).args(COMMON_ARGS);
        // `jj --version` rejects repo-level flags; everything else gets the discipline flag.
        if ignore_working_copy && args != ["--version"] {
            cmd.arg("--ignore-working-copy");
        }
        cmd.args(args);

        let output = cmd.output().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                VcsError::JjNotFound { bin: self.bin.display().to_string() }
            } else {
                VcsError::Io(error)
            }
        })?;

        if !output.status.success() {
            return Err(VcsError::CommandFailed {
                args: args.iter().map(|s| s.to_string()).collect(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        String::from_utf8(output.stdout)
            .map_err(|error| VcsError::Parse(format!("non-UTF-8 jj output: {error}")))
    }
}
