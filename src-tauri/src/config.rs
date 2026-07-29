//! `~/.config/jjdiff/config.toml` — same convention as jj itself (also on macOS).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub ui: UiConfig,
    pub keymap: Keymap,
    pub walkthrough: WalkthroughConfig,
    pub describe: DescribeConfig,
    pub editor: EditorConfig,
    pub workspace: WorkspaceConfig,
}

/// `[workspace]` — where jjdiff puts the working copies it creates.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceConfig {
    /// Parent directory for generated workspaces; each lands at
    /// `<root>/<repo dirname>/<workspace name>`.
    ///
    /// Under `~` rather than beside the repo on purpose: a workspace is a whole second
    /// checkout, and scattering them next to the work turns every project directory into a
    /// list of near-duplicates. Keeping them in one place also gives "is this one ours?" a
    /// definite answer, which is what decides whether jjdiff will delete a directory
    /// (`workspaces.rs`) rather than only forget it.
    pub root: String,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        WorkspaceConfig { root: "~/.jjdiff/workspaces".into() }
    }
}

impl WorkspaceConfig {
    /// The configured root with `~` expanded, or `None` when `HOME` is unset and the setting
    /// needs it — in which case jjdiff has nowhere of its own to put a workspace and says so
    /// rather than inventing a relative path next to whatever the cwd happens to be.
    pub fn resolved_root(&self) -> Option<PathBuf> {
        let raw = self.root.trim();
        if raw.is_empty() {
            return None;
        }
        let Some(rest) = raw.strip_prefix('~') else {
            return Some(PathBuf::from(raw));
        };
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join(rest.trim_start_matches('/')))
    }
}

/// `[describe]` — how the agent writes commit messages.
///
/// Separate from `[walkthrough]` although both drive the same CLI: the two ask
/// for different artefacts, and the instructions you would give for one are
/// rarely the ones you want on the other. "Always name the ticket" belongs on a
/// commit message; "call out anything touching a public API" belongs on a
/// review.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DescribeConfig {
    /// Extra instructions appended to the message-generation prompt — house
    /// rules the diff and the recent history cannot convey on their own.
    pub prompt: String,
}

/// `[editor]` — how "Open in Editor" launches an external editor.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct EditorConfig {
    /// Command template, e.g. `zed {file}:{line}` or `code -g {file}:{line}`.
    /// Placeholders: `{file}` (absolute path), `{line}`, `{repo}` (repo root).
    /// Empty = unset; the UI reports the config key instead of guessing an editor.
    ///
    /// Split on whitespace and executed directly — there is no shell, so pipes,
    /// globs and `&&` do not work (and command injection through a filename
    /// cannot either).
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct WalkthroughConfig {
    /// Which agent CLI generates walkthroughs: claude (default), codex, opencode, pi.
    pub backend: String,
    /// Model override for the selected backend; empty = the CLI's configured default.
    #[serde(alias = "claude-model")]
    pub claude_model: String,
    #[serde(alias = "codex-model")]
    pub codex_model: String,
    #[serde(alias = "opencode-model")]
    pub opencode_model: String,
    #[serde(alias = "pi-model")]
    pub pi_model: String,
    /// Extra instructions appended to every generation prompt.
    pub prompt: String,
}

impl WalkthroughConfig {
    /// The agent CLI this config selects, ready to run. One accessor rather
    /// than the same three lines at each call site: describe and walkthrough
    /// both read `[walkthrough] backend`, so giving `[describe]` its own key
    /// later is a change here and nowhere else.
    pub fn cli_backend(&self) -> crate::walkthrough::CliBackend {
        let selected = crate::walkthrough::Backend::parse(&self.backend);
        let model = self.model_for(selected);
        crate::walkthrough::CliBackend {
            backend: selected,
            model: (!model.is_empty()).then_some(model),
        }
    }

