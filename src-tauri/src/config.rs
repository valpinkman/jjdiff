//! `~/.config/jjdiff/config.toml` — same convention as jj itself (also on macOS).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub ui: UiConfig,
    pub keymap: Keymap,
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
        Keymap { command_bar: "Mod+Shift+p".into() }
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
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig {
            diff_style: "split".into(),
            code_font_size: 12.5,
            ignore_whitespace: false,
            theme: "system".into(),
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
