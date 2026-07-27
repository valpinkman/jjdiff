# jjdiff — plan

**jjdiff**: a fast, minimal desktop diff viewer for reviewing and landing changes in **Jujutsu
colocated repos**, built with **Tauri 2 + Rust + Lit**, jj-native from day one. Informed by a code-level analysis of [nkzw-tech/codiff](https://github.com/nkzw-tech/codiff)
(MIT, Electron): its expensive part is the diff-review UI (~38k LOC renderer); its VCS plumbing is
~3k LOC and cleanly separated behind an IPC surface. We copy the *shape*, not the code — and we
design the review model around jj instead of bending git's index model.

## Product theses

1. **No staging axis.** git tools revolve around staged/unstaged. jj has no index — the working
   copy *is* a commit (`@`). The core loop becomes: edit → review `@` → `describe` → `new`.
   Partial commits become `squash`/`split`, not `git add -p`.
2. **Review the stack, not just a diff.** jj users work in stacks of small changes. First-class
   UI: a stack sidebar (`trunk()..@`), per-change diffs, and "move this file/hunk into that
   change" (`jj squash --into`), plus one-click `jj absorb`.
3. **Change IDs are the identity.** Review state (viewed flags, comments) keys on stable change
   ids, not commit hashes — it survives `describe`, `squash`, rebases. This is impossible to do
   well in git and trivial in jj; it's the killer feature.
4. **Conflicts are data, not errors.** jj materializes conflicts inside commits. Render them as a
   structured three-pane/annotated view instead of raw markers.

## Architecture

```
┌─────────────────────────────────────────────┐
│ ui/            Lit 3 + Vite + TS            │
│   diff view (virtualized), stack sidebar,   │
│   file tree, command bar, comments          │
├──────────────── Tauri IPC ──────────────────┤
│ src-tauri/     app shell, windows, menu,    │
│                CLI entry (`jjdiff [revset]`)│
│ crates/vcs     jj facade: repo model,       │
│                revsets, statuses, mutations │
│ crates/diff    blob loading + diffing       │
│                (imara-diff, word-level)     │
│ crates/watch   op-log + fs watcher (notify) │
└─────────────────────────────────────────────┘
```

- **Tauri 2.x** — ~10 MB bundle / low RSS vs Electron's ~200 MB; native menus; sidecar-free
  because the backend *is* Rust.
- **Frontend: Lit 3 + Vite + TypeScript.** ~5 KB runtime, no VDOM — the frontend counterpart of
  choosing Tauri over Electron. The diff view is custom DOM-heavy work in any framework; Lit's
  fine-grained templating (`repeat()`, no reconciler) suits huge code listings. Virtualization
  via `@lit-labs/virtualizer` (variable-height, fits diff hunks); state via reactive
  controllers / `@lit-labs/signals`; syntax highlighting with **Shiki in a web worker** (lazy
  grammars, zero Rust-side grammar management). Revisit syntect/tree-sitter in Rust only if
  profiling demands it.
  - **Light-DOM rule:** the code/diff pane renders in light DOM
    (`createRenderRoot() { return this }`) so text selection, copy, and find work natively
    across lines; shadow DOM is reserved for chrome (sidebar, dialogs, command bar). Theming
    flows exclusively through CSS custom properties so it crosses both worlds.
- **Rust workspace** (`crates/`) so the vcs/diff/watch logic is testable without Tauri.

### jj integration strategy: CLI-first, jj-lib later

jj is itself Rust, so linking `jj-lib` is tempting, but its API is unstable and version-coupled
to the user's repo. Phase 1 is **CLI-first**:

- All reads run `jj … --ignore-working-copy --color never` — never triggers snapshots, never
  contends for the working-copy lock, never pollutes the op log.
- Structured output via `-T` templates wherever supported (`log`, `file show`, `op log`) —
  jj templates are a real query language; design one template module with escaping helpers.
- Mutations go through the CLI (`describe`, `new`, `squash`, `split`, `absorb`, `restore`) so
  semantics always match what the user's own `jj` does, across jj versions.
- **Blob fast path:** colocated repos have a real `.git`, so plain file contents are read by oid
  via **gix** (no process spawn, batch-friendly — replaces git's `cat-file --batch` trick).
  ⚠ Caveat: jj stores *conflicted* trees using special `.jjconflict-*` entries in the git
  backend; gix tree-walking must detect these and fall back to `jj file show` / jj's resolver.
- Phase 2 (optional, perf-driven): adopt `jj-lib` for hot read paths (tree diffs, evolog,
  conflict model) behind the same `crates/vcs` trait, pinned per release.

### Command / capability mapping

| Need | Source |
|---|---|
| repo root, is-jj-repo | `jj root` |
| working-copy live diff | fs contents vs `@-` tree (gix + crates/diff) — **not** jj snapshot, so the view is live without writing ops |
| diff of a revset / range | `jj diff --git -r X` for statuses/renames, blobs via gix, diff in Rust |
| file statuses | `jj diff --summary` (fallback) → prefer `-T` templates / tree diff |
| history / stack | `jj log --no-graph -T <json-ish template> -r 'trunk()..@'` |
| merge base | revset `fork_point(A \| B)` |
| identity | `jj config get user.name/email` |
| commit flow | `describe -m` / `new`; partial: `squash --into X -- paths`, `split` |
| auto-distribute hunks | `jj absorb` |
| rewrite tracking (review-state migration) | `jj evolog -T` |
| change detection | watch `.jj/repo/op_heads/heads/` + debounced fs events (notify crate) |

Hunk-level (not file-level) squash/split is the one thing the CLI can't do non-interactively:
Phase 1 ships file-level; Phase 2 does hunk-level via a scripted `JJ_EDITOR`/diff-editor shim or

jj-lib tree edits.

### Review-state model

- SQLite (rusqlite) in app data dir, keyed `(repo_id, change_id)`: viewed flags, draft comments,
  walkthrough progress. On op-log change, reconcile via `evolog` so state follows rewrites;
  orphan state on abandoned changes is garbage-collected.
- Config: `~/.config/jjdiff/config.toml` (+ JSON schema for the UI editor), watched live.

### IPC surface (Tauri commands — mirrors codiff's proven verbs)

`get_repo_state(source)`, `get_stack()`, `get_diff(change_id|range, path?)`,
`get_file_contents(rev, path)`, `describe(change_id, msg)`, `new_change()`,
`squash(paths, from, into)`, `absorb()`, `set_viewed(change_id, path, bool)`,
`get_conflicts(change_id)` — plus a `repo-changed` event stream from the watcher.

## Milestones

**M0 — skeleton. ✅ DONE.** Tauri scaffold, Rust workspace, `jj` CLI wrapper with
`--ignore-working-copy` discipline, op-head watcher, `jjdiff [revset]` CLI entry.

**M1 — read-only reviewer. ✅ DONE.** Working-copy (fs-vs-`@-` via gix, zero snapshots —
verified by op-log test) + arbitrary-revset diffs; virtualized split/unified views, word-level
intra-line spans (UTF-16 offsets), whitespace toggle; file tree; shiki/core highlighting in a
worker (JS regex engine, only mapped grammars bundled); binary/too-large guards.

**M2 — jj-native actions. ✅ DONE.** Describe editor + Commit & New (`describe` + `new`),
file-level `squash` to parent, viewed flags keyed by change id (collapse + tree dimming),
Cmd/Ctrl+Shift+P command bar, `~/.config/jjdiff/config.toml`, working-copy fs watcher
(gitignore-aware) so edits appear without a manual refresh.

**M3 — stack review. ✅ DONE.** Stack sidebar over `trunk()..@`, per-change review with
Mark Reviewed (stores the reviewed commit id per change id), "what changed since I last
reviewed" interdiff via `jj interdiff` between evolog commits, Absorb button (summary
surfaced), per-file move-to-change over mutable stack changes.
Note: evolog's template context differs from log's (`json(commit)`, not `json(self)`).

**M4 — conflicts + polish. ✅ DONE.** Conflict surfacing (banner, per-file badges,
highlighted jj conflict-marker lines in diffs, `resolve --list` parsing), forced
light/dark/system theme, configurable command-bar keymap, repo name in the window title,
packaging config (app/dmg/deb/rpm targets, generated icon set).
Deliberate cut: no in-app `jj resolve` launcher — interactive merge editors spawned from a
GUI without a TTY hang more often than they work; the conflict banner points at the
terminal instead.

**M5 — guided walkthroughs (Claude backend). ✅ DONE.** `jjdiff -w` and in-app generation:
structured diff (stable hunk ids) → prompt with guide + JSON contract → Claude Code CLI
headless (`claude -p --output-format json`, stdin prompt, timeout, hallucinated-id
filtering) → stored per **change id** with a diff fingerprint, so an evolved change flags
the walkthrough stale and offers regeneration. Guided mode: overview + steps, each step
filters the diff to its hunks; ←/→ navigation. Backend sits behind an `AgentBackend` trait —
Claude only for now, more CLIs later. Verified end-to-end against the real CLI
(`cargo test -p jjdiff-app real_claude -- --ignored`).

**M6 — daily-driver polish. ✅ DONE.** Keyboard-first review (`j`/`k` files, `n`/`p`
hunks, `v` viewed) with a virtualizer-driven cursor; find-in-diffs (`Mod+F`, live match
count, wrap-around); tree-click scrolls instead of filtering plus a sticky file
breadcrumb; expand-context (`file_content` command; expanders derived from hunk gaps so
they appear before the file is fetched); word-wrap toggle; `jjdiff <revset>` selects a
change on launch.

**M7 — agent + robustness. ✅ DONE.** All four agent CLIs behind one `CliBackend`
(Claude/Codex/OpenCode/Pi — one spawn path, per-backend argv, envelope extraction covering
single-object, JSONL-stream and nested-message shapes), selected via `[walkthrough]
backend`. Agent-authored walkthroughs: `jjdiff --print-hunks` dumps the diff with stable
ids headlessly, `--walkthrough-file` imports the agent's JSON through the same validation
as a generated one, and `skills/jjdiff/SKILL.md` documents the loop. `check_version()`
gates on jj ≥ 0.33. Worktree rename detection (exact-content only) and nested `.gitignore`
support in the fs watcher close the two known correctness gaps.

## Phase 2 — jj as a whole tool ✅ SHIPPED

M0–M7 made jjdiff a *reviewer*. Phase 2 makes it a *client*: acting on the repo, not just
reading it. Ordering is deliberate — the safety net lands before the sharp tools.

### B1 — Bug: description hidden on non-stack changes ✅ FIXED

Selecting an older change shows an empty description box. `select()` looks the change up in
`repo.stack` only ([app.ts:404](ui/src/app.ts:404)), but the stack is `trunk()..@ | @`, so
anything below trunk resolves to `undefined` and seeds `''`. It also sets `seededFor`, which
suppresses the correcting re-seed in `refresh()`. Fix: resolve through `selectedChange`
(which already searches the graph) and render immutable descriptions read-only rather than
as an editable box that silently discards edits.

### M8 — Change detail view ✅

Clicking a change currently jumps to Files, which throws away the change's identity. Replace
that with a **detail pane** as the default view for any non-working-copy change:

- Identity: change id, commit id, author, timestamps, bookmarks, immutable/conflict/empty
  state, and the full multi-line description (read-only when immutable, editable when not).
- Changed-file list with +/− counts, click to jump into the diff.
- Parents/children as navigable links — walking the graph without the sidebar.
- Actions relevant to *that* change (see M10), disabled with a reason when not applicable.
- The working copy keeps today's edit-first layout; the detail view is for everything else.

### M9 — Operation log + undo ✅

jj's superpower is that every operation is reversible. Shipping this **before** the mutation
surface means every new command in M10 arrives with a safety net.

- **Op log tab**: `jj op log -T 'json(self)'` returns id, parents, start/end time,
  description, and the literal `args` — a structured log with no parsing at all.
- **Undo**: `jj undo` for the last operation; `jj op restore <id>` to jump back to any point;
  `jj op revert <id>` to reverse a specific one mid-history.
- **Undo affordance on every mutation**: each command reports jj's own summary plus an Undo
  action, so experimenting is cheap. This is the feature that makes the rest safe.
- **`jj op diff`** between two operations for "what did that actually do".

### M10 — The jj command surface ✅

One `mutate()` helper in `crates/vcs`: run, capture stdout+stderr, return the resulting
operation id for undo. Every command below routes through it, appears in the command bar with
a keybinding, and is disabled with an explanation when it cannot apply (immutable target,
no remote, nothing to squash).

| Group | Commands | Notes |
|---|---|---|
| Navigate | `new [rev]`, `edit <rev>` | "Work on this change" from the detail view |
| Shape | `squash` (whole + per file ✅), `absorb` ✅, `split <paths>`, `duplicate`, `abandon` | Split is **file-level, non-interactive** (`jj split <paths>`), matching our squash approach; interactive hunk-splitting stays deferred |
| History | `rebase -r/-s/-b -d <dest>`, `backout` | Destination picker over the graph; drag-to-rebase deliberately not first — misdrops are expensive, and every rebase is undoable but not free |
| Describe | `describe` ✅, bulk-describe empty changes | |
| Remote | `git fetch`, `git push -b/--change`, bookmark create/set/delete/track, **open pull request** | Push needs bookmarks; `--change` auto-names from the change id. Show ahead/behind per bookmark. See PR note below |
| Files | `restore <paths>` | Discard changes — the one genuinely destructive op; confirm, and it is undoable |

**Safety model** (applies to all of the above): immutable targets are blocked client-side
*and* by jj itself; destructive commands (`abandon`, `restore`, `op abandon`) confirm first;
everything reports what it did and offers Undo. Long operations (fetch/push) run async with
progress, since they already block on the network.

### M11 — Revset filtering ✅

The graph is hardwired to `ancestors(@ | bookmarks())` capped at 60. Replace with a filter
control:

- A preset row for the revsets people actually use: **Stack** (`trunk()..@`), **All**
  (`ancestors(@ | bookmarks())`), **Mine** (`author(exact:"<me>")`), **Recent**
  (`ancestors(@, 50)`), **Conflicts** (`conflicts()`), **Bookmarks** (`bookmarks()`),
  **Working copies** (`@ | @-`).
- A free-form revset input with jj's own error surfaced inline on a bad expression (jj
  validates far better than we could).
