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
}
