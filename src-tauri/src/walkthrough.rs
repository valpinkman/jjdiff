//! LLM-guided review walkthroughs.
//!
//! A walkthrough is an ordered set of steps; each step carries a narrative and references
//! hunk ids from the structured diff. Generation is delegated to an agent CLI — claude,
//! codex, opencode or pi, selected by `[walkthrough] backend` — behind [`AgentBackend`],
//! which is also what `describe` drives to write commit messages.
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
    /// The overview step, as a markdown document: impacted systems (a mermaid
    /// diagram), changed boundaries with their contracts, mutable state and
    /// effects. `None` on walkthroughs written before overviews existed, and on
    /// any reply that omitted it — the UI falls back to [`Walkthrough::summary`].
    #[serde(default)]
    pub overview: Option<String>,
    pub steps: Vec<Step>,
    /// Fingerprint of the diff this walkthrough was generated against (staleness check).
    pub fingerprint: String,
    /// True when the agent was shown the diff's shape rather than its content,
    /// because the diff would not fit. The steps still order the review; the
    /// narratives are necessarily shallower, and the UI says so.
    #[serde(default)]
    pub outline: bool,
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
    #[serde(default)]
    overview: Option<String>,
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

/// Which agent CLI generates walkthroughs. All are driven headlessly with the prompt on
/// stdin; only the argv and the reply envelope differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Claude,
    Codex,
    OpenCode,
    Pi,
}

impl Backend {
    pub fn parse(name: &str) -> Backend {
        match name.trim().to_ascii_lowercase().as_str() {
            "codex" => Backend::Codex,
            "opencode" => Backend::OpenCode,
            "pi" => Backend::Pi,
            _ => Backend::Claude,
        }
    }

    /// Env var overriding binary discovery, and the default binary name.
    fn binary(self) -> (&'static str, &'static str) {
        match self {
            Backend::Claude => ("JJDIFF_CLAUDE_PATH", "claude"),
            Backend::Codex => ("JJDIFF_CODEX_PATH", "codex"),
            Backend::OpenCode => ("JJDIFF_OPENCODE_PATH", "opencode"),
            Backend::Pi => ("JJDIFF_PI_PATH", "pi"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Backend::Claude => "claude",
            Backend::Codex => "codex",
            Backend::OpenCode => "opencode",
            Backend::Pi => "pi",
        }
    }
}

/// One agent CLI invocation: headless flags + prompt on stdin + a reply envelope.
pub struct CliBackend {
    pub backend: Backend,
    pub model: Option<String>,
}

impl CliBackend {
    /// The CLI's argv. `pub(crate)` so `config`'s tests can assert the whole
    /// chain from a config file to the flags the agent is actually run with —
    /// each link was covered alone, and the join is where it would break.
    pub(crate) fn args(&self) -> Vec<String> {
        let model = self.model.as_deref().filter(|m| !m.is_empty());
        let mut args: Vec<String> = match self.backend {
            // Prompt arrives on stdin, so no positional message is passed.
            Backend::Claude => ["-p", "--output-format", "json"].iter().map(|s| s.to_string()).collect(),
            Backend::Codex => ["exec", "--json", "-"].iter().map(|s| s.to_string()).collect(),
            Backend::OpenCode => ["run", "--format", "json"].iter().map(|s| s.to_string()).collect(),
            Backend::Pi => ["--print", "--mode", "json"].iter().map(|s| s.to_string()).collect(),
        };
        if let Some(model) = model {
            match self.backend {
                Backend::Claude => args.extend(["--model".into(), model.into()]),
                Backend::Codex => args.extend(["--model".into(), model.into()]),
                Backend::OpenCode => args.extend(["--model".into(), model.into()]),
                Backend::Pi => args.extend(["--model".into(), model.into()]),
            }
        }
        args
    }