- Persisted per repo, and honoured by `jjdiff <revset>` on launch.

#### Pull requests without a forge integration

Every forge prints a ready-made "create a PR" URL on the push that creates a branch —
tangled (`…/pulls/new?sourceBranch=…&targetBranch=main`), GitHub
(`…/pull/new/<branch>`), GitLab (`…/merge_requests/new?…`). We already capture push
stderr for the mutation summary, so the plan is to **scrape that URL and offer an "Open
pull request" action** rather than integrate any forge API.

That buys tangled, GitHub, GitLab and Gitea support in one small feature with no auth, no
tokens, and nothing to keep current as APIs drift. A real API integration (PR title/body
from the change description, review state in-app) stays deferred until there is a reason to
pick one forge — and tangled would now be the natural first, not GitHub.

Fetch pairs with this: `jj git fetch` plus per-bookmark ahead/behind is what tells you a PR
is out of date before you push over it.

### Still open after Phase 2

- **Interactive (hunk-level) split** — file-level `jj split <paths>` shipped; hunk-level
  still needs a scripted diff-editor shim.
- **Ahead/behind per bookmark** — `fetch`/`push` shipped, but the remote-tracking status
  display did not; wants `jj log -T 'remote_bookmarks'` plumbing.
- **Rebase destination picker** — currently a prompt for a revset; a graph-target picker
  (and eventually drag-to-rebase) is the better UI.
