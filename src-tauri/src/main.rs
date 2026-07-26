// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// `jjdiff --print-hunks [revset]` — dump the diff with stable hunk ids and exit, without
/// starting the GUI. Agents use this to author a walkthrough against real ids before
/// handing it back via `--walkthrough-file`.
fn print_hunks() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let revset = args
        .iter()
        .skip_while(|arg| *arg != "--print-hunks")
        .nth(1)
        .filter(|arg| !arg.starts_with('-'));

    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let repo = jjdiff_vcs::Repo::discover(&cwd).map_err(|e| e.to_string())?;
    repo.check_version().map_err(|e| e.to_string())?;

    let files = match revset {
        Some(revset) => {
            let patch = repo.patch_for(revset, false).map_err(|e| e.to_string())?;
            jjdiff_diff::parse_git_patch(&patch).map_err(|e| e.to_string())?
        }
        None => {
            let base = repo.working_copy_parent().map_err(|e| e.to_string())?;
            jjdiff_diff::worktree::diff_worktree(
                repo.root(),
                base.as_deref(),
                jjdiff_diff::worktree::WorktreeDiffOptions::default(),
            )
            .map_err(|e| e.to_string())?
        }
    };

    for file in &files {
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
    Ok(())
}

fn main() {
    if std::env::args().any(|arg| arg == "--print-hunks") {
        if let Err(error) = print_hunks() {
            eprintln!("jjdiff: {error}");
            std::process::exit(1);
        }
        return;
    }
    jjdiff_app::run();
}
