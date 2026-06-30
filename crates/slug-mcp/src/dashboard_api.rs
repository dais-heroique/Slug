//! Backend for the control dashboard: which brain (Claude vs local) is configured,
//! and the AI-provider catalog/connection state.
//!
//! Dependency-free on purpose (std + serde_json only): the config is read with a
//! tiny hand parser so `slug-mcp` needs no TOML crate.

use std::path::PathBuf;

use serde_json::{json, Value};

/// `~/.slug` (honoured for both the config and the secrets file).
fn slug_home() -> PathBuf {
    if let Ok(h) = std::env::var("HOME") {
        return PathBuf::from(h).join(".slug");
    }
    if let Ok(h) = std::env::var("USERPROFILE") {
        return PathBuf::from(h).join(".slug");
    }
    PathBuf::from(".slug")
}

fn config_path() -> PathBuf {
    std::env::var("SLUG_CONFIG").map(PathBuf::from).unwrap_or_else(|_| slug_home().join("slug.toml"))
}

/// Best-effort read of the configured brain provider and model from `slug.toml`,
/// without a TOML dependency. Returns (`provider`, `model`). Falls back to a
/// sensible default when no config is present.
pub fn brain_info() -> (String, String) {
    let text = std::fs::read_to_string(config_path()).unwrap_or_default();
    let mut provider = String::new();
    let mut section = String::new();
    let mut models: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_string();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim();
            let val = v.trim().trim_matches('"').trim().to_string();
            if section == "brain" && key == "provider" {
                provider = val;
            } else if let Some(p) = section.strip_prefix("providers.") {
                if key == "model" {
                    models.insert(p.to_string(), val);
                }
            }
        }
    }

    if provider.is_empty() {
        provider = "claude".to_string();
    }
    let model = models.get(&provider).cloned().unwrap_or_else(|| match provider.as_str() {
        "claude" => "claude-sonnet-4-6".into(),
        "ollama" => "qwen3:8b".into(),
        _ => String::new(),
    });
    (provider, model)
}

/// Whether a provider runs in the cloud (Claude/OpenAI/…) or locally (Ollama).
pub fn provider_is_local(provider: &str) -> bool {
    matches!(provider, "ollama" | "local")
}

/// The env var that holds the active provider's API key: an explicit
/// `api_key_env` in `slug.toml` wins, else the catalog default for the slot.
fn active_key_env() -> String {
    let (provider, _) = brain_info();
    if let Some(env) = config_value(&format!("providers.{provider}"), "api_key_env") {
        if !env.trim().is_empty() {
            return env;
        }
    }
    provider_catalog()
        .iter()
        .find(|p| p.2 == provider)
        .map(|p| p.4.to_string())
        .unwrap_or_default()
}

/// Whether Slug's **built-in** agent can run: local providers always can; cloud
/// providers need their API key present in the environment. Returns a clear,
/// actionable hint when it can't (shown in the dashboard instead of a silent
/// failure).
pub fn brain_ready() -> Result<(), String> {
    let (provider, _) = brain_info();
    if provider_is_local(&provider) {
        return Ok(());
    }
    let env = active_key_env();
    if env.is_empty() || key_present(&env) {
        Ok(())
    } else {
        Err(format!(
            "no API key for the built-in agent — set {env} in the Brain tab (or export it), \
             then try again."
        ))
    }
}

// ------------------------------- heartbeat ----------------------------------
//
// "Is an MCP client connected" can't be answered from in-process state alone:
// the HTTP daemon (serving the dashboard) and the stdio server Claude Code
// spawns are **separate OS processes** with separate memory. A client driving
// Slug over stdio never touches the daemon's process at all, so an in-memory
// counter on the daemon would forever show "disconnected" even while you're
// actively using Slug from Claude Code. Both transports instead stamp a shared
// file on every request; the dashboard reads it to learn the freshest contact,
// regardless of which process or transport produced it.

fn heartbeat_path() -> PathBuf {
    std::env::var("SLUG_HEARTBEAT").map(PathBuf::from).unwrap_or_else(|_| slug_home().join("heartbeat.json"))
}