- **Conflict resolution** — still terminal-only, for the reason given in M4.
- **`jj op diff`** between two operations — the log lists operations but does not yet diff
  a pair.

### Proactive additions not in the original ask

- **Undo everywhere** (M9) — the single highest-value item here, and the reason to do the op
  log before the commands.
- **Bookmark management** (M10) — not optional: `jj git push` has nothing to push without it.
- **Fetch + ahead/behind** — a review tool that cannot see the remote's state is half-blind.
- **PR creation by URL scraping** — forge-agnostic, no API integration (see above).
- **Evolog drawer** — we already fetch evolog for interdiffs; exposing "this change has 6
  versions, diff any two" is nearly free and has no git equivalent.
- **Conflict resolution** — still deferred (GUI-spawned merge editors without a TTY hang more
  than they work), but M10 should at least offer `jj resolve --list` navigation and mark
  which side is which.
- **Command discoverability** — every command in the command bar with a keybinding, so the
  UI does not become a hunt for buttons.

### Phase 3 — collaboration and reach (planned)

Phase 1 made jjdiff a reviewer; Phase 2 made it a jj client. What remains is everything
that involves *other people* — comments, forges, sharing — plus the terminal entry point
that makes it reachable at all. Ordered by value per unit of work.

### C1 — CLI and terminal helper ✅ DONE

