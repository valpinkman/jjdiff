//! `~/.config/jjdiff/config.toml` — same convention as jj itself (also on macOS).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub ui: UiConfig,
    pub keymap: Keymap,
    pub walkthrough: WalkthroughConfig,
    pub editor: EditorConfig,
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

/// Write `[editor] command` back to the config file.
///
/// Edits rather than re-serializes: this is the user's file, and round-tripping
/// it through `Config` would silently delete their comments, key order and any
/// setting a newer jjdiff added. `toml_edit` touches only the one value.
///
/// Returns the path written, for the confirmation message.
pub fn set_editor_command(command: &str) -> Result<PathBuf, String> {
    set_value("editor", "command", command)
}

/// Write `[ui] theme` back to the config file. Same surgical edit as above — a
/// theme is chosen once and expected to survive a restart.
pub fn set_ui_theme(theme: &str) -> Result<PathBuf, String> {
    set_value("ui", "theme", theme)
}

/// Set one `[table] key` in the user's config, touching nothing else.
///
/// Edits rather than re-serializes: this is the user's file, and round-tripping
/// it through `Config` would silently delete their comments, key order and any
/// setting a newer jjdiff added. `toml_edit` touches only the one value.
///
/// Returns the path written, for the confirmation message.
fn set_value(table_name: &str, key: &str, value: &str) -> Result<PathBuf, String> {
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
    document[table_name][key] = toml_edit::value(value);

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

    /// The document edit `set_editor_command` performs, minus the filesystem.
    /// It writes to `config_path()` (i.e. $HOME), which a test must not touch,
    /// but the edit itself is where the risk of mangling someone's file lives.
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
