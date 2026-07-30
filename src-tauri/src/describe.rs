//! Commit messages written by the configured agent CLI.
//!
//! Shares [`crate::walkthrough`]'s backend plumbing — the same four CLIs, driven
//! headlessly the same way — but nothing else. A walkthrough orders a diff for
//! review; this writes the one paragraph that says what the change is.
//!
//! The prompt carries **recent descriptions from the repo it is writing for**,
//! because a commit message that ignores the house style is a message you
//! rewrite by hand. jjdiff's own history is prose sentences; the monorepo this
//! was built against uses `:gitmoji: (scope): summary`. Neither convention is
//! written down anywhere a model could look up, and both are obvious from five
//! examples.

use jjdiff_diff::{FilePatch, LineKind};

use crate::walkthrough::AgentBackend;

/// Diffs larger than this (in prompt characters) are summarised by their file
/// list instead. Unlike a walkthrough, a message does not need the code — it
/// needs to know what moved — so this degrades rather than refusing.
const MAX_DIFF_CHARS: usize = 120_000;

const GUIDE: &str = "\
You are writing the commit message for a change, to be read by someone who will \
encounter it in `git log` months from now with no other context.

Rules:

- The first line is a summary in the imperative or descriptive present, under \
about 72 characters, with no trailing period.
- Then a blank line, then a body of one to three short paragraphs saying **why** \
the change is the way it is: the problem it solves, the approach and what it \
rules out, anything a reader would otherwise have to reconstruct from the diff. \
Skip the body only for a change that is genuinely self-evident — a typo, a \
version bump.
- Do not enumerate the diff. A list of the files touched is already in the diff, \
and repeating it wastes the one place that can say what the diff cannot.
- Do not invent motivation you cannot see. If the reason is not visible in the \
change, describe what it does and stop.
- Wrap the body at roughly 80 columns.

Match the conventions of the recent messages you are shown — prefix style, \
capitalisation, mood, whether bodies are used at all. They are the repository's \
house style, and it beats any default you would otherwise reach for.";

/// The diff as the agent reads it: whole when it fits, a file list when not.
fn render(files: &[FilePatch]) -> String {
    let mut full = String::new();
    for file in files {
        full.push_str(&format!(
            "=== {} ({:?})  +{} -{}\n",
            file.path, file.status, file.added, file.removed
        ));
        for hunk in &file.hunks {
            for line in &hunk.lines {
                full.push(match line.kind {
                    LineKind::Added => '+',
                    LineKind::Removed => '-',
                    LineKind::Context => ' ',
                });
                full.push_str(&line.text);
                full.push('\n');
            }
        }
    }
    if full.len() <= MAX_DIFF_CHARS {
        return full;
    }

    let mut outline = String::from(
        "(too large to include in full — file list only, so keep the message to \
         what the paths and counts support)\n",
    );
    for file in files {
        outline.push_str(&format!(
            "{} ({:?})  +{} -{}\n",
            file.path, file.status, file.added, file.removed
        ));
    }
    outline
}

/// Build the prompt. `recent` is newest-first; empty is fine, it just means the
/// agent falls back on its own conventions.
pub fn build_prompt(files: &[FilePatch], recent: &[String], extra: &str) -> Result<String, String> {
    if files.is_empty() {
        return Err("nothing to describe — the working copy has no changes".into());
    }
    let examples = if recent.is_empty() {
        String::new()
    } else {
        let mut block = String::from(
            "\nRecent messages from this repository, newest first — match their style:\n",
        );
        for message in recent {
            block.push_str(&format!("\n---\n{}\n", message.trim()));
        }
        block.push_str("---\n");
        block
    };
    let extra_block = if extra.trim().is_empty() {
        String::new()
    } else {
        format!("\nAdditional instructions from the user:\n{extra}\n")
    };

    Ok(format!(
        "{GUIDE}\n{examples}{extra_block}\nRespond with ONLY a JSON object, no markdown fences, \
         matching exactly:\n{{\"message\": \"summary line\\n\\nbody\"}}\n\nThe diff:\n\n{}",
        render(files)
    ))
}

#[derive(serde::Deserialize)]
struct Reply {
    message: String,
}