**We have a CLI.** `Args::parse` (in `src-tauri/src/cli.rs`) runs at the very top of
`main()`, before `tauri::Builder`; `--help`, `--version`, `--walkthrough-guide`,
`--diff`, `--print-hunks` and `--install-terminal-helper` write to stdout and `exit(0)`
without ever creating a window. A bundled macOS binary still has a usable stdout when
invoked from a terminal, so this works without a Node shim.

**We have no CLI.** `LaunchOptions::from_env` parses `-R`, `-w`, `--walkthrough-file` and a
positional revset, but only inside the app binary: there is no `jjdiff` on `PATH`, no
`--help`, and no way to launch from a shell without knowing the bundle path. Codiff solves
this with `bin/codiff.js` plus an "Install Terminal Helper" menu item.

Tauri makes this different from codiff's Electron approach — there is no Node shim, the
app binary *is* the CLI:

1. **Headless-before-GUI.** Parse argv at the very top of `run()`, before
   `tauri::Builder`. `--help`, `--version`, `--walkthrough-guide` and `--print-diff` write
   to stdout and `exit(0)` without ever creating a window. A bundled macOS binary still
   has a usable stdout when invoked from a terminal, so this works.
2. **A shim on PATH.** `Install Terminal Helper` (menu + command bar) writes a two-line
   `exec` script pointing at the bundle binary. Prefer `~/.local/bin`, fall back to
   `/usr/local/bin`, and *never* silently sudo — if neither is writable, print the one
   command the user should run.