    /// Model override for whichever backend is selected.
    pub fn model_for(&self, backend: crate::walkthrough::Backend) -> String {
        use crate::walkthrough::Backend;
        match backend {
            Backend::Claude => self.claude_model.clone(),
            Backend::Codex => self.codex_model.clone(),
            Backend::OpenCode => self.opencode_model.clone(),
            Backend::Pi => self.pi_model.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Keymap {
    /// `Mod` = Cmd on macOS, Ctrl elsewhere. E.g. "Mod+Shift+p".
    #[serde(alias = "command-bar")]
    pub command_bar: String,
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap { command_bar: "Mod+k".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiConfig {
    /// "split" or "unified". Aliases accept jj-style kebab-case keys in the TOML file
    /// while the JSON sent to the UI stays camelCase.
    #[serde(alias = "diff-style")]
    pub diff_style: String,
    #[serde(alias = "code-font-size")]
    pub code_font_size: f32,
    #[serde(alias = "ignore-whitespace")]
    pub ignore_whitespace: bool,
    /// "system" (default), "light", or "dark".
    pub theme: String,
    #[serde(alias = "word-wrap")]
    pub word_wrap: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig {
            diff_style: "split".into(),
            code_font_size: 12.5,
            ignore_whitespace: false,
            theme: "system".into(),
            word_wrap: false,
        }
    }
}

pub fn config_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/jjdiff/config.toml"))
}

/// Load the config; missing file or parse errors fall back to defaults (an unreadable config
/// must never keep the app from starting — the error is logged instead).
pub fn load() -> Config {
    let Some(path) = config_path() else { return Config::default() };
    let Ok(raw) = std::fs::read_to_string(&path) else { return Config::default() };
    match toml::from_str(&raw) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("jjdiff: ignoring invalid {}: {error}", path.display());
            Config::default()
        }
    }
}

/// What TOML type a setting must be written as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Str,
    Bool,
    Float,
}

/// Every setting the settings page may write, and its type.
///
/// An allow-list rather than a passthrough, for two reasons that both bite.
///
/// The WebView renders forge markdown written by anyone who can open a pull
/// request, and it can invoke every command jjdiff exposes; one that took an
/// arbitrary table and key would let anything running in there write anywhere in
/// the user's config file.
///
/// And it pins the *type*. TOML is typed and `serde` is not forgiving about it:
/// `ignore-whitespace = "true"` is a string, deserializes as a bool nowhere, and
/// would take the whole `[ui]` table down to defaults on the next load — the
/// setting you just turned on silently turning everything else off with it.
///
/// Keys are kebab-case, which is what the file uses and what `Config`'s serde
/// aliases read; the JSON going the other way stays camelCase.
const WRITABLE: &[(&str, &str, Kind)] = &[
    ("ui", "theme", Kind::Str),
    ("ui", "diff-style", Kind::Str),
    ("ui", "code-font-size", Kind::Float),
    ("ui", "ignore-whitespace", Kind::Bool),
    ("ui", "word-wrap", Kind::Bool),
    ("keymap", "command-bar", Kind::Str),
    ("walkthrough", "backend", Kind::Str),
    ("walkthrough", "claude-model", Kind::Str),
    ("walkthrough", "codex-model", Kind::Str),
    ("walkthrough", "opencode-model", Kind::Str),
    ("walkthrough", "pi-model", Kind::Str),
    ("walkthrough", "prompt", Kind::Str),
    ("describe", "prompt", Kind::Str),
    ("editor", "command", Kind::Str),
];

/// Set one setting from the settings page. Unknown keys and mistyped values are
/// refused rather than written.
pub fn set_setting(
    table_name: &str,
    key: &str,
    value: &serde_json::Value,
) -> Result<PathBuf, String> {
    let kind = WRITABLE
        .iter()
        .find(|(table, name, _)| *table == table_name && *name == key)
        .map(|(_, _, kind)| *kind)
        .ok_or_else(|| format!("{table_name}.{key} is not a writable setting"))?;

    let written = match kind {
        Kind::Str => value
            .as_str()
            .map(toml_edit::value)
            .ok_or_else(|| format!("{table_name}.{key} takes a string")),
        Kind::Bool => value
            .as_bool()
            .map(toml_edit::value)
            .ok_or_else(|| format!("{table_name}.{key} takes true or false")),
        Kind::Float => value
            .as_f64()
            .map(toml_edit::value)
            .ok_or_else(|| format!("{table_name}.{key} takes a number")),
    }?;
    write_item(table_name, key, written)
}