/// Stamp "a client just talked to us over `transport`" (`"stdio"` | `"http"`).
/// Best-effort: a failed write must never break request handling.
pub fn record_heartbeat(transport: &str) {
    let body = json!({ "transport": transport, "pid": std::process::id(), "ts": now_unix_secs() });
    let path = heartbeat_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, body.to_string());
}

/// Read the last heartbeat, if any client has ever connected.
fn read_heartbeat() -> Option<Value> {
    let text = std::fs::read_to_string(heartbeat_path()).ok()?;
    serde_json::from_str(&text).ok()
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Whether an MCP client (either transport) has been seen within the last
/// `fresh_secs` seconds, and which transport/age that was — for the dashboard's
/// "client connected" indicator.
pub fn client_status(fresh_secs: u64) -> Value {
    let Some(hb) = read_heartbeat() else {
        return json!({ "connected": false, "transport": Value::Null, "last_seen_s": Value::Null });
    };
    let ts = hb.get("ts").and_then(Value::as_u64).unwrap_or(0);
    let age = now_unix_secs().saturating_sub(ts);
    json!({
        "connected": age < fresh_secs,
        "transport": hb.get("transport").cloned().unwrap_or(Value::Null),
        "last_seen_s": age,
    })
}

/// Brain summary for the dashboard header: provider, model, cloud/local, the key
/// env var, and whether the built-in agent is ready to run.
pub fn brain_detail() -> Value {
    let (provider, model) = brain_info();
    let local = provider_is_local(&provider);
    json!({
        "provider": provider,
        "model": model,
        "location": if local { "local" } else { "cloud" },
        "key_env": active_key_env(),
        "ready": brain_ready().is_ok(),
    })
}

// ------------------------------ AI providers -------------------------------
//
// "Connect via API" from the dashboard. Every entry maps to one of the brain's
// provider slots (`claude` / `gemini` / `openrouter` / `openai` / `ollama`); the
// many OpenAI-compatible services (Groq, Mistral, DeepSeek, xAI, Together,
// Perplexity, local servers) all ride the `openai` slot with their own base_url.
// Activating one writes `slug.toml`; **API keys are never written** — they are read
// from the named env var (you can also inject one in-memory for this session).

/// (id, label, slot, base_url, key_env, default_model, kind)
type Preset = (&'static str, &'static str, &'static str, &'static str, &'static str, &'static str, &'static str);

pub fn provider_catalog() -> Vec<Preset> {
    vec![
        ("anthropic", "Anthropic (Claude)", "claude", "", "ANTHROPIC_API_KEY", "claude-sonnet-4-6", "cloud"),
        ("gemini", "Google Gemini", "gemini", "", "GEMINI_API_KEY", "gemini-2.0-flash", "cloud"),
        ("openrouter", "OpenRouter (all models)", "openrouter", "https://openrouter.ai/api/v1", "OPENROUTER_API_KEY", "openai/gpt-4o", "gateway"),
        ("openai", "OpenAI", "openai", "https://api.openai.com/v1", "OPENAI_API_KEY", "gpt-4o", "cloud"),
        ("groq", "Groq", "openai", "https://api.groq.com/openai/v1", "GROQ_API_KEY", "llama-3.3-70b-versatile", "cloud"),
        ("mistral", "Mistral", "openai", "https://api.mistral.ai/v1", "MISTRAL_API_KEY", "mistral-large-latest", "cloud"),
        ("deepseek", "DeepSeek", "openai", "https://api.deepseek.com/v1", "DEEPSEEK_API_KEY", "deepseek-chat", "cloud"),
        ("xai", "xAI (Grok)", "openai", "https://api.x.ai/v1", "XAI_API_KEY", "grok-2-latest", "cloud"),
        ("together", "Together AI", "openai", "https://api.together.xyz/v1", "TOGETHER_API_KEY", "meta-llama/Llama-3.3-70B-Instruct-Turbo", "cloud"),
        ("perplexity", "Perplexity", "openai", "https://api.perplexity.ai", "PERPLEXITY_API_KEY", "sonar", "cloud"),
        ("ollama", "Ollama (local)", "ollama", "http://127.0.0.1:11434", "", "qwen3:8b", "local"),
    ]
}

/// Read a `key = "value"` from a given `[section]` of slug.toml (best effort).
fn config_value(section: &str, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(config_path()).ok()?;
    let mut cur = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            cur = line[1..line.len() - 1].trim().to_string();
        } else if cur == section {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim() == key {
                    return Some(v.trim().trim_matches('"').trim().to_string());
                }
            }
        }
    }
    None
}

