//! Hardware probing and capability tiering.
//!
//! Detects total VRAM (NVIDIA via NVML, AMD via sysfs, Apple via `system_profiler`),
//! system RAM, and CPU cores, then maps them to a [`CapabilityTier`] and a
//! recommended backend/model configuration. The decision boundaries follow the
//! task brief's 4-tier scheme; see also `docs/HARDWARE-TIERING.md` (Doc 5), whose
//! A–G policy this consolidates. The tier→model mapping is overridable in
//! `slug.toml`.

use std::fmt;

use serde::Serialize;

/// A source of hardware facts. Real detection uses [`SystemProbe`]; tests use a
/// mock implementing this trait.
pub trait Probe {
    /// Total dedicated/unified GPU memory in mebibytes (0 if no GPU).
    fn vram_mb(&self) -> u64;
    /// Total system RAM in mebibytes.
    fn ram_mb(&self) -> u64;
    /// Logical CPU cores.
    fn cpu_cores(&self) -> usize;
    /// Human-readable GPU name(s) detected, for the report.
    fn gpu_names(&self) -> Vec<String> {
        Vec::new()
    }
}

/// The capability tier selected from detected hardware.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CapabilityTier {
    /// < 8 GB VRAM (or no GPU): recommend the Claude API.
    TierCloud,
    /// 8–11 GB VRAM: Qwen3 8B (Q4) locally.
    TierLocalSmall,
    /// 12–23 GB VRAM: Qwen3 14B (Q4_K_M) locally.
    TierLocalStd,
    /// ≥ 24 GB VRAM: Qwen3 32B / 30B-A3B MoE locally.
    TierLocalLarge,
}

impl CapabilityTier {
    /// Classify a tier purely from VRAM, in gibibytes — the dominant constraint.
    pub fn from_vram_gb(vram_gb: f64) -> CapabilityTier {
        if vram_gb < 8.0 {
            CapabilityTier::TierCloud
        } else if vram_gb < 12.0 {
            CapabilityTier::TierLocalSmall
        } else if vram_gb < 24.0 {
            CapabilityTier::TierLocalStd
        } else {
            CapabilityTier::TierLocalLarge
        }
    }

    /// The default inference backend for this tier.
    pub fn backend(&self) -> BackendKind {
        match self {
            CapabilityTier::TierCloud => BackendKind::Cloud,
            _ => BackendKind::Local,
        }
    }

    /// The default model id for this tier (Ollama tag, or Claude model for cloud).
    pub fn default_model(&self) -> &'static str {
        match self {
            CapabilityTier::TierCloud => "claude-sonnet-4-6",
            CapabilityTier::TierLocalSmall => "qwen3:8b",
            CapabilityTier::TierLocalStd => "qwen3:14b",
            CapabilityTier::TierLocalLarge => "qwen3:32b",
        }
    }

    /// The recommended quantisation (empty for the cloud tier).
    pub fn default_quant(&self) -> &'static str {
        match self {
            CapabilityTier::TierCloud => "",
            CapabilityTier::TierLocalSmall => "Q4_K_M",
            CapabilityTier::TierLocalStd => "Q4_K_M",
            CapabilityTier::TierLocalLarge => "Q4_K_M",
        }
    }

    /// One-line human description.
    pub fn summary(&self) -> &'static str {
        match self {
            CapabilityTier::TierCloud => "<8GB VRAM — recommend Claude API",
            CapabilityTier::TierLocalSmall => "8–11GB VRAM — Qwen3 8B (Q4) local",
            CapabilityTier::TierLocalStd => "12–23GB VRAM — Qwen3 14B (Q4_K_M) local",
            CapabilityTier::TierLocalLarge => "≥24GB VRAM — Qwen3 32B / 30B-A3B MoE local",
        }
    }
}

/// Which backend to drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    Local,
    Cloud,
}

/// The full assessment: detected facts + selected tier + recommendation.
#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub vram_mb: u64,
    pub ram_mb: u64,
    pub cpu_cores: usize,
    pub gpu_names: Vec<String>,
    pub tier: CapabilityTier,
    pub backend: BackendKind,
    pub model: String,
    pub quant: String,
    /// Whether RAM is also sufficient for the recommended local model (warns if not).
    pub ram_ok: bool,
}

