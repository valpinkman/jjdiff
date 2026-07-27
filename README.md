# jjdiff

A fast, minimal desktop diff viewer for reviewing and landing changes in
[Jujutsu](https://jj-vcs.dev) colocated repos. Tauri 2 + Rust + Lit. jj-native from day one:
no staging axis, change-id identity, stack review. See [PLAN.md](PLAN.md).

## Development

Requires Rust (stable), Node 20+, pnpm, and `jj` ≥ 0.33 on PATH.

```bash
pnpm install
pnpm tauri dev            # app against the repo it was launched from
pnpm tauri dev -- -- -R /path/to/repo   # against another repo
```

Checks:

```bash
cargo test --workspace
cargo clippy --workspace
pnpm build                # typecheck + bundle the UI
```

## Configuration

`~/.config/jjdiff/config.toml`, same convention as jj itself:

```toml
[ui]
diff-style = "split"    # or "unified"
theme = "system"        # or "light" / "dark"

[keymap]
command-bar = "Mod+k"   # Mod is Cmd on macOS, Ctrl elsewhere

[editor]
# Placeholders: {file} (absolute), {line}, {repo}. No shell — split on spaces.
command = "zed {file}:{line}"
```

Press `?` in the app for the shortcut list.

## Layout

- `crates/vcs` — jj CLI facade (read/mutate discipline, JSONL templates)
- `crates/diff` — patch parsing; later fs-vs-tree diffing
- `crates/watch` — op-head watcher (change detection without polling)
- `src-tauri` — app shell + IPC commands
- `ui` — Lit frontend (light-DOM code pane)
