//! LLM-guided review walkthroughs.
//!
//! A walkthrough is an ordered set of steps; each step carries a narrative and references
//! hunk ids from the structured diff. Generation is delegated to an agent CLI — Claude Code
//! only for now, behind [`AgentBackend`] so other CLIs can slot in later.
//!
//! Walkthroughs are keyed by change id and stored with the diff fingerprint they were
//! generated against, so a change that evolves is detected as stale (the jj-native angle:
//! the walkthrough follows the change, not a commit hash).

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use jjdiff_diff::{FilePatch, LineKind};

const GENERATION_TIMEOUT: Duration = Duration::from_secs(300);
/// Diffs larger than this (in prompt characters) are refused rather than truncated silently.
const MAX_PROMPT_CHARS: usize = 400_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Walkthrough {
    pub summary: String,
    pub steps: Vec<Step>,
    /// Fingerprint of the diff this walkthrough was generated against (staleness check).
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub title: String,
    pub narrative: String,
    pub hunk_ids: Vec<String>,
}

/// What the agent must return (before we attach the fingerprint).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentWalkthrough {
    summary: String,
    steps: Vec<AgentStep>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentStep {
    title: String,
    narrative: String,
    hunk_ids: Vec<String>,
}

pub trait AgentBackend {
    /// Run one headless prompt, returning the agent's final text output.
    fn run(&self, prompt: &str) -> Result<String, String>;
    fn name(&self) -> &'static str;
}

/// Claude Code CLI: `claude -p --output-format json`, prompt on stdin.
pub struct ClaudeBackend {
    pub model: Option<String>,
}

impl AgentBackend for ClaudeBackend {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn run(&self, prompt: &str) -> Result<String, String> {
        let bin = std::env::var("JJDIFF_CLAUDE_PATH").unwrap_or_else(|_| "claude".into());
        let mut cmd = Command::new(&bin);
        cmd.args(["-p", "--output-format", "json"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(model) = self.model.as_deref().filter(|m| !m.is_empty()) {
            cmd.args(["--model", model]);
        }
        let mut child = cmd.spawn().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                format!("`{bin}` not found — install Claude Code or set JJDIFF_CLAUDE_PATH")
            } else {
                error.to_string()
            }
        })?;

        child
            .stdin
            .take()
            .ok_or("no stdin")?
            .write_all(prompt.as_bytes())
            .map_err(|e| e.to_string())?;

        // Poll with a deadline; a hung CLI must not wedge the app forever.
        let deadline = Instant::now() + GENERATION_TIMEOUT;
        loop {
            match child.try_wait().map_err(|e| e.to_string())? {
                Some(_) => break,
                None if Instant::now() > deadline => {
                    let _ = child.kill();
                    return Err(format!(
                        "walkthrough generation timed out after {}s",
                        GENERATION_TIMEOUT.as_secs()
                    ));
                }
                None => std::thread::sleep(Duration::from_millis(200)),
            }
        }
        let output = child.wait_with_output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "claude exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let raw = String::from_utf8_lossy(&output.stdout);
        // --output-format json wraps the reply: {"type":"result","result":"...", ...}
        let envelope: serde_json::Value =
            serde_json::from_str(raw.trim()).map_err(|e| format!("bad claude envelope: {e}"))?;
        envelope["result"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| format!("claude envelope has no result field: {raw}"))
    }
}

const GUIDE: &str = "\
You are generating a guided code-review walkthrough. Order the steps so a reviewer builds \
understanding incrementally: start with the core change or data-model shift, then the logic \
that uses it, then tests/config/mechanical fallout. Group related hunks into one step. Titles \
are short noun phrases; narratives are 1-4 sentences explaining what the hunks do and why they \
matter, written for a colleague seeing the diff for the first time. Every step must reference \
at least one hunk id, every hunk id should appear in exactly one step, and you must not invent \
ids that are not in the diff.";

/// Build the full generation prompt from a structured diff.
pub fn build_prompt(files: &[FilePatch], context: &str, extra: &str) -> Result<String, String> {
    let mut diff_text = String::new();
    for file in files {
        if file.hunks.is_empty() {
            continue;
        }
        diff_text.push_str(&format!("=== {} ({:?})\n", file.path, file.status));
        for hunk in &file.hunks {
            diff_text.push_str(&format!("--- hunk id: {}\n", hunk.id));
            for line in &hunk.lines {
                let sign = match line.kind {
                    LineKind::Added => '+',
                    LineKind::Removed => '-',
                    LineKind::Context => ' ',
                };
                diff_text.push(sign);
                diff_text.push_str(&line.text);
                diff_text.push('\n');
            }
        }
    }
    if diff_text.is_empty() {
        return Err("nothing to walk through — the diff has no reviewable hunks".into());
    }
    if diff_text.len() > MAX_PROMPT_CHARS {
        return Err(format!(
            "diff too large for a walkthrough ({} chars > {MAX_PROMPT_CHARS})",
            diff_text.len()
        ));
    }

    let extra_block = if extra.trim().is_empty() {
        String::new()
    } else {
        format!("\nAdditional instructions from the user:\n{extra}\n")
    };
    Ok(format!(
        "{GUIDE}\n{extra_block}\nContext: {context}\n\nRespond with ONLY a JSON object, no \
         markdown fences, matching exactly:\n{{\"summary\": \"one-paragraph overview\", \
         \"steps\": [{{\"title\": \"...\", \"narrative\": \"...\", \"hunkIds\": [\"path#0\"]}}]}}\n\
         \nThe diff:\n\n{diff_text}"
    ))
}