/// Catalog + which preset is active + whether each key env is set in this process.
pub fn providers_status() -> Value {
    let (active_provider, active_model) = brain_info();
    let active_base = config_value(&format!("providers.{active_provider}"), "base_url").unwrap_or_default();

    let items: Vec<Value> = provider_catalog()
        .into_iter()
        .map(|(id, label, slot, base_url, key_env, default_model, kind)| {
            // Active if the brain slot matches and (for the shared openai slot) the
            // base_url matches too.
            let active = slot == active_provider
                && (slot != "openai" || base_url == active_base || active_base.is_empty() && id == "openai");
            let key_present = if key_env.is_empty() {
                true // local (ollama) needs no key
            } else {
                std::env::var(key_env).map(|v| !v.trim().is_empty()).unwrap_or(false)
            };
            json!({
                "id": id, "label": label, "slot": slot, "base_url": base_url,
                "key_env": key_env, "default_model": default_model, "kind": kind,
                "key_present": key_present, "active": active,
                "model": if active { active_model.clone() } else { default_model.to_string() },
            })
        })
        .collect();

    json!({ "active": { "provider": active_provider, "model": active_model }, "providers": items })
}

/// Activate a provider: rewrite slug.toml's `[brain]` + the chosen
/// `[providers.<slot>]` block, preserving `[caps]`/`[safety]`. Keys are NOT stored.
pub fn set_provider(slot: &str, base_url: &str, key_env: &str, model: &str) -> Result<Value, String> {
    let valid = ["claude", "gemini", "openrouter", "openai", "ollama"];
    if !valid.contains(&slot) {
        return Err(format!("unknown provider slot '{slot}'"));
    }
    if model.trim().is_empty() {
        return Err("a model is required".into());
    }

    // Preserve safety/caps if the user set them.
    let max_tokens = config_value("caps", "max_tokens_per_session").unwrap_or_else(|| "200000".into());
    let max_cost = config_value("caps", "max_cost_usd").unwrap_or_else(|| "1.0".into());
    let max_steps = config_value("caps", "max_steps").unwrap_or_else(|| "25".into());
    let confirm = config_value("safety", "confirm_destructive").unwrap_or_else(|| "true".into());

    let mut out = String::new();
    out.push_str("# Slug configuration — managed by the dashboard.\n");
    out.push_str("# API keys are read from the env var named below and are NEVER stored here.\n\n");
    out.push_str(&format!("[brain]\nprovider = \"{slot}\"\n\n"));
    out.push_str(&format!("[providers.{slot}]\n"));
    if !key_env.trim().is_empty() {
        out.push_str(&format!("api_key_env = \"{}\"\n", key_env.trim()));
    }
    if !base_url.trim().is_empty() {
        out.push_str(&format!("base_url = \"{}\"\n", base_url.trim()));
    }
    out.push_str(&format!("model = \"{}\"\n\n", model.trim()));
    out.push_str(&format!(
        "[caps]\nmax_tokens_per_session = {max_tokens}\nmax_cost_usd = {max_cost}\nmax_steps = {max_steps}\n\n"
    ));
    out.push_str(&format!("[safety]\nconfirm_destructive = {confirm}\n"));

    let path = config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("mkdir: {e}"))?;
    }
    std::fs::write(&path, out).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(providers_status())
}

/// Whether an env var holds a (non-empty) value in this process.
pub fn key_present(env: &str) -> bool {
    std::env::var(env).map(|v| !v.trim().is_empty()).unwrap_or(false)
}

// ------------------------------ secret store -------------------------------
//
// API keys are persisted across restarts so you don't re-enter them — but **never
// in `slug.toml`**. They live in a dedicated `~/.slug/secrets.env` (`KEY=value`
// per line) created with owner-only `0600` permissions, and are loaded into the
// process environment at startup (so the brain/agent inherit them). The real
// environment always wins over the file.

