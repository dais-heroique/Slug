# Doc 5 — Hardware Tiering & Local-vs-Cloud Policy

**Milestone:** M6  
**Status:** Final

-----

## 1. Design Decision

Slug selects its inference backend **automatically at boot** based on detected hardware. The agent does not choose its backend; the OS chooses it. The agent receives a `SLUG_BACKEND` environment variable (`local`, `hybrid`, or `cloud`) and an `SLUG_MODEL_ID` string indicating the active model.

The decision table is a static policy file (`/etc/slug/hardware-policy.toml`) evaluated by the `slug-backend-selector` systemd service at session start.

-----

## 2. Qwen3 VRAM Requirements by Quantisation

Qwen3 is the recommended primary model family because it offers strong instruction-following at 7–14B parameters with efficient quantisation. The following figures are measured on an A10 (24GB) and RTX 3090 (24GB) with Ollama 0.4+ using llama.cpp backend.

### Qwen3-8B (7.6B active parameters)

|Quant   |File Size|VRAM (inference)|VRAM (KV cache, 8K ctx)|Total VRAM  |Notes                                   |
|--------|---------|----------------|-----------------------|------------|----------------------------------------|
|`Q2_K`  |3.2 GB   |~3.8 GB         |~0.4 GB                |**~4.2 GB** |Lowest quality; avoid for tool-use tasks|
|`Q3_K_M`|4.1 GB   |~4.8 GB         |~0.4 GB                |**~5.2 GB** |Acceptable for simple navigation        |
|`Q4_K_M`|5.1 GB   |~6.0 GB         |~0.5 GB                |**~6.5 GB** |**Recommended baseline**                |
|`Q5_K_M`|6.1 GB   |~7.0 GB         |~0.5 GB                |**~7.5 GB** |Better tool-use accuracy                |
|`Q6_K`  |7.1 GB   |~8.2 GB         |~0.6 GB                |**~8.8 GB** |Near-full quality                       |
|`Q8_0`  |9.5 GB   |~10.8 GB        |~0.6 GB                |**~11.4 GB**|Full INT8 quality                       |
|`F16`   |16.0 GB  |~18.0 GB        |~0.8 GB                |**~18.8 GB**|Full float16; only for 24GB+            |

### Qwen3-14B (14.8B active parameters)

|Quant   |File Size|VRAM (inference)|VRAM (KV cache, 8K ctx)|Total VRAM  |Notes                        |
|--------|---------|----------------|-----------------------|------------|-----------------------------|
|`Q3_K_M`|7.8 GB   |~9.0 GB         |~0.8 GB                |**~9.8 GB** |Minimum viable for 14B       |
|`Q4_K_M`|9.6 GB   |~11.2 GB        |~0.9 GB                |**~12.1 GB**|**Recommended for 16GB VRAM**|
|`Q5_K_M`|11.5 GB  |~13.2 GB        |~1.0 GB                |**~14.2 GB**|Better reasoning             |
|`Q6_K`  |13.4 GB  |~15.4 GB        |~1.1 GB                |**~16.5 GB**|16GB VRAM GPU at limit       |
|`Q8_0`  |17.8 GB  |~20.4 GB        |~1.2 GB                |**~21.6 GB**|Requires 24GB VRAM           |
|`F16`   |29.6 GB  |~33.6 GB        |~1.6 GB                |**~35.2 GB**|Requires 40GB+ VRAM          |

### Qwen3-32B (MoE, 32B total / ~3B active per token)

|Quant   |File Size|VRAM (inference, full load)|Notes                            |
|--------|---------|---------------------------|---------------------------------|
|`Q4_K_M`|20.0 GB  |**~23 GB**                 |Fits in 24GB VRAM; strong quality|
|`Q3_K_M`|15.8 GB  |**~18 GB**                 |Fits in 20GB VRAM                |

*Note: Qwen3-32B is a Mixture-of-Experts model. “Full load” means all experts resident in VRAM. Active-expert-only inference requires 24GB but keeps all experts in VRAM to avoid paging.*

-----

## 3. Hardware Decision Table

The selector evaluates conditions top-to-bottom and applies the **first matching tier**.