3. **Single instance with arg forwarding** (`tauri-plugin-single-instance`): running
   `jjdiff` in a second repo opens a second window in the existing process rather than a
   rival process fighting over the same review store.

Command surface, mirroring what the app already does:

```
jjdiff [revset]              # open a repo (defaults to cwd)
jjdiff -R <path> [revset]    # explicit repo
jjdiff -w [revset]           # open and generate a walkthrough
jjdiff --walkthrough-file f  # open an agent-authored walkthrough
jjdiff --walkthrough-guide   # print the authoring guide (headless; for agents)
jjdiff --diff [revset]       # print the structured diff as JSON (headless; scripting)
jjdiff --help / --version
```

The `--walkthrough-guide` + `--walkthrough-file` pair closes the agent loop we already
built: a skill can ask jjdiff for its guide, author a walkthrough, and open jjdiff on it —
codiff's `$codiff` pattern, which currently has no entry point on our side.

### C2 — Inline review comments ✅ DONE

The single biggest gap, and the place where jj lets us beat codiff rather than match it.

- **Anchor on change ids, not commits.** A comment records `(change id, path, hunk id,
  side, line)`. Because change ids survive `describe`/`squash`/rebase, a comment stays
  attached to the code it was about — structurally impossible in git, where codiff must
  re-anchor to a commit sha.
- **Drift handling.** Store the commit id the comment was written against. When the change
  evolves, re-anchor by matching line content within the file; if the line is gone, mark
  the comment **outdated** and show it against its original text rather than silently
  dropping or misplacing it.
- **Storage moves to SQLite** (rusqlite). The JSON review store was right for flags and
  walkthroughs; threaded comments with anchors and timestamps want real queries.
- **UI**: click a line number to open an inline composer; comments render under their line;
  a Review tab lists every pending comment with its file and status; **Copy as Markdown**
  produces a paste-ready review for a chat or an agent prompt.

Ships useful without any forge integration — copy-as-markdown alone covers the solo and
agent-assisted workflows.

### C3 — Content jjdiff currently refuses to show

Two dead ends where we print a shrug:

- **Images.** `Binary file` today. Needs a `file_bytes(revset, path)` command returning
  base64 + mime, and a side-by-side old/new image view with a size cap. Renames and
  dimension changes are the interesting cases.
- **Markdown.** Rendered preview for `.md` files and for walkthrough/plan documents.
  Diffs of markdown stay diffs — this is a *view* toggle, not a replacement.

Small, self-contained, and each one removes a visible "the tool can't do this".

### C4 — Forge review via `gh` / `glab`

Reviewing *other people's* work, which jjdiff cannot do at all today. Scoped to the CLIs
rather than REST APIs, so auth is someone else's problem:

- `jjdiff pr 75` / `jjdiff mr 23` — fetch the PR head, review it as a normal diff.
- Show reviewers, merge state and CI checks alongside the diff.
- Submit a review (approve / request changes / comment) from the accumulated C2 comments.
- Colocated repos make this natural: the PR branch is a git ref jj can already address.

