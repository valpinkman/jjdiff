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

**M0 — skeleton (wk 1).** Tauri scaffold, Rust workspace, `jj` CLI wrapper with
`--ignore-working-copy` discipline, op-head watcher, `jjdiff [revset] [path]` CLI entry.

**M1 — read-only reviewer (wks 2–3).** Working-copy + arbitrary-revset diffs; virtualized
split/unified views, word-level intra-line diffs, whitespace toggle; file tree; Shiki
highlighting; large-file/binary/image guards. *Usable as a `jj diff` replacement.*

**M2 — jj-native actions (wk 4).** Describe editor (commit message UX), `new`, file-level
`squash --into`, viewed flags keyed by change id, command bar, config file. *Daily driver.*

**M3 — stack review (wks 5–6).** Stack sidebar over `trunk()..@`, per-change review pages,
interdiff between evolog entries ("what changed since I last reviewed"), absorb button,
move-file-to-change. *The jj-shaped feature no git tool has.*

**M4 — conflicts + polish (wks 7–8).** Structured conflict rendering, `resolve` integration,
themes, keymap, packaging (dmg/deb/rpm + Homebrew tap), signed macOS build.

**Later.** GitHub PR review via `gh` on colocated bookmarks; LLM walkthroughs (port codiff's
agent-CLI pattern — it's VCS-agnostic); shared-review web service; hunk-level squash/split.

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
