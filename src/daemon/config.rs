use regex::Regex;
use serde::Deserialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// A rule for matching windows and triggering actions.
/// At least one of `layer`, `virtual_key`, or `raw_vk_action` should be specified.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rule {
    pub(crate) class: Option<String>,
    pub(crate) title: Option<String>,
    /// Layer to switch to when switching to a native terminal (VT)
    pub(crate) on_native_terminal: Option<String>,
    /// Layer to switch to when rule matches
    pub(crate) layer: Option<String>,
    /// Virtual key to press while window is focused (auto-released on unfocus)
    pub(crate) virtual_key: Option<String>,
    /// Raw virtual key actions to fire on focus (fire-and-forget)
    /// Format: [["vk_name", "Press|Release|Tap|Toggle"], ...]
    pub(crate) raw_vk_action: Option<Vec<(String, String)>>,
    /// Continue matching subsequent rules after this one
    #[serde(default)]
    pub(crate) fallthrough: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct NativeTerminalRule {
    pub(crate) layer: String,
    pub(crate) virtual_key: Option<String>,
    pub(crate) raw_vk_action: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub(crate) enum ConfigEntry {
    Default { default: String },
    Rule(Rule),
}

impl<'de> serde::Deserialize<'de> for ConfigEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        let value = serde_json::Value::deserialize(deserializer)?;

        // Check if it's a "default" entry
        if let Some(obj) = value.as_object()
            && obj.contains_key("default")
        {
            if obj.len() == 1
                && let Some(default) = obj.get("default").and_then(|v| v.as_str())
            {
                return Ok(ConfigEntry::Default {
                    default: default.to_string(),
                });
            }
            return Err(D::Error::custom(
                "'default' entry should only contain the 'default' field",
            ));
        }

        // Try to parse as Rule with custom error handling for unknown fields
        let known_fields = [
            "class",
            "title",
            "on_native_terminal",
            "layer",
            "virtual_key",
            "raw_vk_action",
            "fallthrough",
        ];

        if let Some(obj) = value.as_object() {
            for key in obj.keys() {
                if !known_fields.contains(&key.as_str()) {
                    return Err(D::Error::custom(format!(
                        "unknown field '{}'. Valid fields are: class, title, on_native_terminal, layer, virtual_key, raw_vk_action, fallthrough",
                        key
                    )));
                }
            }
        }

        serde_json::from_value(value)
            .map(ConfigEntry::Rule)
            .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Config {
    pub(crate) rules: Vec<Rule>,
    pub(crate) default_layer: Option<String>,
    pub(crate) native_terminal_rule: Option<NativeTerminalRule>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct WindowInfo {
    pub(crate) class: String,
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) is_native_terminal: bool,
}

pub(crate) fn load_config(config_path: Option<&Path>) -> Config {
    let path = config_path.map(|p| p.to_path_buf()).unwrap_or_else(|| {
        let xdg_config = env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| dirs::home_dir().unwrap().join(".config"));
        xdg_config.join("kanata").join("kanata-switcher.json")
    });

    if !path.exists() {
        eprintln!("[Config] Error: Config file not found: {}", path.display());
        eprintln!();
        eprintln!("Example config:");
        eprintln!(
            r#"[
  {{"default": "base"}},
  {{"on_native_terminal": "tty"}},
  {{"class": "firefox", "layer": "browser"}},
  {{"class": "alacritty", "title": "vim", "layer": "vim"}}
]"#
        );
        std::process::exit(1);
    }

    match fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<Vec<ConfigEntry>>(&content) {
            Ok(entries) => {
                let mut rules = Vec::new();
                let mut default_layer: Option<String> = None;
                let mut native_terminal_rule: Option<NativeTerminalRule> = None;

                for entry in entries {
                    match entry {
                        ConfigEntry::Default { default } => {
                            if default_layer.is_some() {
                                eprintln!(
                                    "[Config] Error: multiple 'default' entries found, only one allowed"
                                );
                                std::process::exit(1);
                            }
                            default_layer = Some(default);
                        }
                        ConfigEntry::Rule(rule) => {
                            if let Some(layer) = rule.on_native_terminal.clone() {
                                if rule.class.is_some() || rule.title.is_some() {
                                    eprintln!(
                                        "[Config] Error: 'on_native_terminal' cannot be combined with 'class' or 'title'"
                                    );
                                    std::process::exit(1);
                                }
                                if rule.layer.is_some() {
                                    eprintln!(
                                        "[Config] Error: 'on_native_terminal' cannot be combined with 'layer'"
                                    );
                                    std::process::exit(1);
                                }
                                if native_terminal_rule.is_some() {
                                    eprintln!(
                                        "[Config] Error: multiple 'on_native_terminal' rules found, only one allowed"
                                    );
                                    std::process::exit(1);
                                }
                                native_terminal_rule = Some(NativeTerminalRule {
                                    layer,
                                    virtual_key: rule.virtual_key.clone(),
                                    raw_vk_action: rule.raw_vk_action.clone().unwrap_or_default(),
                                });
                            } else {
                                // Rule with no matchers and no fallthrough would match everything
                                // and stop further matching, which is almost certainly a bug
                                if rule.class.is_none() && rule.title.is_none() && !rule.fallthrough
                                {
                                    eprintln!(
                                        "[Config] Error: Rule with no 'class' or 'title' matcher requires 'fallthrough: true'"
                                    );
                                    eprintln!(
                                        "[Config] Hint: A catch-all rule without fallthrough would match all windows and stop further matching"
                                    );
                                    std::process::exit(1);
                                }
                                rules.push(rule);
                            }
                        }
                    }
                }

                println!(
                    "[Config] Loaded {} rules from {}",
                    rules.len(),
                    path.display()
                );

                Config {
                    rules,
                    default_layer,
                    native_terminal_rule,
                }
            }
            Err(e) => {
                eprintln!("[Config] Error: Failed to parse {}: {}", path.display(), e);
                std::process::exit(1);
            }
        },
        Err(e) => {
            eprintln!("[Config] Error: Failed to read {}: {}", path.display(), e);
            std::process::exit(1);
        }
    }
}

pub(crate) fn match_pattern(pattern: Option<&str>, value: &str) -> bool {
    match pattern {
        None => true,
        Some("*") => true,
        Some(pat) => match Regex::new(pat) {
            Ok(re) => re.is_match(value),
            Err(_) => value.contains(pat),
        },
    }
}