Depends on C2 for the comment model and C1 for the `pr`/`mr` subcommands.

### C5 — Native app polish

Individually small, collectively the difference between "a window" and "a Mac app":

- **Menu bar** (Tauri menu API): File / View / Change / Repository, mirroring the palette
  groups so the two never drift.
- **Keyboard shortcuts help** — a discoverable cheatsheet. We have `j/k/n/p/v`, Mod+F and
  the palette, and no way to learn them without reading the source.
- **Multi-window**, one per repo, which C1's single-instance work makes cheap.
- **Open in editor** — `editorCommand` config with `{file}`, `{line}`, `{repo}`
  placeholders, wired to a keybinding and the file tree's context menu.
- **App icon** — still the placeholder purple square generated in M0.

### C6 — Shared review links (deliberately last)

Codiff's Cloudflare service turns a walkthrough into a URL. It is the largest item here
(a service, a database, auth, retention) and the least useful while jjdiff has one user.
Revisit only if people other than the author start reviewing with it.

### Suggested order

1. **C1** ✅ — nothing else is reachable from a terminal without it, and it is a day or two.
2. **C2** ✅ — the biggest capability gap; ~1.5–2 weeks.
3. **C3** — a few days, removes two dead ends.
4. **C5** — polish, can be interleaved whenever.
5. **C4** — only when reviewing others' PRs actually matters to you.
6. **C6** — probably never, and that is fine.

## Risks specific to Phase 2

- **Mutations move the working copy.** `jj edit`/`new`/`rebase` rewrite files on disk; the
  watchers already catch this, but the UI must not hold stale paths across the change.
- **Rebase produces conflicts by design in jj** — that is not an error path, and the UI must
  present it as a normal outcome with the conflict surfacing we already have.
- **`jj op restore` rewrites the working copy too** — the same refresh discipline applies,
  and it must never be offered without a confirmation naming what it will undo.
- **Command sprawl.** Every command needs an applicability rule, a confirmation policy, and a
  test; the `mutate()` helper exists to keep that uniform rather than ad hoc per command.

**Deferred, with reasons.**
- *Signed macOS build + Homebrew tap* — blocked on an Apple Developer identity; the
  unsigned bundle already builds (`pnpm tauri build`).
- *Forge PR review* — the repo now lives on tangled.sh, so the original `gh`-based design
  no longer fits; wants a rethink against tangled's API rather than a port.
- *Shared-review web service* — weeks of work (codiff's Cloudflare equivalent) and
  questionable value before there is a second user.
- *Hunk-level squash/split* — needs a scripted diff-editor shim or jj-lib; file-level
  `squash`/`absorb` covers most of the need today.
- *Line-anchored review comments* — designed (change-id keyed, like viewed flags) but not
  built; would move the JSON store to SQLite.
- *Full-file syntax highlighting* — expand-context lines are deliberately untokenized;
  proper highlighting wants the whole file through shiki, which needs a highlight cache
  rework.

## Risks

- **jj CLI output stability** — mitigate with `-T` templates (stable contract), integration
  tests against a pinned jj version matrix (0.43+), and a single parsing module.
- **Working-copy staleness** — we diff the fs directly for `@`, so no snapshot needed for
  viewing; mutations let jj snapshot as usual. Must handle "user ran jj mid-view" via op watcher.
- **Conflicted trees via gix** — detect `.jjconflict-*`, always fall back to `jj file show`.
- **jj-lib churn** — it's Phase 2 and optional; the `crates/vcs` trait keeps it swappable.
- **Diff-view performance** — the real work (codiff's ReviewCodeView is 4k LOC alone). Budget
  M1 generously; virtualize from day one; keep highlighting off the main thread.
- **Shadow-DOM selection** — cross-shadow-root text selection/copy is still uneven
  (`getComposedRanges`); the light-DOM rule for the code pane exists to avoid it entirely.
  Verify select/copy/find UX in M1 before building comments on top.
- **Lit ecosystem thinness** — fewer off-the-shelf widgets than React (e.g. markdown comment
  editor will be hand-rolled). Acceptable for this app's surface; noted so it isn't a surprise.

## Effort

Solo: **~4 weeks to daily-driver (M2), ~8 weeks to M4.** The jj plumbing is the cheap 15%; the
diff UI is the long pole. Everything here is MIT-compatible; borrow UX (not code) from codiff.