/// Pull the message out of the agent's reply.
///
/// JSON first, because that is what was asked for. But a commit message is
/// plain prose and a model that answers with the prose itself has done the task
/// — refusing that would be pedantry, so unparseable output is taken verbatim
/// once it is clear it is not a JSON object. The fence strip is the same
/// tolerance the walkthrough parser has.
pub fn parse_response(reply: &str) -> Result<String, String> {
    let trimmed = reply.trim();
    if trimmed.is_empty() {
        return Err("the agent returned nothing".into());
    }
    let unfenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|rest| rest.trim_end_matches("```"))
        .unwrap_or(trimmed)
        .trim();

    let message = match serde_json::from_str::<Reply>(unfenced) {
        Ok(parsed) => parsed.message,
        Err(_) if unfenced.starts_with('{') => {
            return Err("the agent returned JSON that is not a message".into())
        }
        Err(_) => unfenced.to_string(),
    };
    let message = message.trim().to_string();
    if message.is_empty() {
        return Err("the agent returned an empty message".into());
    }
    Ok(message)
}

/// Full pipeline: prompt → agent → message.
pub fn generate(
    backend: &dyn AgentBackend,
    files: &[FilePatch],
    recent: &[String],
    extra: &str,
) -> Result<String, String> {
    let prompt = build_prompt(files, recent, extra)?;
    parse_response(&backend.run(&prompt)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jjdiff_diff::parse_git_patch;

    fn files() -> Vec<FilePatch> {
        parse_git_patch(
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,2 @@\n-old\n+new\n ctx\n",
        )
        .unwrap()
    }

    /// The examples are the whole reason this beats a generic prompt: neither
    /// gitmoji nor prose-sentence is written down anywhere a model could look
    /// up, and both are obvious from a handful of real messages.
    #[test]
    fn recent_messages_are_shown_as_the_house_style() {
        let recent = vec![
            ":sparkles: (auth): add login".to_string(),
            ":bug: (api): fix null pointer".to_string(),
        ];
        let prompt = build_prompt(&files(), &recent, "").unwrap();
        assert!(prompt.contains(":sparkles: (auth): add login"));
        assert!(prompt.contains(":bug: (api): fix null pointer"));
        assert!(prompt.contains("match their style"));
        assert!(prompt.contains("+new"), "the diff itself is still there");
    }

    #[test]
    fn a_repo_with_no_history_still_gets_a_prompt() {
        let prompt = build_prompt(&files(), &[], "").unwrap();
        assert!(!prompt.contains("newest first"));
        assert!(prompt.contains("ONLY a JSON object"));
    }

    #[test]
    fn nothing_to_describe_is_an_error() {
        assert!(build_prompt(&[], &[], "").is_err());
    }

    #[test]
    fn user_instructions_are_appended() {
        let prompt = build_prompt(&files(), &[], "always mention the ticket").unwrap();
        assert!(prompt.contains("always mention the ticket"));
    }

    /// Past the cap the code goes and the shape stays — a message needs to know
    /// what moved, not to read every line of it.
    #[test]
    fn an_oversized_diff_degrades_to_a_file_list() {
        let mut patch = String::from("diff --git a/big.rs b/big.rs\n--- a/big.rs\n+++ b/big.rs\n");
        for hunk in 0..300 {
            patch.push_str(&format!("@@ -{},1 +{},1 @@\n", hunk * 10 + 1, hunk * 10 + 1));
            patch.push_str(&format!("-{}\n", "old ".repeat(120)));
            patch.push_str(&format!("+{}\n", "new ".repeat(120)));
        }
        let files = parse_git_patch(&patch).unwrap();
        let prompt = build_prompt(&files, &[], "").unwrap();
        assert!(prompt.contains("file list only"));
        assert!(prompt.contains("big.rs"));
        assert!(!prompt.contains(&"old ".repeat(120)));
    }

    #[test]
    fn parses_the_json_envelope() {
        assert_eq!(
            parse_response(r#"{"message": "Fix the thing\n\nBecause it was broken."}"#).unwrap(),
            "Fix the thing\n\nBecause it was broken."
        );
        // Fenced despite the instruction.
        assert_eq!(
            parse_response("```json\n{\"message\": \"Fix it\"}\n```").unwrap(),
            "Fix it"
        );
    }

    /// A model that answers with the message itself has done the task. Refusing
    /// that because it is not JSON would be pedantry.
    #[test]
    fn plain_prose_is_taken_at_face_value() {
        assert_eq!(parse_response("Fix the thing\n\nBecause.").unwrap(), "Fix the thing\n\nBecause.");
    }

    #[test]
    fn empty_and_wrong_shaped_replies_are_errors() {
        assert!(parse_response("   ").is_err());
        assert!(parse_response(r#"{"summary": "wrong key"}"#).is_err());
        assert!(parse_response(r#"{"message": "  "}"#).is_err());
    }
}