|Tier                     |VRAM                |RAM       |CPU     |Decision                     |Model                              |Quant |Notes                                               |
|-------------------------|--------------------|----------|--------|-----------------------------|-----------------------------------|------|----------------------------------------------------|
|**A** — Performance local|≥24 GB              |≥32 GB    |Any     |**Ollama local**             |Qwen3-14B                          |Q6_K  |Full-quality local inference                        |
|**B** — Standard local   |≥12 GB              |≥16 GB    |Any     |**Ollama local**             |Qwen3-14B                          |Q4_K_M|Good quality, fits 12–16GB VRAM                     |
|**C** — Budget local (8B)|≥8 GB               |≥16 GB    |Any     |**Ollama local**             |Qwen3-8B                           |Q5_K_M|Recommended for 8–11GB VRAM                         |
|**D** — Minimum local    |≥6 GB               |≥12 GB    |Any     |**Ollama local**             |Qwen3-8B                           |Q4_K_M|Minimum viable local tier                           |
|**E** — CPU-only         |<6 GB VRAM or no GPU|≥32 GB RAM|≥8 cores|**Ollama local (CPU)**       |Qwen3-8B                           |Q4_K_M|~3–5 tokens/sec; acceptable for low-frequency tasks |
|**F** — Cloud fallback   |<6 GB VRAM          |<32 GB RAM|Any     |**Claude API**               |claude-sonnet-4-6                  |N/A   |Network required; usage metered                     |
|**G** — Hybrid           |≥6 GB VRAM          |≥16 GB RAM|Any     |**Ollama local + Claude API**|Qwen3-8B Q4_K_M + claude-sonnet-4-6|—     |Local for navigation; cloud for multi-step reasoning|

### Tier G — Hybrid mode logic

In hybrid mode, the session daemon routes requests based on a task complexity heuristic:

- **Local (Qwen3-8B Q4_K_M):** Single-step tool calls, tree navigation, simple form filling, state queries
- **Cloud (Claude API):** Multi-step plans requiring >3 tool calls, tasks requiring code generation, tasks requiring extended reasoning

The threshold is configurable in `/etc/slug/backend-policy.toml`:

```toml
[hybrid]
local_max_steps = 3
local_max_context_tokens = 4096
cloud_model = "claude-sonnet-4-6"
```

-----

## 4. Backend Configuration File

`/etc/slug/backend-policy.toml` (system-wide defaults, overrideable per-user):

```toml
[detection]
vram_minimum_gb = 6.0          # Below this, no GPU inference
ram_minimum_gb = 12.0          # Below this, CPU inference not attempted
cpu_cores_minimum = 4

[local]
ollama_host = "http://127.0.0.1:11434"
default_context_length = 8192  # tokens
request_timeout_ms = 120000    # 2 minutes for slow GPU

[cloud]
provider = "anthropic"         # "anthropic" | "openai" | "custom"
model = "claude-sonnet-4-6"
api_key_env = "SLUG_ANTHROPIC_API_KEY"
max_tokens = 4096

[hybrid]
enabled = true
local_max_steps = 3
local_max_context_tokens = 4096
fallback_on_local_error = true

[tiers]
# Override automatic tier selection. Remove to use auto-detection.
# force_tier = "B"
```

-----

## 5. Model Download Policy

At first boot, `slug-backend-selector` determines the tier and runs:

```
slug-model-pull --tier <A|B|C|D|E>
```

This calls `ollama pull qwen3:<tag>` with the appropriate quant tag. The pull is mandatory before the first agent session; the session daemon will not start without a local model in tiers A–E.

Model storage: `/var/lib/slug/models/` (system-wide, shared across users).

-----

## 6. Performance Expectations

Rough tokens/second benchmarks on representative hardware:

|Hardware                   |Model    |Quant |Tokens/sec (generation)|
|---------------------------|---------|------|-----------------------|
|RTX 4090 (24GB)            |Qwen3-14B|Q6_K  |~55 t/s                |
|RTX 3090 (24GB)            |Qwen3-14B|Q4_K_M|~40 t/s                |
|RTX 3080 (10GB)            |Qwen3-8B |Q5_K_M|~48 t/s                |
|RTX 3070 (8GB)             |Qwen3-8B |Q4_K_M|~42 t/s                |
|RTX 3060 (8GB)             |Qwen3-8B |Q4_K_M|~35 t/s                |
|RX 7900 XTX (24GB)         |Qwen3-14B|Q4_K_M|~38 t/s (ROCm)         |
|Apple M3 Pro (18GB unified)|Qwen3-8B |Q6_K  |~45 t/s                |
|CPU only (Ryzen 7 7700X)   |Qwen3-8B |Q4_K_M|~4 t/s                 |

At 40 t/s with 500-token responses, a typical agent action takes ~12 seconds end-to-end (tree snapshot + LLM inference + action dispatch). This is acceptable for an agent OS; interactive human latency is not a constraint.

-----

## 7. Vision Model (Last-Resort Fallback)

When the agent invokes `screenshot_region` or `screenshot_surface`, a separate vision-capable model is required. Decision:

|Tier              |Vision Backend                                              |
|------------------|------------------------------------------------------------|
|A, B (≥24GB VRAM) |Qwen3-VL-7B-Instruct (local, Q4_K_M, ~6.5GB additional VRAM)|
|C, D (8–12GB VRAM)|Claude API (claude-sonnet-4-6 with vision)                  |
|E (CPU), F (cloud)|Claude API                                                  |

Vision model use is logged in the audit trail (see Doc 6). The session daemon tracks vision call frequency; if it exceeds 20% of all actions, a warning is emitted suggesting the application vendor implement semantic accessibility.