    /// Pull the assistant's final text out of each CLI's reply format.
    ///
    /// Claude wraps a single object (`{"result": "..."}`); Codex and OpenCode stream JSONL
    /// events, so the last event carrying text wins; Pi's `--mode json` is a single object
    /// whose text field name has varied across versions. Anything unrecognized falls back to
    /// raw stdout, which the schema validation downstream will reject if it is not usable.
    fn extract(&self, raw: &str) -> Result<String, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(format!("{} returned no output", self.backend.label()));
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(text) = first_text(&value) {
                return Ok(text);
            }
        }
        // JSONL event stream: scan backwards for the last event with usable text.
        let last = trimmed
            .lines()
            .rev()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .find_map(|value| first_text(&value));
        Ok(last.unwrap_or_else(|| trimmed.to_string()))
    }
}

/// Common text-bearing fields across the agent CLIs' JSON shapes.
fn first_text(value: &serde_json::Value) -> Option<String> {
    for key in ["result", "text", "content", "message", "response", "output"] {
        match &value[key] {
            serde_json::Value::String(text) if !text.trim().is_empty() => {
                return Some(text.clone())
            }
            // e.g. {"message": {"content": "..."}}
            nested @ serde_json::Value::Object(_) => {
                if let Some(text) = first_text(nested) {
                    return Some(text);
                }
            }
            _ => {}
        }
    }
    None
}

impl AgentBackend for CliBackend {
    fn name(&self) -> &'static str {
        self.backend.label()
    }

    fn run(&self, prompt: &str) -> Result<String, String> {
        let (env_var, default_bin) = self.backend.binary();
        let bin = std::env::var(env_var).unwrap_or_else(|_| default_bin.to_string());
        let mut cmd = Command::new(&bin);
        cmd.args(self.args())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                format!(
                    "`{bin}` not found — install the {} CLI or set {env_var}",
                    self.backend.label()
                )
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
                    // Name the tool, not the task: this backend also writes
                    // commit messages, and "walkthrough generation timed out"
                    // during a describe names something the user never asked for.
                    return Err(format!(
                        "{} timed out after {}s",
                        self.backend.label(),
                        GENERATION_TIMEOUT.as_secs()
                    ));
                }
                None => std::thread::sleep(Duration::from_millis(200)),
            }
        }
        let output = child.wait_with_output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "{} exited with {}: {}",
                self.backend.label(),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        self.extract(&String::from_utf8_lossy(&output.stdout))
    }
}

/// The whole authoring contract, shared with [`crate::cli::WALKTHROUGH_GUIDE`]
/// (which agents fetch with `jjdiff --walkthrough-guide`) and with
/// `skills/jjdiff/SKILL.md`. Keep the three in step: a walkthrough jjdiff
/// generates and one an agent hands it must be the same artefact.
///
/// Two parts, because the overview and the steps answer different questions.
/// The steps are a reading order through the diff. The overview is a *synthetic*
/// document — what systems the change touches, what contracts moved, what state
/// and effects appeared — which is the thing a reviewer needs before the first
/// hunk makes sense, and the thing a file-by-file summary never produces.
///
/// The overview's headings nest `#` title → `##` section → `###` boundary, and
/// that is the one level the three copies have to agree on:
/// `cli::tests::the_three_authoring_guides_state_one_overview_contract` reads
/// all three and is the only place the decision is made.
pub(crate) const GUIDE: &str = "\
You are generating a guided code-review walkthrough for a human reviewer. It has two parts: \
an overview document and an ordered set of steps through the diff.

# Part 1 — the overview

`overview` is a markdown document describing the change as a whole. It is a synthetic \
description, not a file-by-file summary of the diff, and it never prescribes an \
implementation. Write only what the diff supports.

Markers: ➕ addition · ✏️ modification · ➖ deletion.

Any section may have nothing to report. Write `None` under it; do not invent entries to \
fill it.

Open with a `#` heading naming the change, then one short paragraph stating its purpose, \
then the marker legend line, then these four sections in this order:

## Impacted Systems

A ```mermaid fence holding a `flowchart LR` of the concrete processes, services, binaries, \
crates, modules or applications that a changed boundary connects. Quote every node label \
and every edge label. Label an edge `existing` when it is unchanged and appears only as \
context. Keep it to the systems the change actually reaches.

## Changes to System Boundaries

One `###` section per changed boundary, headed `### <marker> <left system> ⇄ <right system> — <what crosses>`. \
A boundary is where two systems meet: an IPC or RPC surface, a CLI one process shells out \
to, a wire or file format, a database schema, a public API of one crate consumed by \
another, a protocol. Name concrete systems, not a module-plus-caller pair. Do not list an \
unchanged downstream boundary as changed merely because new routing now reaches it.

Under each boundary:

- **Routing** — bullets: which side handles what, and what a side does with a call it does \
  not handle.
- **Files:** one to three changed source files, each as inline code holding its \
  repository-relative path exactly as it appears in the diff, separated by ` · `. jjdiff \
  turns those paths into links into the diff itself, so a path it cannot match is a dead \
  link.
- **Contract changes** — a ```diff fence giving the relevant shapes and operations almost \
  in full: inputs, outputs, variants, optional fields, collections and errors. Prefix an \
  added declaration with `+` and a removed one with `-`; inside an otherwise unchanged \
  shape, prefix only the added or removed fields. Leave necessary unchanged context \
  unprefixed. Write them in the language of the code being changed — Rust items for Rust, \
  TypeScript types for TypeScript, the equivalent shape and operations for anything else.
- Any behaviour the shapes do not state and a reviewer would have to infer: error mapping, \
  what a failure leaves behind, what is deliberately not forwarded.

## Changes to Mutable State

A markdown table with the header `| State | Ownership, cardinality, lifecycle |`, one row \
per added, modified or deleted piece of held data. Put the major system on the first line \
of the right cell and the concrete owner — struct, closure, module variable, component, \
table, cache — on the second. Cardinality describes the data relationship itself, not the \
number of copies. Record held data only: not function bags, clients, handles or other \
resources that own no data, and not existing state merely because new code reads it.

## Changes to Effects

A markdown table with the header `| Effect | Ownership and failure handling |`, one row per \
changed entry point that makes the system touch the outside world: filesystem, persistence, \
network, OS, external process. A call across a boundary already listed above is not an \
effect. If a changed entry point reaches a pre-existing effect, record the entry point and \
name the existing downstream work rather than calling that work changed. Do not record \
ordinary query, synchronization or cache work — record its state instead. Same two-line \
ownership convention as above, and keep failure handling to the behaviour that changed.

# Part 2 — the steps

Order the steps so a reviewer builds understanding incrementally: start with the core change \
or data-model shift, then the logic that uses it, then tests/config/mechanical fallout. \
Group related hunks into one step. Titles are short noun phrases; narratives are 1-4 \
sentences explaining what the hunks do and why they matter, written for a colleague seeing \
the diff for the first time. Every step must reference at least one hunk id, every hunk id \
should appear in exactly one step, and you must not invent ids that are not in the diff. \
HARD CONSTRAINT: all hunks of the same file must be grouped into the same step — a file must \
never be split across steps (reviewers mark whole files as viewed, so a split file would \
show as already seen in a later step).

`summary` is one plain paragraph — no markdown — shown where the document does not fit.";

/// Extra instruction when the agent is given the diff's shape instead of its
/// content. Without it the narratives read as though the code had been examined.
const OUTLINE_GUIDE: &str = "\
IMPORTANT: this diff is too large to send in full, so what follows is its *outline* — every \
file and hunk with its position and header line, but not the code. Order and group the files \
into steps as usual, and title them from the paths and headers. Keep narratives to what the \
outline actually supports (what area a step covers and why it is read at that point); do not \
describe code you have not been shown, and do not guess at implementation detail. The same \
limit applies to the overview: keep Impacted Systems and the boundary list to what the paths \
and headers show, and omit the Contract changes fences entirely rather than inventing \
shapes you have not seen.";

/// The diff as the agent will read it: the whole thing when it fits, its shape
/// when it does not.
///
/// Refusing outright was the previous behaviour, and it is right that a diff is
/// never silently truncated — a walkthrough of half a change, presented as a
/// walkthrough of the change, is worse than none. But "no" is a poor answer to
/// a 1400-file proposal, which is exactly where being told what to read first
/// is worth most. So past the cap the content is dropped and the *structure* is
/// kept: every file and hunk, with counts and the `@@` header, which is what
/// ordering and grouping need. It is a different, shallower artefact, and it
/// says so — both to the agent, so the prose does not overreach, and in
/// [`Walkthrough::outline`], so the reader knows.
fn render_diff(files: &[FilePatch]) -> (String, bool) {
    let mut full = String::new();
    for file in files {
        if file.hunks.is_empty() {
            continue;
        }
        full.push_str(&format!("=== {} ({:?})\n", file.path, file.status));
        for hunk in &file.hunks {
            full.push_str(&format!("--- hunk id: {}\n", hunk.id));
            for line in &hunk.lines {
                let sign = match line.kind {
                    LineKind::Added => '+',
                    LineKind::Removed => '-',
                    LineKind::Context => ' ',
                };
                full.push(sign);
                full.push_str(&line.text);
                full.push('\n');
            }
        }
    }
    if full.len() <= MAX_PROMPT_CHARS {
        return (full, false);
    }

    let mut outline = String::new();
    for file in files {
        if file.hunks.is_empty() {
            continue;
        }
        outline.push_str(&format!(
            "=== {} ({:?})  +{} −{}\n",
            file.path, file.status, file.added, file.removed
        ));
        for hunk in &file.hunks {
            // The `@@` header plus jj's context label is the one line that says
            // *where* a hunk is without quoting any of it.
            outline.push_str(&format!(
                "  {}  @@ -{},{} +{},{} @@{}\n",
                hunk.id,
                hunk.old_start,
                hunk.old_lines,
                hunk.new_start,
                hunk.new_lines,
                if hunk.context.is_empty() { String::new() } else { format!(" {}", hunk.context) }
            ));
        }
    }
    (outline, true)
}

/// Build the full generation prompt from a structured diff.
pub fn build_prompt(files: &[FilePatch], context: &str, extra: &str) -> Result<(String, bool), String> {
    let (diff_text, outline) = render_diff(files);
    if diff_text.is_empty() {
        return Err("nothing to walk through — the diff has no reviewable hunks".into());
    }
    // An outline that still will not fit means a diff with more hunks than an
    // agent can order at all; there is nothing further to drop.
    if diff_text.len() > MAX_PROMPT_CHARS {
        return Err(format!(
            "diff too large for a walkthrough even as an outline ({} files, {} chars > \
             {MAX_PROMPT_CHARS}) — review it a change at a time",
            files.len(),
            diff_text.len()
        ));
    }

    let extra_block = if extra.trim().is_empty() {
        String::new()
    } else {
        format!("\nAdditional instructions from the user:\n{extra}\n")
    };
    let outline_block = if outline { format!("\n{OUTLINE_GUIDE}\n") } else { String::new() };
    let heading = if outline { "The diff outline:" } else { "The diff:" };
    Ok((
        format!(
            "{GUIDE}\n{outline_block}{extra_block}\nContext: {context}\n\nRespond with ONLY a JSON \
             object, no markdown fences around the object itself, matching exactly:\n\
             {{\"summary\": \"one plain paragraph\", \"overview\": \"# Title\\n\\n…the markdown \
             document, fences and all…\", \"steps\": [{{\"title\": \"...\", \"narrative\": \
             \"...\", \"hunkIds\": [\"path#0\"]}}]}}\n\n{heading}\n\n{diff_text}"
        ),
        outline,
    ))
}

/// Parse and validate the agent's reply against the actual diff.
pub fn parse_response(reply: &str, files: &[FilePatch], outline: bool) -> Result<Walkthrough, String> {
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

    let steps = enforce_file_exclusivity(steps);

    if steps.is_empty() {
        return Err("agent produced no steps referencing real hunks".into());
    }
    Ok(Walkthrough {
        summary: parsed.summary,
        // An empty string is the same absence as a missing key, and the UI has
        // one fallback rather than two.
        overview: parsed.overview.filter(|text| !text.trim().is_empty()),
        steps,
        fingerprint: jjdiff_diff::diff_fingerprint(files),
        outline,
    })
}

fn file_of(hunk_id: &str) -> &str {
    &hunk_id[..hunk_id.rfind('#').unwrap_or(hunk_id.len())]
}

/// A file must belong to exactly one step: viewed flags are per file, so a file split
/// across steps would show as "already seen" in the later step. The agent is instructed
/// not to split files, but this enforces it — every hunk moves to the first step that
/// mentions its file, duplicates collapse, and emptied steps drop out.
fn enforce_file_exclusivity(steps: Vec<Step>) -> Vec<Step> {
    let mut owner: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (index, step) in steps.iter().enumerate() {
        for id in &step.hunk_ids {
            owner.entry(file_of(id).to_string()).or_insert(index);
        }
    }
    let mut rehomed: Vec<Step> = steps
        .iter()
        .map(|step| Step {
            title: step.title.clone(),
            narrative: step.narrative.clone(),
            hunk_ids: Vec::new(),
        })
        .collect();
    for step in &steps {
        for id in &step.hunk_ids {
            let target = owner[file_of(id)];
            if !rehomed[target].hunk_ids.contains(id) {
                rehomed[target].hunk_ids.push(id.clone());
            }
        }
    }
    rehomed.retain(|step| !step.hunk_ids.is_empty());
    rehomed
}

/// Full pipeline: prompt → agent → validated walkthrough.
pub fn generate(
    backend: &dyn AgentBackend,
    files: &[FilePatch],
    context: &str,
    extra: &str,
) -> Result<Walkthrough, String> {
    let (prompt, outline) = build_prompt(files, context, extra)?;
    let reply = backend.run(&prompt)?;
    parse_response(&reply, files, outline)
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
        let (prompt, outline) = build_prompt(&files(), "change xyz: fix things", "").unwrap();
        assert!(!outline, "a small diff goes in whole");
        assert!(prompt.contains("hunk id: a.rs#0"));
        assert!(prompt.contains("hunk id: b.rs#0"));
        assert!(prompt.contains("+new"));
        assert!(prompt.contains("change xyz"));
        assert!(prompt.contains("ONLY a JSON object"));
    }

    /// What the overview document must contain is asserted once for all three
    /// copies of the guide, in
    /// `cli::tests::the_three_authoring_guides_state_one_overview_contract`.
    /// What is this test's own is the join: that the prompt carries the guide
    /// at all, and asks for the key the guide spends most of its length on.
    #[test]
    fn prompt_carries_the_guide_and_asks_for_the_overview() {
        let (prompt, _) = build_prompt(&files(), "ctx", "").unwrap();
        assert!(prompt.contains(GUIDE), "the prompt does not carry the authoring guide");
        assert!(prompt.contains("\"overview\""), "the JSON skeleton does not ask for an overview");
    }

    /// The overview rides alongside the steps, and its absence is not an error:
    /// walkthroughs stored before overviews existed still load.
    #[test]
    fn overview_is_captured_when_present_and_optional_when_not() {
        // `r##` because the document itself opens with `"# `, which closes an `r#` literal.
        let with = r##"{"summary":"s","overview":"# T\n\n```mermaid\nflowchart LR\n  a-->b\n```",
            "steps":[{"title":"t","narrative":"n","hunkIds":["a.rs#0"]}]}"##;
        let parsed = parse_response(with, &files(), false).unwrap();
        assert!(parsed.overview.unwrap().contains("flowchart LR"));

        let without = r#"{"summary":"s","steps":[{"title":"t","narrative":"n","hunkIds":["a.rs#0"]}]}"#;
        assert!(parse_response(without, &files(), false).unwrap().overview.is_none());

        // Blank is the same absence, so the UI has one fallback rather than two.
        let blank = r#"{"summary":"s","overview":"  \n ","steps":[{"title":"t","narrative":"n","hunkIds":["a.rs#0"]}]}"#;
        assert!(parse_response(blank, &files(), false).unwrap().overview.is_none());
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
        let walkthrough = parse_response(reply, &files(), false).unwrap();
        assert_eq!(walkthrough.steps.len(), 2, "ghost-only step dropped");
        assert_eq!(walkthrough.steps[0].hunk_ids, vec!["a.rs#0"]);
        assert!(!walkthrough.fingerprint.is_empty());
    }

    #[test]
    fn files_split_across_steps_are_consolidated() {
        // a.rs has hunks #0 and #1; the agent wrongly puts them in different steps.
        let reply = r#"{"summary":"s","steps":[
            {"title":"First","narrative":"...","hunkIds":["a.rs#0"]},
            {"title":"Second","narrative":"...","hunkIds":["b.rs#0","a.rs#1"]}
        ]}"#;
        let walkthrough = parse_response(reply, &files(), false).unwrap();
        assert_eq!(walkthrough.steps.len(), 2);
        // a.rs#1 moved into the step that owns a.rs; b.rs stays put.
        assert_eq!(walkthrough.steps[0].hunk_ids, vec!["a.rs#0", "a.rs#1"]);
        assert_eq!(walkthrough.steps[1].hunk_ids, vec!["b.rs#0"]);
    }

    #[test]
    fn step_emptied_by_consolidation_is_dropped() {
        let reply = r#"{"summary":"s","steps":[
            {"title":"Owns both","narrative":"...","hunkIds":["a.rs#0","b.rs#0"]},
            {"title":"Only strays","narrative":"...","hunkIds":["a.rs#1"]}
        ]}"#;
        let walkthrough = parse_response(reply, &files(), false).unwrap();
        assert_eq!(walkthrough.steps.len(), 1);
        assert_eq!(walkthrough.steps[0].hunk_ids, vec!["a.rs#0", "b.rs#0", "a.rs#1"]);
    }

    #[test]
    fn backend_parsing_defaults_to_claude() {
        assert_eq!(Backend::parse("codex"), Backend::Codex);
        assert_eq!(Backend::parse("OpenCode"), Backend::OpenCode);
        assert_eq!(Backend::parse(" pi "), Backend::Pi);
        assert_eq!(Backend::parse(""), Backend::Claude);
        assert_eq!(Backend::parse("nonsense"), Backend::Claude);
    }

    #[test]
    fn backend_args_carry_model_and_headless_flags() {
        let claude = CliBackend { backend: Backend::Claude, model: Some("sonnet".into()) };
        assert_eq!(claude.args(), vec!["-p", "--output-format", "json", "--model", "sonnet"]);
        // Empty model string is treated as "use the CLI default".
        let opencode = CliBackend { backend: Backend::OpenCode, model: Some(String::new()) };
        assert_eq!(opencode.args(), vec!["run", "--format", "json"]);
    }

    #[test]
    fn extracts_text_from_each_cli_envelope() {
        let claude = CliBackend { backend: Backend::Claude, model: None };
        assert_eq!(claude.extract(r#"{"type":"result","result":"hello"}"#).unwrap(), "hello");

        // JSONL event stream: the last text-bearing event wins.
        let codex = CliBackend { backend: Backend::Codex, model: None };
        let stream = "{\"type\":\"start\"}\n{\"text\":\"first\"}\n{\"text\":\"final\"}";
        assert_eq!(codex.extract(stream).unwrap(), "final");

        // Nested message objects.
        let pi = CliBackend { backend: Backend::Pi, model: None };
        assert_eq!(pi.extract(r#"{"message":{"content":"nested"}}"#).unwrap(), "nested");

        // Unrecognized output falls through raw for schema validation to judge.
        assert_eq!(pi.extract("plain text").unwrap(), "plain text");
        assert!(pi.extract("   ").is_err());
    }

    #[test]
    fn tolerates_markdown_fences() {
        let reply = "```json\n{\"summary\":\"s\",\"steps\":[{\"title\":\"t\",\"narrative\":\"n\",\"hunkIds\":[\"a.rs#0\"]}]}\n```";
        assert!(parse_response(reply, &files(), false).is_ok());
    }

    #[test]
    fn garbage_response_is_an_error() {
        assert!(parse_response("I cannot do that.", &files(), false).is_err());
        assert!(parse_response(r#"{"summary":"s","steps":[]}"#, &files(), false).is_err());
    }

    /// Live test against the real Claude CLI — run explicitly:
    /// `cargo test -p jjdiff-app real_claude -- --ignored --nocapture`
    #[test]
    #[ignore = "spawns the real claude CLI (network, credits)"]
    fn real_claude_end_to_end() {
        let files = files();
        let backend = CliBackend { backend: Backend::Claude, model: None };
        let walkthrough =
            generate(&backend, &files, "test change: two tiny renames", "").unwrap();
        eprintln!("summary: {}", walkthrough.summary);
        for step in &walkthrough.steps {
            eprintln!("- {} → {:?}", step.title, step.hunk_ids);
        }
        assert!(!walkthrough.steps.is_empty());
    }

    /// Past the cap the diff's *content* is dropped and its shape kept, rather
    /// than the whole thing being refused. The ids have to survive that — they
    /// are what the agent's steps reference and what the UI filters on.
    #[test]
    fn an_oversized_diff_becomes_an_outline_rather_than_an_error() {
        // One file, enough hunks of enough lines to blow the cap outright.
        let mut patch = String::from("diff --git a/big.rs b/big.rs\n--- a/big.rs\n+++ b/big.rs\n");
        for hunk in 0..400 {
            patch.push_str(&format!("@@ -{},2 +{},2 @@ fn area_{hunk}()\n", hunk * 10 + 1, hunk * 10 + 1));
            patch.push_str(&format!("-{}\n", "old ".repeat(200)));
            patch.push_str(&format!("+{}\n", "new ".repeat(200)));
        }
        let files = parse_git_patch(&patch).unwrap();

        let (prompt, outline) = build_prompt(&files, "big change", "").unwrap();
        assert!(outline, "should degrade rather than refuse");
        assert!(prompt.len() <= MAX_PROMPT_CHARS, "the outline itself must fit");

        // Structure kept: every hunk id, and where it is.
        assert!(prompt.contains("big.rs#0"));
        assert!(prompt.contains("big.rs#399"));
        assert!(prompt.contains("fn area_399()"), "the @@ context line locates a hunk");
        // Content dropped, and the agent told not to invent it.
        assert!(!prompt.contains(&"old ".repeat(200)));
        assert!(prompt.contains("do not describe code you have not been shown"));
        // …including in the overview, whose contract fences need the code.
        assert!(prompt.contains("omit the Contract changes fences"));

        // The flag rides on the artefact so the reader knows what they have.
        let reply = r#"{"summary":"s","steps":[{"title":"t","narrative":"n","hunkIds":["big.rs#0"]}]}"#;
        assert!(parse_response(reply, &files, outline).unwrap().outline);
        assert!(!parse_response(reply, &files, false).unwrap().outline);
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
