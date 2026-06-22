# Screenshot/vision vs Slug — playing chess.com

A grounded comparison of perceiving and playing a game on **chess.com** two ways:

- **Vision** (e.g. Claude Computer Use): take a **screenshot**, let the model read
  the pixels and return click coordinates.
- **Slug**: read the OS **accessibility tree as text**, narrowed server-side, and
  click the board by computed coordinates.

Everything below is **reproducible** — the Slug sizes are measured by this repo's
own serializer; the vision sizes use Anthropic's published image-token formula:

```sh
cargo test -p slug-core --test snapshot_vs_vision -- --nocapture report
```

## What is measured vs assumed

| Measured (from the code) | Assumed (sourced constant) |
|--------------------------|----------------------------|
| Size of a full `slug_snapshot` of a realistic ~1,600-node page | Screenshot **1280×800** → **1,365 image tokens** = `(w·h)/750` (Anthropic) |
| Size of a **filtered** snapshot (move list only) | ~**4 chars/token** for text (standard heuristic) |
| | **$3 / 1M input tokens** (Claude Sonnet, published; image & text input billed alike) |
| | **2 screenshots / turn** (see board to move, then to verify) — also shown at 1 |
| | Game length **40 moves/side** |

The page is synthetic but realistic: ~1,600 accessibility nodes (a real complex
web app), site chrome as links/buttons/containers, the board as an opaque
`Canvas`, and the moves as `static_text`.

## Measured snapshot sizes

| Game point | Full page snapshot | Filtered move list |
|-----------|--------------------|--------------------|
| move 1  | 1,525 nodes · 100,215 chars (~**25,054 tok**) | 746 chars (~**186 tok**) |
| move 10 | 1,543 nodes · 100,909 chars (~25,227 tok) | 1,332 chars (~333 tok) |
| move 20 | 1,563 nodes · 101,689 chars (~25,422 tok) | 1,992 chars (~498 tok) |
| move 40 | 1,603 nodes · 103,249 chars (~25,812 tok) | 3,312 chars (~**828 tok**) |

> **One screenshot = 1,365 tokens — more than reading the entire 40-move game as
> text (828 tokens).**

## Per-turn input tokens

| Per turn | move 1 | move 40 |
|----------|--------|---------|
| Vision (2 screenshots) | 2,931 | 2,931 |
| Slug — filtered move list | **386** | **1,028** |
| Slug — *naïve* full snapshot | 25,254 | 26,012 |

Slug needs **0** perception tokens to *make* a move (the board is a canvas — it's
two clicks at computed coordinates); it only spends tokens to *read the reply*.

## Whole game (40 moves)

| Approach | Input tokens | Cost @ $3/Mtok |
|----------|-------------:|---------------:|
| Vision — 2 shots/turn | 117,227 | **$0.352** |
| Vision — 1 shot/turn | 62,600 | $0.188 |
| **Slug — filtered** | **28,259** | **$0.085** |
| Slug — naïve full snapshot/turn | 1,025,289 | $3.076 |

- **Slug (filtered) is ~4× cheaper than 2-shot vision, ~2× cheaper than 1-shot
  vision, and ~16× cheaper than vision in the opening** (when the move list is
  short while a screenshot is always 1,365 tokens).
- **Honest caveat:** a *naïve* Slug loop that re-snapshots the whole page every
  turn is **~9× worse than vision** (1.0M tokens). That is precisely why Slug does
  **server-side filtering** (`slug_snapshot {roles:["static_text"]}`) — the
  win comes from sending only the relevant nodes, not from "text vs image" alone.
  This is enforced by a regression test (`snapshot_vs_vision.rs`).

## Latency & correctness (reasoned, not micro-benchmarked here)

These are not pure token counts, so they're labeled as reasoning, not measured:

- **Fewer model round-trips per move.** Vision must screenshot → have the model
  locate the source/target squares in pixels → click → usually screenshot again to
  verify. Slug computes the square coordinates itself, so a move is **two clicks
  with no model round-trip to "find" them**. At ~1–3 s per model round-trip, that
  is the dominant real-time saving in a blitz game.
- **Exactness.** Slug reads moves as algebraic text (`1. e4 e5`) — unambiguous.
  Vision must infer the board from pixels, where a piece a few px off, a themed
  board, or an animation mid-capture causes misreads and mis-clicks.
- **Caveat:** the accessibility *harvest* of a huge page can itself be slow; Slug's
  clear latency win is on the **acting** path (clicks, no round-trip) and on the
  **token/context** size the model must read each turn.

## Bottom line

For chess.com, Slug is **several times cheaper in tokens/cost** than a screenshot
loop and avoids a model round-trip per move — **provided** snapshots are filtered.
The same property generalizes to any task where the relevant state is a small slice
of a large UI.
