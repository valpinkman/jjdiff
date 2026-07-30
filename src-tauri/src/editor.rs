//! Launching things outside the app: the user's editor, and URLs.
//!
//! The command comes from `[editor] command` in the config as a template with
//! `{file}`, `{line}` and `{repo}` placeholders. Templates are split on
//! whitespace *before* substitution, so a path containing spaces stays a single
//! argv entry and nothing a filename contains can inject an extra argument.
//! There is no shell involved.

use std::path::Path;
use std::process::{Command, Stdio};

/// Build argv from a template. Returns the program plus its arguments.
///
/// `line` defaults to 1 when the caller has no line in hand (the file tree),
/// so a `{file}:{line}` template never produces a trailing-colon path.
pub fn build_argv(
    template: &str,
    file: &Path,
    line: Option<u32>,
    repo: &Path,
) -> Result<Vec<String>, String> {
    let file = file.to_string_lossy();
    let repo = repo.to_string_lossy();
    let line = line.unwrap_or(1).to_string();

    // Split first, substitute second: a space inside {file} must not become an
    // argument boundary.
    let argv: Vec<String> = template
        .split_whitespace()
        .map(|token| {
            token
                .replace("{file}", &file)
                .replace("{line}", &line)
                .replace("{repo}", &repo)
        })
        .collect();

    if argv.is_empty() {
        return Err(
            "no editor configured — set `command` under `[editor]` in ~/.config/jjdiff/config.toml \
             (for example `command = \"zed {file}:{line}\"`)"
                .into(),
        );
    }
    Ok(argv)
}

/// Spawn the editor detached: no inherited stdio, and the child is reaped on a
/// throwaway thread so a long-lived GUI editor does not accumulate as a zombie.
pub fn spawn(argv: &[String]) -> Result<(), String> {
    let (program, args) = argv.split_first().expect("argv is non-empty");
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("cannot run `{program}`: {error}"))?;

    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

/// Open a URL in the system browser.
///
/// A WebView is not a browser: `target="_blank"` has no tab to open and simply
/// does nothing, so every outbound link has to go through the host OS. Same
/// class of gap as the missing JS dialogs (see `ui/src/prompt.ts`).
///
/// Only `http`/`https` are accepted. The URLs we open come from the forge CLI
/// rather than user input, but handing an arbitrary scheme to `open` is how a
/// `file://` or a registered-handler URL turns a link into code execution.
pub fn open_url(url: &str) -> Result<(), String> {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(format!("refusing to open a non-web URL: {url}"));
    }
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    spawn(&[program.to_string(), url.to_string()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn argv(template: &str, file: &str, line: Option<u32>) -> Vec<String> {
        build_argv(template, &PathBuf::from(file), line, &PathBuf::from("/repo")).unwrap()
    }

    #[test]
    fn substitutes_every_placeholder() {
        assert_eq!(
            argv("code -g {file}:{line}", "/repo/src/a.rs", Some(42)),
            vec!["code", "-g", "/repo/src/a.rs:42"]
        );
        assert_eq!(
            build_argv(
                "editor --cwd {repo} {file}",
                &PathBuf::from("/repo/a.rs"),
                None,
                &PathBuf::from("/repo")
            )
            .unwrap(),
            vec!["editor", "--cwd", "/repo", "/repo/a.rs"]
        );
    }

    #[test]
    fn missing_line_defaults_to_one() {
        // A `{file}:{line}` template must never yield a trailing colon.
        assert_eq!(argv("zed {file}:{line}", "/repo/a.rs", None), vec!["zed", "/repo/a.rs:1"]);
    }

    #[test]
    fn paths_with_spaces_stay_one_argument() {
        // Substituting after the split is the whole point: this must be 2 args,
        // not 3, and a filename can never inject a flag.
        assert_eq!(
            argv("zed {file}", "/repo/my notes/a b.md", None),
            vec!["zed", "/repo/my notes/a b.md"]
        );
        assert_eq!(
            argv("zed {file}", "/repo/x --wait -n /etc/passwd", None),
            vec!["zed", "/repo/x --wait -n /etc/passwd"]
        );
    }

    #[test]
    fn only_web_urls_are_opened() {
        // These would otherwise be handed straight to `open`, which happily
        // launches applications and registered URL handlers.
        for hostile in ["file:///etc/passwd", "javascript:alert(1)", "vscode://x", "  ftp://h/f"] {
            assert!(open_url(hostile).is_err(), "{hostile} must be refused");
        }
        assert!(open_url("").is_err());
    }

    #[test]
    fn empty_template_names_the_config_key() {
        let error = build_argv("", &PathBuf::from("/a"), None, &PathBuf::from("/repo")).unwrap_err();
        assert!(
            error.starts_with("no editor configured"),
            "ui/src/app.ts matches this exact prefix to offer the configure prompt \
             instead of showing a raw config error: {error}"
        );
        assert!(build_argv("   ", &PathBuf::from("/a"), None, &PathBuf::from("/repo")).is_err());
    }
}
