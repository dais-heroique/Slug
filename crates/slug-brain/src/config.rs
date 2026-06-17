//! `slug.toml` configuration: backend selection, models, and safety caps.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Backend selection policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Selection {
    /// Pick local or cloud automatically from the hardware tier.
    #[default]
    Auto,
    /// Force the local Ollama backend.
    Local,
    /// Force the Claude API backend.
    Cloud,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct BackendCfg {
    pub selection: Selection,
}

impl Default for BackendCfg {
    fn default() -> Self {
        BackendCfg { selection: Selection::Auto }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalCfg {
    pub ollama_host: String,
    /// Empty → use the hardware tier's recommended model.
    pub model: String,
    pub num_ctx: u32,
}

impl Default for LocalCfg {
    fn default() -> Self {
        LocalCfg { ollama_host: "http://127.0.0.1:11434".into(), model: String::new(), num_ctx: 8192 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CloudCfg {
    pub model: String,
    pub api_key_env: String,
    pub max_tokens: u32,
}

impl Default for CloudCfg {
    fn default() -> Self {
        CloudCfg {
            model: "claude-sonnet-4-6".into(),
            api_key_env: "ANTHROPIC_API_KEY".into(),
            max_tokens: 4096,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CapsCfg {
    /// Per-session cumulative token cap (input+output). 0 = unlimited.
    pub max_tokens_per_session: u64,
    /// Per-session USD cost cap (cloud only). 0 = unlimited.
    pub max_cost_usd: f64,
    /// Maximum observe→reason→act→verify iterations before giving up.
    pub max_steps: u32,
}

impl Default for CapsCfg {
    fn default() -> Self {
        CapsCfg { max_tokens_per_session: 200_000, max_cost_usd: 1.0, max_steps: 25 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SafetyCfg {
    /// Require confirmation before destructive actions (delete/send/purchase…).
    pub confirm_destructive: bool,
}

impl Default for SafetyCfg {
    fn default() -> Self {
        SafetyCfg { confirm_destructive: true }
    }
}

/// The full configuration.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub backend: BackendCfg,
    pub local: LocalCfg,
    pub cloud: CloudCfg,
    pub caps: CapsCfg,
    pub safety: SafetyCfg,
}

impl Config {
    /// Load from a TOML file. Returns defaults if the file does not exist.
    pub fn load(path: &Path) -> anyhow::Result<Config> {
        if !path.exists() {
            return Ok(Config::default());
        }
        let text = std::fs::read_to_string(path)?;
        let cfg: Config = toml::from_str(&text)?;
        Ok(cfg)
    }

    /// Serialize to a TOML string (for `--write-config`).
    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip() {
        let cfg = Config::default();
        let toml = cfg.to_toml();
        let back: Config = toml::from_str(&toml).unwrap();
        assert_eq!(back.backend.selection, Selection::Auto);
        assert_eq!(back.cloud.model, "claude-sonnet-4-6");
        assert_eq!(back.caps.max_steps, 25);
    }

    #[test]
    fn partial_toml_fills_defaults() {
        let cfg: Config = toml::from_str("[backend]\nselection = \"cloud\"\n").unwrap();
        assert_eq!(cfg.backend.selection, Selection::Cloud);
        // Unspecified sections still get defaults.
        assert_eq!(cfg.local.ollama_host, "http://127.0.0.1:11434");
    }

    #[test]
    fn missing_file_is_defaults() {
        let cfg = Config::load(Path::new("/nonexistent/slug.toml")).unwrap();
        assert_eq!(cfg.backend.selection, Selection::Auto);
    }
}
