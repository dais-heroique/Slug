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

/// A provider choice for `[brain] provider`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// Pick a provider from the hardware tier (cloud → claude, local → ollama).
    #[default]
    Auto,
    Claude,
    Openai,
    Openrouter,
    Gemini,
    Ollama,
}

/// Per-provider settings. API keys are read from the env var named here and are
/// **never** stored in the file.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderCfg {
    /// Environment variable holding the API key (empty for keyless local servers).
    pub api_key_env: String,
    /// API base URL (ignored by `claude`; the API root for the others).
    pub base_url: String,
    /// Model id; empty → use the hardware tier's recommendation.
    pub model: String,
}

impl ProviderCfg {
    fn make(api_key_env: &str, base_url: &str, model: &str) -> Self {
        ProviderCfg { api_key_env: api_key_env.into(), base_url: base_url.into(), model: model.into() }
    }
}

impl Default for ProviderCfg {
    fn default() -> Self {
        ProviderCfg::make("", "", "")
    }
}

/// All provider blocks (`[providers.X]`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ProvidersCfg {
    pub claude: ProviderCfg,
    pub openai: ProviderCfg,
    pub openrouter: ProviderCfg,
    pub gemini: ProviderCfg,
    pub ollama: ProviderCfg,
}

impl Default for ProvidersCfg {
    fn default() -> Self {
        ProvidersCfg {
            claude: ProviderCfg::make("ANTHROPIC_API_KEY", "", "claude-sonnet-4-6"),
            openai: ProviderCfg::make("OPENAI_API_KEY", "https://api.openai.com/v1", "gpt-4o"),
            openrouter: ProviderCfg::make(
                "OPENROUTER_API_KEY",
                "https://openrouter.ai/api/v1",
                "openai/gpt-4o",
            ),
            gemini: ProviderCfg::make(
                "GEMINI_API_KEY",
                "https://generativelanguage.googleapis.com",
                "gemini-2.0-flash",
            ),
            ollama: ProviderCfg::make("", "http://127.0.0.1:11434", ""),
        }
    }
}

/// `[brain]` — top-level provider selection.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BrainCfg {
    pub provider: Provider,
}

/// The full configuration.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub brain: BrainCfg,
    pub providers: ProvidersCfg,
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

    /// Canonical `(api_key_env, base_url)` fallback for a provider — used to fill
    /// empty fields when a user writes a partial `[providers.X]` block.
    pub fn provider_defaults(p: Provider) -> (&'static str, &'static str) {
        match p {
            Provider::Claude => ("ANTHROPIC_API_KEY", ""),
            Provider::Openai => ("OPENAI_API_KEY", "https://api.openai.com/v1"),
            Provider::Openrouter => ("OPENROUTER_API_KEY", "https://openrouter.ai/api/v1"),
            Provider::Gemini => ("GEMINI_API_KEY", "https://generativelanguage.googleapis.com"),
            Provider::Ollama => ("", "http://127.0.0.1:11434"),
            Provider::Auto => ("", ""),
        }
    }

    /// The provider block with empty `api_key_env`/`base_url` filled from the
    /// canonical defaults, so partial config blocks still work.
    pub fn resolved_provider(&self, p: Provider) -> ProviderCfg {
        let block = match p {
            Provider::Claude => &self.providers.claude,
            Provider::Openai => &self.providers.openai,
            Provider::Openrouter => &self.providers.openrouter,
            Provider::Gemini => &self.providers.gemini,
            Provider::Ollama | Provider::Auto => &self.providers.ollama,
        }
        .clone();
        let (env, url) = Self::provider_defaults(p);
        ProviderCfg {
            api_key_env: if block.api_key_env.is_empty() { env.to_string() } else { block.api_key_env },
            base_url: if block.base_url.is_empty() { url.to_string() } else { block.base_url },
            model: block.model,
        }
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

    #[test]
    fn provider_defaults_and_parsing() {
        // Default provider is Auto, and every provider block has sane defaults.
        let cfg = Config::default();
        assert_eq!(cfg.brain.provider, Provider::Auto);
        assert_eq!(cfg.providers.openai.base_url, "https://api.openai.com/v1");
        assert_eq!(cfg.providers.openrouter.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(cfg.providers.gemini.api_key_env, "GEMINI_API_KEY");
        assert!(cfg.providers.ollama.api_key_env.is_empty());

        // A user can select a provider and override one block.
        let cfg: Config = toml::from_str(
            "[brain]\nprovider = \"openrouter\"\n[providers.openrouter]\nmodel = \"anthropic/claude-3.5-sonnet\"\n",
        )
        .unwrap();
        assert_eq!(cfg.brain.provider, Provider::Openrouter);
        assert_eq!(cfg.providers.openrouter.model, "anthropic/claude-3.5-sonnet");
        // A partial block resets siblings to empty at the serde layer, but
        // `resolved_provider` fills them from the canonical defaults.
        let resolved = cfg.resolved_provider(Provider::Openrouter);
        assert_eq!(resolved.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(resolved.api_key_env, "OPENROUTER_API_KEY");
        assert_eq!(resolved.model, "anthropic/claude-3.5-sonnet");
    }
}