impl Report {
    fn vram_gb(&self) -> f64 {
        self.vram_mb as f64 / 1024.0
    }
    fn ram_gb(&self) -> f64 {
        self.ram_mb as f64 / 1024.0
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Slug — Can I run it?")?;
        writeln!(f, "====================")?;
        let gpu = if self.gpu_names.is_empty() {
            "(none detected)".to_string()
        } else {
            self.gpu_names.join(", ")
        };
        writeln!(f, "GPU:        {gpu}")?;
        writeln!(f, "VRAM:       {:.1} GB", self.vram_gb())?;
        writeln!(f, "RAM:        {:.1} GB", self.ram_gb())?;
        writeln!(f, "CPU cores:  {}", self.cpu_cores)?;
        writeln!(f)?;
        writeln!(f, "Tier:       {:?} — {}", self.tier, self.tier.summary())?;
        match self.backend {
            BackendKind::Cloud => {
                writeln!(f, "Backend:    cloud")?;
                writeln!(f, "Provider:   claude (default) — or set [brain] provider = openai | openrouter | gemini")?;
                writeln!(f, "Model:      {}", self.model)?;
                writeln!(f, "Note:       set the provider's API key env var (e.g. ANTHROPIC_API_KEY) to use it.")?;
            }
            BackendKind::Local => {
                writeln!(f, "Backend:    local")?;
                writeln!(f, "Provider:   ollama")?;
                writeln!(f, "Model:      {} [{}]", self.model, self.quant)?;
                writeln!(f, "Pull with:  ollama pull {}", self.model)?;
                if !self.ram_ok {
                    writeln!(
                        f,
                        "WARNING:    RAM is below the recommended 16 GB for this tier — \
                         expect swapping or fall back to cloud."
                    )?;
                }
            }
        }
        Ok(())
    }
}

/// Assess hardware via a [`Probe`] and produce a [`Report`].
pub fn assess(probe: &dyn Probe) -> Report {
    let vram_mb = probe.vram_mb();
    let ram_mb = probe.ram_mb();
    let cpu_cores = probe.cpu_cores();
    let tier = CapabilityTier::from_vram_gb(vram_mb as f64 / 1024.0);
    // Local tiers want at least ~16 GB RAM (Doc 5 tiers B/C); small/minimum want 12+.
    let ram_ok = match tier {
        CapabilityTier::TierCloud => true,
        _ => ram_mb as f64 / 1024.0 >= 15.0,
    };
    Report {
        vram_mb,
        ram_mb,
        cpu_cores,
        gpu_names: probe.gpu_names(),
        tier,
        backend: tier.backend(),
        model: tier.default_model().to_string(),
        quant: tier.default_quant().to_string(),
        ram_ok,
    }
}

/// The real probe: NVML → AMD sysfs → Apple, plus `sysinfo` for RAM/CPU.
pub struct SystemProbe {
    vram_mb: u64,
    ram_mb: u64,
    cpu_cores: usize,
    gpu_names: Vec<String>,
}

impl SystemProbe {
    /// Detect hardware now. Detection is best-effort: a failure in any probe
    /// degrades to 0 VRAM (which selects the cloud tier).
    pub fn detect() -> SystemProbe {
        let (vram_mb, gpu_names) = detect_gpu();
        let (ram_mb, cpu_cores) = detect_ram_cpu();
        SystemProbe { vram_mb, ram_mb, cpu_cores, gpu_names }
    }
}

impl Probe for SystemProbe {
    fn vram_mb(&self) -> u64 {
        self.vram_mb
    }
    fn ram_mb(&self) -> u64 {
        self.ram_mb
    }
    fn cpu_cores(&self) -> usize {
        self.cpu_cores
    }
    fn gpu_names(&self) -> Vec<String> {
        self.gpu_names.clone()
    }
}

fn detect_ram_cpu() -> (u64, usize) {
    use sysinfo::System;
    let mut sys = System::new();
    sys.refresh_memory();
    let ram_mb = sys.total_memory() / (1024 * 1024); // total_memory() is bytes
    let cpu_cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    (ram_mb, cpu_cores)
}

/// Detect total VRAM (MB) and GPU names. Sums across NVIDIA GPUs; falls back to
/// AMD sysfs, then Apple unified memory.
fn detect_gpu() -> (u64, Vec<String>) {
    if let Some(r) = detect_nvidia() {
        return r;
    }
    if let Some(r) = detect_amd_sysfs() {
        return r;
    }
    #[cfg(target_os = "macos")]
    if let Some(r) = detect_apple() {
        return r;
    }
    (0, Vec::new())
}

fn detect_nvidia() -> Option<(u64, Vec<String>)> {
    use nvml_wrapper::Nvml;
    let nvml = Nvml::init().ok()?;
    let count = nvml.device_count().ok()?;
    if count == 0 {
        return None;
    }
    let mut total_mb = 0u64;
    let mut names = Vec::new();
    for i in 0..count {
        if let Ok(device) = nvml.device_by_index(i) {
            if let Ok(mem) = device.memory_info() {
                total_mb += mem.total / (1024 * 1024);
            }
            if let Ok(name) = device.name() {
                names.push(name);
            }
        }
    }
    if total_mb == 0 {
        None
    } else {
        Some((total_mb, names))
    }
}

/// AMD: read `mem_info_vram_total` (bytes) from each render node under sysfs.
fn detect_amd_sysfs() -> Option<(u64, Vec<String>)> {
    let mut total_mb = 0u64;
    let mut names = Vec::new();
    let entries = std::fs::read_dir("/sys/class/drm").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // Match cardN (not cardN-eDP-1 connector subdirs).
        if !(name.starts_with("card") && name[4..].chars().all(|c| c.is_ascii_digit()) && name.len() > 4)
        {
            continue;
        }
        let vram_path = entry.path().join("device/mem_info_vram_total");
        if let Ok(contents) = std::fs::read_to_string(&vram_path) {
            if let Ok(bytes) = contents.trim().parse::<u64>() {
                total_mb += bytes / (1024 * 1024);
                names.push(format!("AMD GPU ({name})"));
            }
        }
    }
    if total_mb == 0 {
        None
    } else {
        Some((total_mb, names))
    }
}

