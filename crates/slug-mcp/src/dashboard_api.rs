//! Backend for the control dashboard: which brain (Claude vs local) is configured,
//! and a persisted list of MCP servers the user can add/remove and see the live
//! reachability of.
//!
//! Dependency-free on purpose (std + serde_json only): the config is read with a
//! tiny hand parser so `slug-mcp` needs no TOML crate, and reachability is a plain
//! TCP connect (no HTTP client).

use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

/// `~/.slug` (honoured for both the config and the MCP-server list).
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

fn servers_path() -> PathBuf {
    std::env::var("SLUG_MCP_SERVERS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| slug_home().join("mcp_servers.json"))
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

// ----------------------------- MCP server list -----------------------------

fn load_servers() -> Vec<Value> {
    let text = std::fs::read_to_string(servers_path()).unwrap_or_default();
    serde_json::from_str::<Vec<Value>>(&text).unwrap_or_default()
}

fn save_servers(servers: &[Value]) -> std::io::Result<()> {
    let path = servers_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::File::create(&path)?;
    f.write_all(serde_json::to_string_pretty(servers).unwrap_or_default().as_bytes())
}

/// Parse `host:port` out of a `url` for a TCP reachability probe. Handles
/// `http(s)://host[:port]/path`; returns `None` for non-network entries (e.g. a
/// `stdio:`/command server, which can't be TCP-probed).
fn host_port(url: &str) -> Option<(String, u16)> {
    let u = url.trim();
    let (scheme, rest) = match u.split_once("://") {
        Some((s, r)) => (s, r),
        None => return None,
    };
    if scheme == "stdio" || scheme == "cmd" {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let default_port = if scheme == "https" { 443 } else { 80 };
    if let Some((h, p)) = authority.rsplit_once(':') {
        let host = h.trim_matches(['[', ']']);
        let port = p.parse().unwrap_or(default_port);
        Some((host.to_string(), port))
    } else {
        Some((authority.trim_matches(['[', ']']).to_string(), default_port))
    }
}

/// TCP-connect to `host:port` with a short timeout → reachable.
fn reachable(host: &str, port: u16) -> bool {
    let Ok(mut addrs) = (host, port).to_socket_addrs() else { return false };
    addrs.any(|a| TcpStream::connect_timeout(&a, Duration::from_millis(300)).is_ok())
}

/// The MCP servers list with live reachability, for the dashboard. Always
/// includes this Slug server itself first.
pub fn list_servers(self_url: &str) -> Value {
    let mut out = vec![json!({
        "name": "slug (this app)",
        "url": self_url,
        "kind": "self",
        "status": "serving",
    })];
    for s in load_servers() {
        let name = s.get("name").and_then(Value::as_str).unwrap_or("").to_string();
        let url = s.get("url").and_then(Value::as_str).unwrap_or("").to_string();
        let status = match host_port(&url) {
            Some((h, p)) => {
                if reachable(&h, p) {
                    "reachable"
                } else {
                    "unreachable"
                }
            }
            None => "command", // stdio/command server — can't TCP-probe
        };
        out.push(json!({ "name": name, "url": url, "kind": "custom", "status": status }));
    }
    json!({ "servers": out })
}

/// Add (or replace by name) a custom MCP server. Returns the refreshed list.
pub fn add_server(name: &str, url: &str, self_url: &str) -> Result<Value, String> {
    let name = name.trim();
    let url = url.trim();
    if name.is_empty() || url.is_empty() {
        return Err("both 'name' and 'url' are required".into());
    }
    let mut servers = load_servers();
    servers.retain(|s| s.get("name").and_then(Value::as_str) != Some(name));
    servers.push(json!({ "name": name, "url": url }));
    save_servers(&servers).map_err(|e| format!("could not save: {e}"))?;
    Ok(list_servers(self_url))
}

/// Remove a custom MCP server by name. Returns the refreshed list.
pub fn remove_server(name: &str, self_url: &str) -> Result<Value, String> {
    let mut servers = load_servers();
    let before = servers.len();
    servers.retain(|s| s.get("name").and_then(Value::as_str) != Some(name));
    if servers.len() == before {
        return Err(format!("no MCP server named '{name}'"));
    }
    save_servers(&servers).map_err(|e| format!("could not save: {e}"))?;
    Ok(list_servers(self_url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_port_parsing() {
        assert_eq!(host_port("http://127.0.0.1:9000/mcp"), Some(("127.0.0.1".into(), 9000)));
        assert_eq!(host_port("https://example.com/mcp"), Some(("example.com".into(), 443)));
        assert_eq!(host_port("http://localhost"), Some(("localhost".into(), 80)));
        assert_eq!(host_port("stdio:///usr/bin/foo"), None);
    }

    #[test]
    fn brain_info_defaults_when_no_config() {
        // With a config path that does not exist, we still return a sane default.
        std::env::set_var("SLUG_CONFIG", "/nonexistent/slug.toml");
        let (p, m) = brain_info();
        assert_eq!(p, "claude");
        assert_eq!(m, "claude-sonnet-4-6");
        std::env::remove_var("SLUG_CONFIG");
    }

    #[test]
    fn brain_info_reads_provider_and_model() {
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
    fn add_list_remove_servers_roundtrip() {
        let dir = std::env::temp_dir().join(format!("slugmcp{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("SLUG_MCP_SERVERS", dir.join("mcp_servers.json"));

        add_server("github", "http://127.0.0.1:65000/mcp", "http://127.0.0.1:7333/mcp").unwrap();
        let listed = list_servers("http://127.0.0.1:7333/mcp");
        let arr = listed["servers"].as_array().unwrap();
        assert_eq!(arr[0]["kind"], "self");
        assert!(arr.iter().any(|s| s["name"] == "github"));

        remove_server("github", "http://127.0.0.1:7333/mcp").unwrap();
        let listed = list_servers("http://127.0.0.1:7333/mcp");
        assert!(!listed["servers"].as_array().unwrap().iter().any(|s| s["name"] == "github"));
        assert!(remove_server("ghost", "x").is_err());

        std::env::remove_var("SLUG_MCP_SERVERS");
    }
}
