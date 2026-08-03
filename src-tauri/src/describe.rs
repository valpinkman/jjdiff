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

/// How much of each recent message the prompt carries as a style example.
///
/// The examples are there to convey *convention* — prefix style, mood,
/// capitalisation, whether bodies are written at all — and the opening of a
/// message carries all of that. What the rest of it carries is length, and the
/// prompt's next sentence is "match their style", so the model matches that
/// too: on this repository, whose messages run to several thousand characters,
/// the whole ones produced a 2861-token commit message and 36 seconds of
/// waiting for it. Excerpted to 600 the same prompt answers in 9 with 410, and
/// the convention still lands. A demonstration outweighs an instruction, and
/// the instruction here already said "one to three short paragraphs".
const MAX_EXAMPLE_CHARS: usize = 600;

/// One example message, cut to whole lines and marked where it was cut.
///
/// Line-wise so a truncated example is never a half-sentence, and marked so the
/// model reads a long message that has been shortened rather than a repository
/// whose messages stop mid-thought — the ellipsis is a style example too.
fn style_excerpt(message: &str) -> String {
    let message = message.trim();
    if message.len() <= MAX_EXAMPLE_CHARS {
        return message.to_string();
    }
    let mut kept = String::new();
    for line in message.lines() {
        if !kept.is_empty() && kept.len() + line.len() + 1 > MAX_EXAMPLE_CHARS {
            break;
        }
        if !kept.is_empty() {
            kept.push('\n');
        }
        kept.push_str(line);
    }
    // A first line longer than the budget: keep it, since the subject is the
    // single most useful thing here and half of one teaches nothing.
    if kept.is_empty() {
        kept.push_str(message.lines().next().unwrap_or_default());
    }
    format!("{}\n…", kept.trim_end())
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
            block.push_str(&format!("\n---\n{}\n", style_excerpt(message)));
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

    /// The examples teach length as surely as they teach style, so they are
    /// excerpted — pinned because the failure is invisible in the output and
    /// shows up only as a wait: whole examples on this repository's own history
    /// produced a 2861-token message in 36s where excerpts give 410 in 9.
    #[test]
    fn style_examples_are_excerpted_so_they_teach_convention_and_not_length() {
        // Short enough to stand as it is: no marker, nothing removed.
        let short = ":memo: (docs): Fix a typo\n\nIt was spelled wrong.";
        assert_eq!(style_excerpt(short), short);

        // A long one keeps its opening, is cut on a line boundary, and says so.
        let long = format!(
            ":sparkles: (forge): Open a pull request\n\n{}",
            "A paragraph of reasoning that runs on.\n".repeat(40)
        );
        let excerpt = style_excerpt(&long);
        assert!(excerpt.starts_with(":sparkles: (forge): Open a pull request"), "{excerpt}");
        assert!(excerpt.ends_with('…'), "a cut example says it was cut: {excerpt}");
        assert!(excerpt.len() < MAX_EXAMPLE_CHARS + 40, "still bounded: {}", excerpt.len());
        assert!(
            excerpt.lines().all(|line| long.contains(line) || line == "…"),
            "cut between lines, never inside one"
        );

        // A subject longer than the whole budget is kept anyway — half a subject
        // teaches nothing, and this is the one line always worth showing.
        let huge_subject = "x".repeat(MAX_EXAMPLE_CHARS * 2);
        assert!(style_excerpt(&huge_subject).starts_with(&huge_subject));

        // And the prompt actually uses it.
        let files = vec![FilePatch {
            path: "a.rs".into(),
            ..parse_git_patch("diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-a\n+b\n")
                .unwrap()
                .remove(0)
        }];
        let prompt = build_prompt(&files, std::slice::from_ref(&long), "").unwrap();
        assert!(prompt.contains('…'), "the excerpt reached the prompt");
        assert!(!prompt.contains(&long), "the whole message did not");
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