/// Parse and validate the agent's reply against the actual diff.
pub fn parse_response(reply: &str, files: &[FilePatch]) -> Result<Walkthrough, String> {
    let trimmed = reply.trim();
    // Tolerate fenced output despite instructions.
    let json = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|rest| rest.trim_end_matches("```"))
        .unwrap_or(trimmed)
        .trim();

    let parsed: AgentWalkthrough =
        serde_json::from_str(json).map_err(|e| format!("agent returned invalid JSON: {e}"))?;

    let known: std::collections::HashSet<&str> = files
        .iter()
        .flat_map(|file| file.hunks.iter().map(|hunk| hunk.id.as_str()))
        .collect();

    let steps: Vec<Step> = parsed
        .steps
        .into_iter()
        .map(|step| Step {
            title: step.title,
            narrative: step.narrative,
            // Hallucinated ids are dropped, not fatal — but a step must keep at least one.
            hunk_ids: step
                .hunk_ids
                .into_iter()
                .filter(|id| known.contains(id.as_str()))
                .collect(),
        })
        .filter(|step| !step.hunk_ids.is_empty())
        .collect();

    if steps.is_empty() {
        return Err("agent produced no steps referencing real hunks".into());
    }
    Ok(Walkthrough {
        summary: parsed.summary,
        steps,
        fingerprint: jjdiff_diff::diff_fingerprint(files),
    })
}

/// Full pipeline: prompt → agent → validated walkthrough.
pub fn generate(
    backend: &dyn AgentBackend,
    files: &[FilePatch],
    context: &str,
    extra: &str,
) -> Result<Walkthrough, String> {
    let prompt = build_prompt(files, context, extra)?;
    let reply = backend.run(&prompt)?;
    parse_response(&reply, files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jjdiff_diff::parse_git_patch;

    const PATCH: &str = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,2 @@\n-old\n+new\n ctx\n@@ -9,1 +9,2 @@\n+more\n ctx2\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,1 +1,1 @@\n-x\n+y\n";

    fn files() -> Vec<jjdiff_diff::FilePatch> {
        parse_git_patch(PATCH).unwrap()
    }

    #[test]
    fn hunk_ids_are_assigned_and_stable() {
        let files = files();
        assert_eq!(files[0].hunks[0].id, "a.rs#0");
        assert_eq!(files[0].hunks[1].id, "a.rs#1");
        assert_eq!(files[1].hunks[0].id, "b.rs#0");
    }

    #[test]
    fn prompt_contains_ids_and_content() {
        let prompt = build_prompt(&files(), "change xyz: fix things", "").unwrap();
        assert!(prompt.contains("hunk id: a.rs#0"));
        assert!(prompt.contains("hunk id: b.rs#0"));
        assert!(prompt.contains("+new"));
        assert!(prompt.contains("change xyz"));
        assert!(prompt.contains("ONLY a JSON object"));
    }

    #[test]
    fn empty_diff_is_an_error() {
        assert!(build_prompt(&[], "ctx", "").is_err());
    }

    #[test]
    fn parses_valid_response_and_drops_hallucinated_ids() {
        let reply = r#"{"summary":"Two renames.","steps":[
            {"title":"Rename","narrative":"...","hunkIds":["a.rs#0","made-up#9"]},
            {"title":"Ghost","narrative":"...","hunkIds":["nope#0"]},
            {"title":"Second","narrative":"...","hunkIds":["b.rs#0"]}
        ]}"#;
        let walkthrough = parse_response(reply, &files()).unwrap();
        assert_eq!(walkthrough.steps.len(), 2, "ghost-only step dropped");
        assert_eq!(walkthrough.steps[0].hunk_ids, vec!["a.rs#0"]);
        assert!(!walkthrough.fingerprint.is_empty());
    }

    #[test]
    fn tolerates_markdown_fences() {
        let reply = "```json\n{\"summary\":\"s\",\"steps\":[{\"title\":\"t\",\"narrative\":\"n\",\"hunkIds\":[\"a.rs#0\"]}]}\n```";
        assert!(parse_response(reply, &files()).is_ok());
    }

    #[test]
    fn garbage_response_is_an_error() {
        assert!(parse_response("I cannot do that.", &files()).is_err());
        assert!(parse_response(r#"{"summary":"s","steps":[]}"#, &files()).is_err());
    }

    /// Live test against the real Claude CLI — run explicitly:
    /// `cargo test -p jjdiff-app real_claude -- --ignored --nocapture`
    #[test]
    #[ignore = "spawns the real claude CLI (network, credits)"]
    fn real_claude_end_to_end() {
        let files = files();
        let backend = ClaudeBackend { model: None };
        let walkthrough =
            generate(&backend, &files, "test change: two tiny renames", "").unwrap();
        eprintln!("summary: {}", walkthrough.summary);
        for step in &walkthrough.steps {
            eprintln!("- {} → {:?}", step.title, step.hunk_ids);
        }
        assert!(!walkthrough.steps.is_empty());
    }

    #[test]
    fn fingerprint_tracks_content() {
        let a = jjdiff_diff::diff_fingerprint(&files());
        let b = jjdiff_diff::diff_fingerprint(&files());
        assert_eq!(a, b);
        let other = parse_git_patch(
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-old\n+different\n",
        )
        .unwrap();
        assert_ne!(a, jjdiff_diff::diff_fingerprint(&other));
    }
}
