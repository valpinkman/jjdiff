//! Headless CLI: argv parsing, the commands that exit before any window is
//! created (`--help`, `--version`, `--walkthrough-guide`, `--diff`,
//! `--print-hunks`), and the "Install Terminal Helper" shim writer used by
//! both the headless path and the in-app command.
//!
//! Discipline (see PLAN.md, C1): the bundled binary *is* the CLI. We parse
//! argv at the very top of `run()`, before `tauri::Builder`, and anything
//! headless writes to stdout and `exit(0)`s without ever creating a window.
//! A bundled macOS binary still has a usable stdout when invoked from a
//! terminal, so this works without a Node shim.

use std::path::{Path, PathBuf};

use jjdiff_diff::FilePatch;
use jjdiff_vcs::Repo;

/// One parsed invocation. Mirrors the surface documented in PLAN.md:
/// One parsed invocation. Mirrors the surface documented in PLAN.md:
///
/// ```text
/// jjdiff [revset]              # open a repo (defaults to cwd)
/// jjdiff -R <path> [revset]    # explicit repo
/// jjdiff -w [revset]           # open and generate a walkthrough
/// jjdiff --walkthrough-file f # open an agent-authored walkthrough
/// jjdiff --walkthrough-guide  # print the authoring guide (headless)
/// jjdiff --diff [revset]      # structured diff as JSON (headless)
/// jjdiff --print-hunks [revset] # hunk dump for agents (headless)
/// jjdiff --install-terminal-helper   # write the PATH shim (headless)
/// jjdiff --help / --version
/// ```
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Args {
    pub repo_path: Option<PathBuf>,
    pub revset: Option<String>,
    pub walkthrough: bool,
    pub walkthrough_file: Option<PathBuf>,
    /// A headless command selected on argv. `None` means "launch the GUI".
    pub headless: Option<Headless>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Headless {
    Help,
    Version,
    WalkthroughGuide,
    /// `--diff [revset]` — JSON to stdout. `None` = working copy.
    Diff(Option<String>),
    /// `--print-hunks [revset]` — text dump for agents.
    PrintHunks(Option<String>),
    InstallTerminalHelper,
}

impl Args {
    /// Parse `std::env::args()` (without `argv[0]`). Unknown flags starting
    /// with `-` are ignored so `tauri dev`'s own injected flags do not break
    /// the GUI path; headless commands reject unknowns as errors instead.
    pub fn parse(argv: &[String]) -> Result<Args, ParseError> {
        Args::parse_iter(argv.iter().map(|s| s.as_str()))
    }

    fn parse_iter<'a, I: Iterator<Item = &'a str>>(args: I) -> Result<Args, ParseError> {
        let mut repo_path: Option<PathBuf> = None;
        let mut revset: Option<String> = None;
        let mut walkthrough = false;
        let mut walkthrough_file: Option<PathBuf> = None;
        let mut headless: Option<Headless> = None;

        // Helper: read the next value for a flag that takes one.
        let want_value = |args: &mut std::iter::Peekable<I>, flag: &str| -> Result<String, ParseError> {
            args.next()
                .map(|s| s.to_string())
                .ok_or_else(|| ParseError::MissingValue(flag.to_string()))
        };

        let mut peekable = args.peekable();
        while let Some(arg) = peekable.next() {
            match arg {
                "-h" | "--help" => headless = Some(Headless::Help),
                "-V" | "--version" => headless = Some(Headless::Version),
                "-R" | "--repo" => {
                    repo_path = Some(PathBuf::from(want_value(&mut peekable, arg)?));
                }
                "-w" | "--walkthrough" => walkthrough = true,
                "--walkthrough-file" => {
                    walkthrough_file = Some(PathBuf::from(want_value(&mut peekable, arg)?));
                }
                "--walkthrough-guide" => headless = Some(Headless::WalkthroughGuide),
                "--print-hunks" => {
                    let rev = Args::maybe_positional(&mut peekable);
                    headless = Some(Headless::PrintHunks(rev));
                }
                "--diff" => {
                    let rev = Args::maybe_positional(&mut peekable);
                    headless = Some(Headless::Diff(rev));
                }
                "--install-terminal-helper" => headless = Some(Headless::InstallTerminalHelper),
                // Tauri / `pnpm tauri dev` injects its own flags (e.g. `--no-watch`).
                // Ignore them on the GUI path so dev mode keeps working; the headless
                // commands above never see them because they short-circuit earlier.
                flag if flag.starts_with('-') => {
                    // If it's `--flag=value` form, the value is already consumed.
                    // If the next token doesn't start with `-`, assume it belongs to
                    // the unknown flag and skip it — matches the pre-C1 leniency.
                    if !flag.contains('=') {
                        if let Some(next) = peekable.peek() {
                            if !next.starts_with('-') {
                                peekable.next();
                            }
                        }
                    }
                }
                positional if revset.is_none() => revset = Some(positional.to_string()),
                positional => return Err(ParseError::UnexpectedPositional(positional.to_string())),
            }
        }

        Ok(Args { repo_path, revset, walkthrough, walkthrough_file, headless })
    }

    /// Consume an optional positional value following a headless flag
    /// (`--diff <revset>` / `--print-hunks <revset>`), without swallowing a
    /// following flag.
    fn maybe_positional<'a, I: Iterator<Item = &'a str>>(args: &mut std::iter::Peekable<I>) -> Option<String> {
        let is_value = args.peek().map(|next| !next.starts_with('-')).unwrap_or(false);
        if is_value { Some(args.next().unwrap().to_string()) } else { None }
    }

    /// Resolve the repo to act on: explicit `-R`, else the cwd.
    pub fn repo_or_cwd(&self) -> PathBuf {
        self.repo_path
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    MissingValue(String),
    UnexpectedPositional(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::MissingValue(flag) => write!(f, "{flag} expects a value"),
            ParseError::UnexpectedPositional(value) => {
                write!(f, "unexpected positional argument: {value}")
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// The literal help text. Keep it short and accurate to [`Args`].
pub const HELP: &str = "\
jjdiff — a fast, minimal diff viewer for Jujutsu colocated repos.

USAGE:
    jjdiff [revset]                Open a repo (defaults to cwd)
    jjdiff -R <path> [revset]      Open an explicit repo
    jjdiff -w [revset]             Open and generate a walkthrough
    jjdiff --walkthrough-file <f>  Open an agent-authored walkthrough
    jjdiff --walkthrough-guide     Print the walkthrough authoring guide
    jjdiff --diff [revset]         Print the structured diff as JSON
    jjdiff --print-hunks [revset]  Print the diff with stable hunk ids
    jjdiff --install-terminal-helper
                                   Install the `jjdiff` shim on PATH
    jjdiff --help                  Show this help
    jjdiff --version               Show the version

A revset of `@` (the working copy) is used when none is given. Headless
commands write to stdout and exit without opening a window.

Repository:  https://tangled.sh/valpinkman.tngl.sh/jjdiff
";

/// Version string: `jjdiff <version>` (workspace version injected at build).
pub fn version_line() -> String {
    format!("jjdiff {}", env!("CARGO_PKG_VERSION"))
}

/// The authoring guide agents fetch via `jjdiff --walkthrough-guide`. Mirrors
/// [`skills/jjdiff/SKILL.md`](../../skills/jjdiff/SKILL.md) so an agent that
/// does not have the skill installed can still author a walkthrough.
pub const WALKTHROUGH_GUIDE: &str = "\
# jjdiff walkthrough authoring guide

You are generating a guided code-review walkthrough. Order the steps so a \
reviewer builds understanding incrementally: start with the core change or \
data-model shift, then the logic that uses it, then tests/config/mechanical \
fallout. Group related hunks into one step. Titles are short noun phrases; \
narratives are 1-4 sentences explaining what the hunks do and why they matter, \
written for a colleague seeing the diff for the first time. Every step must \
reference at least one hunk id, every hunk id should appear in exactly one \
step, and you must not invent ids that are not in the diff. HARD CONSTRAINT: \
all hunks of the same file must be grouped into the same step — a file must \
never be split across steps (reviewers mark whole files as viewed, so a split \
file would show as already seen in a later step).

## Steps

1. Get the diff with stable hunk ids:

       jjdiff --print-hunks            # working copy
       jjdiff --print-hunks <revset>   # a specific change

   Each hunk is printed as `<path>#<index>` followed by its lines.

2. Write JSON to a temp file matching exactly:

       {
         \"summary\": \"one-paragraph overview of the change\",
         \"steps\": [
           { \"title\": \"short noun phrase\",
             \"narrative\": \"1-4 sentences\",
             \"hunkIds\": [\"src/a.rs#0\"] }
         ]
       }

   Rules jjdiff enforces on import — violating them silently loses content:
   - Every hunkIds entry must exist in the diff; invented ids are dropped.
   - All hunks of one file belong to one step (reviewers mark whole files
     viewed, so a file split across steps shows as already-seen in the later
     one).
   - Order steps so understanding builds: core change first, then its callers,
     then tests/config/mechanical fallout.

3. Open jjdiff on it:

       jjdiff --walkthrough-file /tmp/walkthrough.json [revset]

The walkthrough is stored against the change id, so it survives \
`jj describe`/`squash` and is flagged stale if the diff later moves.
";

/// Run a headless command. Returns `Ok(())` on success; the caller exits 0.
/// Errors are returned as a string for the caller to print to stderr and
/// exit 1.
pub fn run_headless(headless: &Headless, args: &Args) -> Result<(), String> {
    match headless {
        Headless::Help => {
            println!("{HELP}");
            Ok(())
        }
        Headless::Version => {
            println!("{}", version_line());
            Ok(())
        }
        Headless::WalkthroughGuide => {
            print!("{WALKTHROUGH_GUIDE}");
            Ok(())
        }
        Headless::InstallTerminalHelper => {
            let report = install_terminal_helper()?;
            println!("{report}");
            Ok(())
        }
        Headless::Diff(revset) => {
            let files = diff_files(args, revset.as_deref())?;
            let json = serde_json::to_string_pretty(&files)
                .map_err(|e| format!("failed to serialize diff: {e}"))?;
            println!("{json}");
            Ok(())
        }
        Headless::PrintHunks(revset) => {
            let files = diff_files(args, revset.as_deref())?;
            print_hunks(&files);
            Ok(())
        }
    }
}

/// Compute the diff for a headless command. `None` = live working copy
/// (fs-vs-`@-` via gix — no snapshot, no op written); otherwise parses
/// `jj diff --git` output for the revset.
fn diff_files(args: &Args, revset: Option<&str>) -> Result<Vec<FilePatch>, String> {
    let cwd = args.repo_or_cwd();
    let repo = Repo::discover(&cwd).map_err(|e| e.to_string())?;
    repo.check_version().map_err(|e| e.to_string())?;
    match revset {
        Some(revset) => {
            let patch = repo.patch_for(revset, false).map_err(|e| e.to_string())?;
            jjdiff_diff::parse_git_patch(&patch).map_err(|e| e.to_string())
        }
        None => {
            let base = repo.working_copy_parent().map_err(|e| e.to_string())?;
            jjdiff_diff::worktree::diff_worktree(
                repo.root(),
                base.as_deref(),
                jjdiff_diff::worktree::WorktreeDiffOptions::default(),
            )
            .map_err(|e| e.to_string())
        }
    }
}

fn print_hunks(files: &[FilePatch]) {
    for file in files {
        println!("=== {} ({:?})", file.path, file.status);
        for hunk in &file.hunks {
            println!("--- hunk id: {}", hunk.id);
            for line in &hunk.lines {
                let sign = match line.kind {
                    jjdiff_diff::LineKind::Added => '+',
                    jjdiff_diff::LineKind::Removed => '-',
                    jjdiff_diff::LineKind::Context => ' ',
                };
                println!("{sign}{}", line.text);
            }
        }
    }
}

/// Where the PATH shim points. Exposed for the in-app command and tests.
///
/// On macOS the bundle binary lives at `Contents/MacOS/jjdiff` inside the
/// `.app`; `std::env::current_exe()` resolves that even when invoked through
/// a symlink, so the shim always points at the real binary.
fn bundle_binary() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| format!("cannot resolve bundle binary: {e}"))
}

/// Candidate directories for the shim, in preference order. We never sudo —
/// if neither is writable, we hand back the one command the user should run.
///
/// `JJDIFF_BIN_DIR` overrides both candidates (used by tests; also handy for
/// users who want the shim somewhere unusual).
fn shim_dirs() -> Vec<PathBuf> {
    if let Some(dir) = std::env::var_os("JJDIFF_BIN_DIR") {
        return vec![PathBuf::from(dir)];
    }
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/bin"));
    }
    dirs.push(PathBuf::from("/usr/local/bin"));
    dirs
}

/// Write a two-line `exec` shim pointing at the bundle binary into the first
/// writable candidate dir. Returns a human-readable report (success or the
/// command the user should run manually).
pub fn install_terminal_helper() -> Result<String, String> {
    let binary = bundle_binary()?;
    let script = shim_script(&binary);

    for dir in shim_dirs() {
        if !dir.exists() {
            let _ = std::fs::create_dir_all(&dir);
        }
        if !dir_is_writable(&dir) {
            continue;
        }
        let shim = dir.join("jjdiff");
        // `write` truncates, so reinstalling updates the path rather than
        // silently pointing at a stale binary.
        std::fs::write(&shim, &script).map_err(|e| format!("write {}: {e}", shim.display()))?;
        set_executable(&shim);

        return Ok(format!(
            "Installed `jjdiff` on PATH at {}\nPointing at: {}",
            shim.display(),
            binary.display()
        ));
    }

    // None writable. Hand the user the one command to run instead.
    let dir = shim_dirs().first().cloned().unwrap_or_else(|| PathBuf::from("/usr/local/bin"));
    let shim = dir.join("jjdiff");
    let mkdir = if dir.exists() { String::new() } else { format!("mkdir -p {} && ", dir.display()) };
    let write = format!(
        "cat > {shim} <<'EOF'\n{script}EOF",
        shim = shim.display()
    );
    let chmod = format!("chmod +x {}", shim.display());
    Ok(format!(
        "Could not write the shim automatically (no writable directory on PATH).\n\
         Run this, then reopen a terminal:\n\n  {mkdir}{write} && {chmod}\n\n\
         Pointing at: {}",
        binary.display()
    ))
}

/// The shim itself: `exec` the bundle binary, forwarding every arg. `$0` keeps
/// the original argv intact without needing a wrapper that quotes args.
fn shim_script(binary: &Path) -> String {
    format!("#!/bin/sh\nexec {} \"$@\"\n", binary.display())
}

#[cfg(unix)]
fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(0o755);
        let _ = std::fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) {}

/// Whether we can actually create a file in `dir`. Probing mode bits is not
/// enough — a freshly `create_dir_all`'d directory has the write bit set
/// regardless of whether our process owns it.
fn dir_is_writable(dir: &Path) -> bool {
    if !dir.exists() {
        return false;
    }
    let probe = dir.join(".jjdiff_probe");
    match std::fs::OpenOptions::new().write(true).create_new(true).truncate(true).open(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests mutate `JJDIFF_BIN_DIR` (process-global), so they must not
    // run in parallel. The guard is held for the whole body.
    static ENV_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_GUARD.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    fn argv(s: &str) -> Vec<String> {
        s.split_whitespace().map(|w| w.to_string()).collect()
    }

    #[test]
    fn parses_bare_invocation_as_gui() {
        let args = Args::parse(&[]).unwrap();
        assert_eq!(args.headless, None);
        assert_eq!(args.revset, None);
        assert!(!args.walkthrough);
    }

    #[test]
    fn parses_positional_revset() {
        let args = Args::parse(&argv("main..@")).unwrap();
        assert_eq!(args.revset.as_deref(), Some("main..@"));
        assert_eq!(args.headless, None);
    }

    #[test]
    fn parses_repo_and_revset() {
        let args = Args::parse(&argv("-R /code/repo trunk()..@")).unwrap();
        assert_eq!(args.repo_path.as_deref(), Some(std::path::Path::new("/code/repo")));
        assert_eq!(args.revset.as_deref(), Some("trunk()..@"));
    }

    #[test]
    fn parses_walkthrough_flags() {
        let args = Args::parse(&argv("-w @-")).unwrap();
        assert!(args.walkthrough);
        assert_eq!(args.revset.as_deref(), Some("@-"));
    }

    #[test]
    fn parses_walkthrough_file() {
        let args = Args::parse(&argv("--walkthrough-file /tmp/w.json @")).unwrap();
        assert_eq!(args.walkthrough_file.as_deref(), Some(std::path::Path::new("/tmp/w.json")));
        assert_eq!(args.revset.as_deref(), Some("@"));
    }

    #[test]
    fn headless_help_short_circuits() {
        let args = Args::parse(&argv("--help")).unwrap();
        assert_eq!(args.headless, Some(Headless::Help));
    }

    #[test]
    fn headless_version_short_circuits() {
        let args = Args::parse(&argv("-V")).unwrap();
        assert_eq!(args.headless, Some(Headless::Version));
    }

    #[test]
    fn headless_diff_with_revset() {
        let args = Args::parse(&argv("--diff trunk()..@")).unwrap();
        assert_eq!(args.headless, Some(Headless::Diff(Some("trunk()..@".to_string()))));
    }

    #[test]
    fn headless_diff_without_revset() {
        let args = Args::parse(&argv("--diff")).unwrap();
        assert_eq!(args.headless, Some(Headless::Diff(None)));
    }

    #[test]
    fn headless_diff_does_not_swallow_following_flag() {
        // `--diff -R /x` must not consume `-R` as the revset.
        let args = Args::parse(&argv("--diff -R /code/repo")).unwrap();
        assert_eq!(args.headless, Some(Headless::Diff(None)));
        assert_eq!(args.repo_path.as_deref(), Some(std::path::Path::new("/code/repo")));
    }

    #[test]
    fn headless_print_hunks_keeps_working() {
        let args = Args::parse(&argv("--print-hunks @-")).unwrap();
        assert_eq!(args.headless, Some(Headless::PrintHunks(Some("@-".to_string()))));
    }

    #[test]
    fn headless_install_helper() {
        let args = Args::parse(&argv("--install-terminal-helper")).unwrap();
        assert_eq!(args.headless, Some(Headless::InstallTerminalHelper));
    }

    #[test]
    fn headless_walkthrough_guide() {
        let args = Args::parse(&argv("--walkthrough-guide")).unwrap();
        assert_eq!(args.headless, Some(Headless::WalkthroughGuide));
    }

    #[test]
    fn unknown_flag_with_value_is_swallowed_on_gui_path() {
        // Mirrors the pre-C1 leniency: `tauri dev` injects `--no-watch` etc.
        let args = Args::parse(&argv("--tauri-internal value @")).unwrap();
        assert_eq!(args.revset.as_deref(), Some("@"));
    }

    #[test]
    fn unknown_flag_equals_form_is_ignored() {
        let args = Args::parse(&argv("--tauri-flag=x @")).unwrap();
        assert_eq!(args.revset.as_deref(), Some("@"));
    }

    #[test]
    fn missing_value_for_repo_is_an_error() {
        let err = Args::parse(&argv("-R")).unwrap_err();
        assert!(matches!(err, ParseError::MissingValue(flag) if flag == "-R"));
    }

    #[test]
    fn missing_value_for_walkthrough_file_is_an_error() {
        let err = Args::parse(&argv("--walkthrough-file")).unwrap_err();
        assert!(matches!(err, ParseError::MissingValue(flag) if flag == "--walkthrough-file"));
    }

    #[test]
    fn second_positional_is_an_error() {
        // `jjdiff a b` is ambiguous; reject rather than silently drop.
        let err = Args::parse(&argv("a b")).unwrap_err();
        assert!(matches!(err, ParseError::UnexpectedPositional(s) if s == "b"));
    }

    #[test]
    fn help_text_lists_every_flag() {
        for flag in [
            "--walkthrough-file",
            "--walkthrough-guide",
            "--diff",
            "--print-hunks",
            "--install-terminal-helper",
            "--help",
            "--version",
        ] {
            assert!(HELP.contains(flag), "HELP is missing {flag}");
        }
    }

    #[test]
    fn version_line_uses_pkg_version() {
        assert_eq!(version_line(), format!("jjdiff {}", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn walkthrough_guide_mentions_hunk_ids_and_exclusivity() {
        assert!(WALKTHROUGH_GUIDE.contains("hunkIds"));
        assert!(WALKTHROUGH_GUIDE.contains("never be split across steps"));
    }

    #[test]
    fn shim_script_execs_binary_with_dollar_at() {
        let script = shim_script(Path::new("/Applications/jjdiff.app/Contents/MacOS/jjdiff"));
        assert!(script.starts_with("#!/bin/sh\n"));
        assert!(script.contains("exec /Applications/jjdiff.app/Contents/MacOS/jjdiff \"$@\""));
    }

    #[test]
    fn install_helper_writes_shim_and_reports_path() {
        let _guard = lock_env();
        let tmp = tempfile::tempdir().unwrap();
        let bin_dir = tmp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::env::set_var("JJDIFF_BIN_DIR", &bin_dir);
        let report = install_terminal_helper().unwrap();
        let shim = bin_dir.join("jjdiff");
        assert!(shim.exists(), "shim not written at {}", shim.display());
        let written = std::fs::read_to_string(&shim).unwrap();
        assert!(written.starts_with("#!/bin/sh\n"));
        assert!(written.contains("exec "));
        assert!(report.contains("Installed"));
        assert!(report.contains(shim.to_string_lossy().as_ref()));
        std::env::remove_var("JJDIFF_BIN_DIR");
    }

    #[test]
    fn install_helper_reinstalls_over_stale_shim() {
        let _guard = lock_env();
        let tmp = tempfile::tempdir().unwrap();
        let bin_dir = tmp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let shim = bin_dir.join("jjdiff");
        std::fs::write(&shim, "#!/bin/sh\necho stale\n").unwrap();

        std::env::set_var("JJDIFF_BIN_DIR", &bin_dir);
        install_terminal_helper().unwrap();
        std::env::remove_var("JJDIFF_BIN_DIR");

        let after = std::fs::read_to_string(&shim).unwrap();
        assert!(!after.contains("echo stale"), "{after}");
        assert!(after.contains("exec "), "{after}");
    }

    #[test]
    fn install_helper_gives_a_command_when_no_dir_writable() {
        let _guard = lock_env();
        // A read-only directory: exists (so `create_dir_all` is a no-op) but
        // rejects file creation, which `dir_is_writable`'s probe detects.
        let tmp = tempfile::tempdir().unwrap();
        let ro = tmp.path().join("readonly");
        std::fs::create_dir_all(&ro).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
        }
        // On non-unix we cannot easily make a dir read-only; skip the test.
        #[cfg(not(unix))]
        return;

        std::env::set_var("JJDIFF_BIN_DIR", &ro);
        let report = install_terminal_helper().unwrap();
        std::env::remove_var("JJDIFF_BIN_DIR");
        assert!(report.contains("Could not write the shim automatically"), "{report}");
        assert!(report.contains("cat > "), "{report}");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755));
        }
    }
}