fn secrets_path() -> PathBuf {
    std::env::var("SLUG_SECRETS").map(PathBuf::from).unwrap_or_else(|_| slug_home().join("secrets.env"))
}

fn load_secret_map() -> std::collections::BTreeMap<String, String> {
    let mut m = std::collections::BTreeMap::new();
    if let Ok(text) = std::fs::read_to_string(secrets_path()) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                m.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }
    m
}

fn write_secret_map(map: &std::collections::BTreeMap<String, String>) -> Result<(), String> {
    let path = secrets_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("mkdir: {e}"))?;
    }
    let body: String = map.iter().map(|(k, v)| format!("{k}={v}\n")).collect();

    // On Unix, create the file with owner-only (0600) permissions from the
    // first byte — no window where it's briefly world/group-readable, unlike
    // write-then-chmod.
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| format!("open {}: {e}", path.display()))?;
        f.write_all(body.as_bytes()).map_err(|e| format!("write {}: {e}", path.display()))?;
        // `mode()` only applies to a freshly created file; re-assert 0600 in
        // case this file pre-dates this fix and was left more permissive.
        let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&path, body).map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    Ok(())
}

/// Persist an API key for a provider env var, so it survives an app restart.
/// Stored in `~/.slug/secrets.env` (0600), **never** in `slug.toml`.
pub fn save_secret(env: &str, value: &str) -> Result<(), String> {
    let env = env.trim();
    if env.is_empty() || value.trim().is_empty() {
        return Err("env and value required".into());
    }
    let mut map = load_secret_map();
    map.insert(env.to_string(), value.trim().to_string());
    write_secret_map(&map)
}

/// Forget a previously-saved key.
pub fn forget_secret(env: &str) -> Result<(), String> {
    let mut map = load_secret_map();
    map.remove(env.trim());
    write_secret_map(&map)
}