/// Apple: parse `system_profiler SPDisplaysDataType` for VRAM / unified memory.
#[cfg(target_os = "macos")]
fn detect_apple() -> Option<(u64, Vec<String>)> {
    use std::process::Command;
    let out = Command::new("system_profiler").arg("SPDisplaysDataType").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut names = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("Chipset Model:") {
            names.push(rest.trim().to_string());
        }
    }
    // On Apple Silicon the GPU shares system RAM (unified memory). Approximate
    // usable VRAM as ~70% of system RAM.
    let (ram_mb, _) = detect_ram_cpu();
    let vram_mb = (ram_mb as f64 * 0.70) as u64;
    if vram_mb == 0 {
        None
    } else {
        Some((vram_mb, names))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mock probe so tiering logic can be tested without real hardware.
    struct MockProbe {
        vram_mb: u64,
        ram_mb: u64,
        cpu_cores: usize,
    }

    impl Probe for MockProbe {
        fn vram_mb(&self) -> u64 {
            self.vram_mb
        }
        fn ram_mb(&self) -> u64 {
            self.ram_mb
        }
        fn cpu_cores(&self) -> usize {
            self.cpu_cores
        }
    }

    fn gb(n: f64) -> u64 {
        (n * 1024.0) as u64
    }

    #[test]
    fn tier_boundaries() {
        assert_eq!(CapabilityTier::from_vram_gb(0.0), CapabilityTier::TierCloud);
        assert_eq!(CapabilityTier::from_vram_gb(7.9), CapabilityTier::TierCloud);
        assert_eq!(CapabilityTier::from_vram_gb(8.0), CapabilityTier::TierLocalSmall);
        assert_eq!(CapabilityTier::from_vram_gb(11.0), CapabilityTier::TierLocalSmall);
        assert_eq!(CapabilityTier::from_vram_gb(12.0), CapabilityTier::TierLocalStd);
        assert_eq!(CapabilityTier::from_vram_gb(23.0), CapabilityTier::TierLocalStd);
        assert_eq!(CapabilityTier::from_vram_gb(24.0), CapabilityTier::TierLocalLarge);
        assert_eq!(CapabilityTier::from_vram_gb(48.0), CapabilityTier::TierLocalLarge);
    }

    #[test]
    fn no_gpu_is_cloud() {
        let p = MockProbe { vram_mb: 0, ram_mb: gb(64.0), cpu_cores: 16 };
        let r = assess(&p);
        assert_eq!(r.tier, CapabilityTier::TierCloud);
        assert_eq!(r.backend, BackendKind::Cloud);
        assert_eq!(r.model, "claude-sonnet-4-6");
    }

    #[test]
    fn rtx_3060_8gb_is_small() {
        let p = MockProbe { vram_mb: gb(8.0), ram_mb: gb(32.0), cpu_cores: 8 };
        let r = assess(&p);
        assert_eq!(r.tier, CapabilityTier::TierLocalSmall);
        assert_eq!(r.backend, BackendKind::Local);
        assert_eq!(r.model, "qwen3:8b");
        assert_eq!(r.quant, "Q4_K_M");
        assert!(r.ram_ok);
    }

    #[test]
    fn rtx_4070_16gb_is_std() {
        let p = MockProbe { vram_mb: gb(16.0), ram_mb: gb(32.0), cpu_cores: 12 };
        let r = assess(&p);
        assert_eq!(r.tier, CapabilityTier::TierLocalStd);
        assert_eq!(r.model, "qwen3:14b");
    }

    #[test]
    fn rtx_4090_24gb_is_large() {
        let p = MockProbe { vram_mb: gb(24.0), ram_mb: gb(64.0), cpu_cores: 24 };
        let r = assess(&p);
        assert_eq!(r.tier, CapabilityTier::TierLocalLarge);
        assert_eq!(r.model, "qwen3:32b");
    }

    #[test]
    fn local_tier_with_low_ram_warns() {
        let p = MockProbe { vram_mb: gb(12.0), ram_mb: gb(8.0), cpu_cores: 8 };
        let r = assess(&p);
        assert_eq!(r.tier, CapabilityTier::TierLocalStd);
        assert!(!r.ram_ok, "8GB RAM should fail the local RAM check");
    }

    #[test]
    fn report_renders_without_panicking() {
        let p = MockProbe { vram_mb: gb(24.0), ram_mb: gb(64.0), cpu_cores: 24 };
        let text = format!("{}", assess(&p));
        assert!(text.contains("Can I run it?"));
        assert!(text.contains("qwen3:32b"));
    }
}
