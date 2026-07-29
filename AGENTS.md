# AGENTS.md — jjdiff

## Project

**jjdiff** — A fast, minimal desktop diff viewer for reviewing and landing changes in [Jujutsu](https://jj-vcs.dev) colocated repos. Tauri 2 + Rust + Lit.

## Commands

```bash
pnpm install                  # Install JS dependencies
pnpm tauri dev                # Dev mode (app against current directory)
pnpm tauri dev -- -- -R /path # Against another repo
pnpm build                    # Typecheck + bundle UI
cargo test --workspace        # Run all Rust tests
cargo clippy --workspace      # Lint
```

## Layout

```
crates/vcs/     — jj CLI facade (read/mutate discipline, JSONL templates)
crates/diff/    — patch parsing; fs-vs-tree diffing via gix
crates/watch/   — op-head watcher (change detection without polling)
src-tauri/      — Tauri app shell + IPC commands
ui/             — Lit frontend (light-DOM code pane)
```

## Architecture

- **Backend**: Rust workspace (`src-tauri` + 3 crates). Tauri commands in `src-tauri/src/lib.rs` are `async` wrappers around blocking `jj`/fs work via `spawn_blocking`.
- **Frontend**: Lit web components in `ui/src/`. IPC types in `ui/src/ipc.ts` mirror `lib.rs` commands. In `pnpm dev` (browser), `mock.ts` provides fixtures via dynamic `import()`.
- **VCS layer**: `Repo` struct is cheap-to-clone (two paths). Every *read* uses `--ignore-working-copy --color=never --no-pager` to avoid snapshots and lock contention. Every *mutation* goes through the real CLI without those flags.
- **Diff layer**: Two producers (`parse_git_patch` from `jj diff --git`, and `worktree::diff_worktree` from live fs vs base tree via gix) both converge on `FilePatch[]` and call `assign_hunk_ids` + `add_word_spans`.
- **Watch layer**: `RepoWatcher` uses `notify` to watch op heads and working copy; emits `repo-changed` events to the frontend.

## Gotchas

- `jj` must be ≥ 0.33 on PATH. Set `JJDIFF_JJ_PATH` to override the binary.
- Repos must be **colocated** (`.git` directory inside the jj workspace). Non-colocated repos error with `NotColocated`.
- `--ignore-working-copy` is only used for reads — mutations bypass it to match user's own `jj` snapshot semantics.
- Span offsets in `Line.spans` are **UTF-16 code units** (to match JS `String` indexing).
- The `IN_TAURI` check (`'__TAURI_INTERNALS__' in window`) gates all IPC calls — frontend must handle the mock path for `pnpm dev`.
- An overlay's backdrop test is `event.composedPath()[0] === this`. Bound to the host, a listener sees inside clicks retargeted to the host, so `event.target === this` closes the overlay on any click it receives. The same retargeting means an overlay with a text field must handle keys on its panel and `stopPropagation()`, or the app's single-key review shortcuts fire while you type.
- `Repo::op_diff` returns jj's prose for display, not a structure: `jj op diff` has no `json()` form. Don't write a parser for it.
- Hunk-level split works by jjdiff *being* jj's diff editor: `jj split --tool` re-enters the binary as `--apply-split-plan <plan> $left $right`. The plan comes from the diff on screen, and `apply_selected_hunks` refuses it unless applying every hunk reproduces the right side exactly.
