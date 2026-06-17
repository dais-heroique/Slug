# Slug OS — Design & Specification Dossier

**Version:** 0.1-draft  
**Date:** 2026-06-16  
**Status:** Engineering input — every decision is final unless annotated `[OPEN]`

-----

## What is Slug?

Slug is a Linux operating system whose primary user is an AI agent. Instead of perceiving the screen via screenshots, the agent reads a mandatory, OS-wide **semantic UI layer** — a structured, typed, delta-compressed representation of every widget on every surface, delivered in real time by a Wayland compositor extension. A local 7–14 B text LLM (Qwen3 family, quantised to fit available VRAM) drives native applications through this layer. Vision (screenshot) is a last-resort fallback, not a first-class path.

Slug is not a wrapper around a desktop. It is a ground-up OS design whose entire compositor, session daemon, and application toolkit are built around the assumption that the primary consumer of UI state is a machine, not a human.

-----

## Document Map

|#|Document                                    |One-line summary                                                                |
|-|--------------------------------------------|--------------------------------------------------------------------------------|
|1|[SEMANTIC-SCHEMA.md](./SEMANTIC-SCHEMA.md)  |SlugDoc node schema, role/state enum, stable-ref scheme, delta/event model      |
|2|[WAYLAND-PROTOCOL.md](./WAYLAND-PROTOCOL.md)|`slug_semantic_v1` Wayland protocol XML + rationale, Newton architecture summary|
|3|[MCP-TOOL-CATALOG.md](./MCP-TOOL-CATALOG.md)|Full MCP tool list with JSON-Schema for each tool                               |
|4|[PRIOR-ART-MATRIX.md](./PRIOR-ART-MATRIX.md)|Competitive analysis vs 9 prior systems; novelty claims; ASPLOS 2026 note       |
|5|[HARDWARE-TIERING.md](./HARDWARE-TIERING.md)|VRAM/RAM → backend decision table; Qwen3 quant sizes                            |
|6|[RISK-REGISTER.md](./RISK-REGISTER.md)      |Security threat model, prompt injection, sandboxing approach                    |

-----

## Core Design Axioms

These axioms govern every document below. Changing an axiom requires a full dossier revision.

**A1 — Text-first perception.** The agent receives a typed semantic tree, not pixels. Screenshots are emitted only when `role=CANVAS` or `role=MEDIA` nodes are encountered, or when the agent explicitly invokes `screenshot_region`.

**A2 — Push, not poll.** The compositor pushes semantic frame deltas to a session daemon after each committed Wayland frame. The agent never polls.

**A3 — Stable references survive redraws.** Every interactable node carries a `slug_ref` (128-bit ULID) that is stable across redraws, scroll, and theme changes. Refs are recycled only on explicit widget destruction.

**A4 — Local-first inference.** The primary inference backend is Ollama running a quantised Qwen3 model on the local GPU. The Claude API is a tiered fallback for hardware below the minimum threshold, or for multi-step reasoning tasks that exceed local context.

**A5 — Sandboxed agent actions.** Every agent action goes through a capability-gated MCP tool. No agent code runs with raw X11/Wayland socket access. The session daemon is the single action bottleneck.

**A6 — AT-SPI2 compatibility is a floor, not a ceiling.** Slug maps all AT-SPI2 roles and states but is not constrained to AT-SPI2’s information model. Slug nodes carry richer semantic metadata (value ranges, enumerated option lists, validation state) that AT-SPI2 cannot express.

-----

## Relationship to Newton (GNOME)

GNOME’s Newton project (2024–present) is the closest prior art in the Wayland compositor space. Newton also uses a push-based, frame-synchronised semantic tree delivered over a Wayland protocol extension, with D-Bus bridging for AT clients. Slug **forks Newton’s architecture** for the compositor layer and **extends it** with:

- A richer node schema (see Doc 1)
- An MCP server layer that exposes semantic tree + actions to LLM agents (see Doc 3)
- A hardware tiering policy for local LLM selection (see Doc 5)
- A security model designed for autonomous (not just assistive) agents (see Doc 6)

Where Newton diverges from Slug’s requirements, Doc 2 explains the specific design departure.

-----

## Engineering Milestone Sequence

```
M0  Protocol freeze        slug_semantic_v1 XML + node schema locked
M1  Compositor shim        wlroots plugin emitting semantic frames
M2  Session daemon         delta engine + AT-SPI2 bridge + D-Bus API
M3  MCP server             all tools in Doc 3 passing conformance tests
M4  LLM integration        Qwen3-8B-Q4_K_M loop on synthetic desktop
M5  Security hardening     sandbox, capability ledger, kill-switch
M6  Hardware tiering       automatic backend selection at boot
```

Each document below maps to one or more milestones.