/// Load saved secrets into the process environment at startup. The real
/// environment wins, so an explicitly-exported key is never overridden.
pub fn load_secrets_into_env() {
    for (k, v) in load_secret_map() {
        if std::env::var_os(&k).is_none() {
            std::env::set_var(&k, &v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests mutate the shared `SLUG_CONFIG` env var; serialize them.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn brain_info_defaults_when_no_config() {
        let _g = ENV_LOCK.lock().unwrap();
        // With a config path that does not exist, we still return a sane default.
        std::env::set_var("SLUG_CONFIG", "/nonexistent/slug.toml");
        let (p, m) = brain_info();
        assert_eq!(p, "claude");
        assert_eq!(m, "claude-sonnet-4-6");
        std::env::remove_var("SLUG_CONFIG");
    }

    #[test]
    fn brain_info_reads_provider_and_model() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("slugcfg{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("slug.toml");
        std::fs::write(
            &cfg,
            "[brain]\nprovider = \"ollama\"\n\n[providers.ollama]\nmodel = \"qwen3:14b\"\n",
        )
        .unwrap();
        std::env::set_var("SLUG_CONFIG", &cfg);
        let (p, m) = brain_info();
        assert_eq!(p, "ollama");
        assert_eq!(m, "qwen3:14b");
        assert!(provider_is_local(&p));
        std::env::remove_var("SLUG_CONFIG");
    }

    #[test]
    fn brain_ready_requires_a_key_for_cloud_but_not_local() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("slugready{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("slug.toml");

        // Cloud provider with no key in the env → not ready, with an actionable hint.
        std::fs::write(&cfg, "[brain]\nprovider = \"claude\"\n").unwrap();
        std::env::set_var("SLUG_CONFIG", &cfg);
        std::env::remove_var("ANTHROPIC_API_KEY");
        let err = brain_ready().unwrap_err();
        assert!(err.contains("ANTHROPIC_API_KEY"), "hint should name the env var: {err}");
        assert!(!brain_detail()["ready"].as_bool().unwrap());

        // …key present → ready.
        std::env::set_var("ANTHROPIC_API_KEY", "sk-test");
        assert!(brain_ready().is_ok());
        assert!(brain_detail()["ready"].as_bool().unwrap());
        std::env::remove_var("ANTHROPIC_API_KEY");

        // Local provider (ollama) is always ready — no key needed.
        std::fs::write(&cfg, "[brain]\nprovider = \"ollama\"\n").unwrap();
        assert!(brain_ready().is_ok());
        assert_eq!(brain_detail()["location"], "local");

        std::env::remove_var("SLUG_CONFIG");
    }

    #[test]
    fn provider_catalog_covers_the_majors() {
        let ids: Vec<&str> = provider_catalog().iter().map(|p| p.0).collect();
        for must in ["anthropic", "gemini", "openrouter", "openai", "groq", "mistral", "deepseek", "xai", "ollama"] {
            assert!(ids.contains(&must), "catalog missing {must}");
        }
    }

    #[test]
    fn set_provider_writes_config_without_keys() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("slugprov{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("slug.toml");
        std::env::set_var("SLUG_CONFIG", &cfg);

        // Activate Groq (rides the openai slot with a custom base_url).
        set_provider("openai", "https://api.groq.com/openai/v1", "GROQ_API_KEY", "llama-3.3-70b-versatile").unwrap();
        let written = std::fs::read_to_string(&cfg).unwrap();
        assert!(written.contains("provider = \"openai\""));
        assert!(written.contains("https://api.groq.com/openai/v1"));
        assert!(written.contains("api_key_env = \"GROQ_API_KEY\""));
        // The key VALUE must never be written, only the env var name.
        assert!(!written.contains("sk-"), "a key value must never be stored");

        let (p, m) = brain_info();
        assert_eq!(p, "openai");
        assert_eq!(m, "llama-3.3-70b-versatile");

        assert!(set_provider("bogus", "", "", "x").is_err());
        std::env::remove_var("SLUG_CONFIG");
    }

    #[test]
    fn secrets_persist_and_load_into_env() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("slugsec{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let secrets = dir.join("secrets.env");
        std::env::set_var("SLUG_SECRETS", &secrets);
        std::env::remove_var("SLUG_TEST_KEY");

        save_secret("SLUG_TEST_KEY", "sk-abc123").unwrap();
        // Persisted to the secrets file, never anywhere else.
        let body = std::fs::read_to_string(&secrets).unwrap();
        assert!(body.contains("SLUG_TEST_KEY=sk-abc123"));
        // 0600 on unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&secrets).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "secrets file must be owner-only");
        }
        // Loads into the env on startup (real env wins, but it's unset here).
        load_secrets_into_env();
        assert_eq!(std::env::var("SLUG_TEST_KEY").unwrap(), "sk-abc123");
        assert!(key_present("SLUG_TEST_KEY"));

        forget_secret("SLUG_TEST_KEY").unwrap();
        assert!(!std::fs::read_to_string(&secrets).unwrap().contains("SLUG_TEST_KEY"));

        std::env::remove_var("SLUG_TEST_KEY");
        std::env::remove_var("SLUG_SECRETS");
    }

    #[test]
    fn client_status_reflects_the_cross_process_heartbeat() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("slughb{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("SLUG_HEARTBEAT", dir.join("heartbeat.json"));

        // No client has ever connected (no heartbeat file yet).
        let s = client_status(60);
        assert_eq!(s["connected"], false);
        assert!(s["transport"].is_null());

        // A stdio client just connected — both transports must be able to
        // report their own activity into the same file (that's the whole
        // point: the HTTP daemon and the stdio process are separate OS
        // processes that otherwise share no state).
        record_heartbeat("stdio");
        let s = client_status(60);
        assert_eq!(s["connected"], true);
        assert_eq!(s["transport"], "stdio");

        // A 0-second freshness window means even a heartbeat from "just now"
        // reads as stale — exercises the staleness branch deterministically,
        // without sleeping.
        let s = client_status(0);
        assert_eq!(s["connected"], false);

        // The other transport overwrites the heartbeat — last writer wins,
        // matching the dashboard's "currently connected" semantics.
        record_heartbeat("http");
        assert_eq!(client_status(60)["transport"], "http");

        std::env::remove_var("SLUG_HEARTBEAT");
    }
}