/// The surgical edit itself.
///
/// Edits rather than re-serializes: this is the user's file, and round-tripping
/// it through `Config` would silently delete their comments, key order and any
/// setting a newer jjdiff added. `toml_edit` touches only the one value.
///
/// Returns the path written, for the confirmation message.
fn write_item(table_name: &str, key: &str, value: toml_edit::Item) -> Result<PathBuf, String> {
    let path = config_path().ok_or_else(|| "cannot locate $HOME".to_string())?;
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut document = existing
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| format!("{} is not valid TOML: {error}", path.display()))?;

    // Create the table explicitly when absent. Assigning straight into
    // `document[table][key]` auto-vivifies an *inline* table
    // (`editor = { command = "…" }`) and hoists it above any leading comment,
    // reordering a file we were asked only to add one key to.
    if !document.as_table().contains_key(table_name) {
        let mut table = toml_edit::Table::new();
        table.set_implicit(false);
        document[table_name] = toml_edit::Item::Table(table);
    }
    document[table_name][key] = value;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    std::fs::write(&path, document.to_string())
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let config = Config::default();
        assert_eq!(config.ui.diff_style, "split");
        assert!(!config.ui.ignore_whitespace);
        // Empty backend string parses to the Claude default.
        assert_eq!(
            crate::walkthrough::Backend::parse(&config.walkthrough.backend),
            crate::walkthrough::Backend::Claude
        );
    }

    #[test]
    fn walkthrough_backend_and_model_selection() {
        let config: Config = toml::from_str(
            "[walkthrough]\nbackend = \"opencode\"\nopencode-model = \"anthropic/claude-sonnet-4-6\"\nclaude-model = \"ignored\"\n",
        )
        .unwrap();
        let backend = crate::walkthrough::Backend::parse(&config.walkthrough.backend);
        assert_eq!(backend, crate::walkthrough::Backend::OpenCode);
        assert_eq!(config.walkthrough.model_for(backend), "anthropic/claude-sonnet-4-6");
    }

    /// A model set without a backend still reaches the CLI's argv.
    ///
    /// The whole chain, because each link was tested alone and the join is where
    /// it would break: an absent `backend` parses to Claude, `model_for` picks
    /// the matching per-backend key, and an empty override has to become *no*
    /// `--model` flag rather than an empty one — `--model ""` is an error, not a
    /// default. This is the config a real user hit it with: `claude-model` set,
    /// `backend` never written because Claude is already the default.
    #[test]
    fn a_model_with_no_backend_reaches_the_cli_arguments() {
        use crate::walkthrough::{Backend, CliBackend};

        let config: Config =
            toml::from_str("[walkthrough]\nclaude-model = \"claude-haiku-4-5\"\n").unwrap();
        let selected = Backend::parse(&config.walkthrough.backend);
        assert_eq!(selected, Backend::Claude, "an unset backend is Claude");
        let model = config.walkthrough.model_for(selected);
        assert_eq!(model, "claude-haiku-4-5");

        let cli = CliBackend { backend: selected, model: (!model.is_empty()).then_some(model) };
        assert_eq!(
            cli.args(),
            vec!["-p", "--output-format", "json", "--model", "claude-haiku-4-5"]
        );

        // And with nothing set at all, no `--model` — an empty one is an error.
        let bare: Config = toml::from_str("").unwrap();
        let selected = Backend::parse(&bare.walkthrough.backend);
        let model = bare.walkthrough.model_for(selected);
        let cli = CliBackend { backend: selected, model: (!model.is_empty()).then_some(model) };
        assert_eq!(cli.args(), vec!["-p", "--output-format", "json"]);
    }

    #[test]
    fn parses_partial_config() {
        let config: Config =
            toml::from_str("[ui]\ndiff-style = \"unified\"\ntheme = \"dark\"\n[keymap]\ncommand-bar = \"Mod+k\"\n")
                .unwrap();
        assert_eq!(config.ui.diff_style, "unified");
        assert_eq!(config.ui.theme, "dark");
        assert_eq!(config.keymap.command_bar, "Mod+k");
        // Unspecified fields keep defaults.
        assert_eq!(config.ui.code_font_size, 12.5);
    }

    #[test]
    fn garbage_falls_back_to_defaults() {
        assert!(toml::from_str::<Config>("not toml [").is_err());
    }

    /// The document edit a write performs, minus the filesystem. `set_setting`
    /// writes to `config_path()` (i.e. $HOME), which a test must not touch, but
    /// the edit itself is where the risk of mangling someone's file lives.
    fn edited(original: &str, command: &str) -> String {
        let mut document = original.parse::<toml_edit::DocumentMut>().unwrap();
        if !document.as_table().contains_key("editor") {
            let mut table = toml_edit::Table::new();
            table.set_implicit(false);
            document["editor"] = toml_edit::Item::Table(table);
        }
        document["editor"]["command"] = toml_edit::value(command);
        document.to_string()
    }

    /// The editor write must be surgical.
    #[test]
    fn editing_the_editor_command_keeps_the_rest_of_the_file() {
        let original = "# my jjdiff config\n\
                        [ui]\n\
                        # I like it dense\n\
                        theme = \"dark\"\n\
                        code-font-size = 11.5\n\n\
                        [walkthrough]\n\
                        backend = \"codex\"\n";
        let written = edited(original, "zed {file}:{line}");

        // Comments, key order and unrelated sections all survive.
        assert!(written.contains("# my jjdiff config"));
        assert!(written.contains("# I like it dense"));
        assert!(written.contains("backend = \"codex\""));
        assert!(written.contains("code-font-size = 11.5"));
        // A real section, appended — not an inline table hoisted above the
        // user's leading comment, which is what auto-vivification produces.
        assert!(written.contains("[editor]"), "expected a [editor] section:\n{written}");
        assert!(!written.contains("editor = {"), "must not write an inline table:\n{written}");
        assert!(
            written.trim_start().starts_with("# my jjdiff config"),
            "the file must still open with the user's own comment:\n{written}"
        );

        // And it reloads as the value we set, without disturbing the others.
        let parsed: Config = toml::from_str(&written).unwrap();
        assert_eq!(parsed.editor.command, "zed {file}:{line}");
        assert_eq!(parsed.ui.theme, "dark");
        assert_eq!(parsed.walkthrough.backend, "codex");
    }

    #[test]
    fn setting_the_editor_twice_replaces_rather_than_appends() {
        let written = edited("[editor]\ncommand = \"vim {file}\"\n", "code -g {file}:{line}");
        assert_eq!(written.matches("command =").count(), 1, "no duplicate key");
        let parsed: Config = toml::from_str(&written).unwrap();
        assert_eq!(parsed.editor.command, "code -g {file}:{line}");
    }

    /// The settings page may write these and nothing else. A command taking an
    /// arbitrary table and key would let anything running in the WebView — which
    /// renders forge markdown from anyone who can open a pull request — write
    /// anywhere in the user's config.
    #[test]
    fn only_known_settings_are_writable() {
        for (table, key, _) in WRITABLE {
            assert!(
                lookup(table, key).is_some(),
                "{table}.{key} is listed but not findable"
            );
        }
        assert!(lookup("ui", "themes").is_none(), "a typo must not be written");
        assert!(lookup("git", "user").is_none(), "an unrelated table must not be written");
        assert!(lookup("editor", "args").is_none());
    }

    fn lookup(table_name: &str, key: &str) -> Option<Kind> {
        WRITABLE
            .iter()
            .find(|(table, name, _)| *table == table_name && *name == key)
            .map(|(_, _, kind)| *kind)
    }

    /// Types are pinned, and this is not pedantry: TOML is typed and serde is
    /// not forgiving. `ignore-whitespace = "true"` is a string, deserializes as
    /// a bool nowhere, and takes the whole `[ui]` table down to defaults on the
    /// next load — the setting you just turned on quietly turning the rest off.
    #[test]
    fn a_setting_written_with_the_wrong_type_is_refused() {
        use serde_json::json;
        assert_eq!(lookup("ui", "ignore-whitespace"), Some(Kind::Bool));
        assert_eq!(lookup("ui", "code-font-size"), Some(Kind::Float));
        assert_eq!(lookup("ui", "theme"), Some(Kind::Str));

        // The conversion, without touching $HOME — `set_setting` writes to the
        // real config path, so only its typing half is exercised here.
        let typed = |kind: Kind, value: serde_json::Value| -> Option<String> {
            match kind {
                Kind::Str => value.as_str().map(toml_edit::value),
                Kind::Bool => value.as_bool().map(toml_edit::value),
                Kind::Float => value.as_f64().map(toml_edit::value),
            }
            .map(|item| item.to_string().trim().to_string())
        };
        assert_eq!(typed(Kind::Bool, json!(true)).as_deref(), Some("true"));
        assert_eq!(typed(Kind::Bool, json!("true")), None, "a string is not a bool");
        assert_eq!(typed(Kind::Float, json!(13.5)).as_deref(), Some("13.5"));
        assert_eq!(typed(Kind::Float, json!("13.5")), None, "a string is not a number");
        assert_eq!(typed(Kind::Str, json!("dark")).as_deref(), Some("\"dark\""));
        assert_eq!(typed(Kind::Str, json!(false)), None, "a bool is not a string");
    }

    /// A bool must land unquoted, or it reloads as a string and the `[ui]` table
    /// falls back to defaults.
    #[test]
    fn a_boolean_setting_round_trips_as_a_boolean() {
        let mut document = "[ui]\ntheme = \"dark\"\n".parse::<toml_edit::DocumentMut>().unwrap();
        document["ui"]["word-wrap"] = toml_edit::value(true);
        let written = document.to_string();
        assert!(written.contains("word-wrap = true"), "{written}");
        let parsed: Config = toml::from_str(&written).unwrap();
        assert!(parsed.ui.word_wrap);
        assert_eq!(parsed.ui.theme, "dark", "the neighbour survives");
    }

    #[test]
    fn writes_into_an_empty_or_absent_config() {
        // First-run case: no file yet, so the document starts empty.
        let written = edited("", "zed {file}:{line}");
        let parsed: Config = toml::from_str(&written).unwrap();
        assert_eq!(parsed.editor.command, "zed {file}:{line}");
        assert!(written.contains("[editor]"));
        // Defaults for everything else still apply.
        assert_eq!(parsed.ui.diff_style, "split");
    }
}
