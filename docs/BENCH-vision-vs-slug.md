# Playing chess.com: Slug vs a screenshot loop — in tokens

How many **input tokens** does it take to actually *play* a game of chess.com —
counting the whole loop (looking, clicking, reading the reply) — two ways:

- **Vision** (e.g. Claude Computer Use): screenshot the board, and take a fresh
  screenshot after each click to verify it.
- **Slug**: play the move as two synthetic clicks (the board is a canvas), and read
  the opponent's reply as a small filtered text snapshot — **no screenshots**.

Reproducible — Slug's text sizes are measured by this repo's own serializer; the
screenshot cost uses Anthropic's published image-token formula:

```sh
cargo test -p slug-core --test snapshot_vs_vision -- --nocapture report
```

## Counting the whole move (this is the point)

A chess move = **pick up a piece + drop it = 2 clicks**.

| Per move | Vision | Slug |
|----------|--------|------|
| look at the board | 1 screenshot (1,365 tok) | 0 — board is a canvas |
| click 1 (pick up) | + 1 verify screenshot (1,365) | "ok: clicked" (~15 tok) |
| click 2 (drop) | + 1 verify screenshot (1,365) | "ok: clicked" (~15 tok) |
| read the reply | (next screenshot) | 1 filtered move-list read |

Vision pays **3 screenshots ≈ 4,095 image tokens every move**, because computer-use
returns a screenshot after each action. Slug's clicks are nearly free text and it
never screenshots to act.

## Measured / sourced inputs

- **Measured** (this crate's serializer): the filtered move-list snapshot —
  ~186 tok at move 1 growing to ~828 tok at move 40.
- **Sourced**: screenshot **1280×800 → 1,365 tokens** = `(w·h)/750` (Anthropic
  computer-use docs); ~4 chars/token; +200 reasoning tokens/move (both sides).

## Tokens to play ONE move

| Game point | Vision | Slug | Slug advantage |
|-----------|-------:|-----:|:--------------:|
| move 1  | 4,296 | **416** | **10× fewer** |
| move 10 | 4,296 | 563 | 8× fewer |
| move 20 | 4,296 | 728 | 6× fewer |
| move 40 | 4,296 | 1,058 | 4× fewer |

## Whole game (40 moves) — total input tokens

| Approach | Input tokens |
|----------|-------------:|
| Vision (screenshot loop) | 171,840 |
| **Slug (filtered)** | **29,459** |

> **Slug plays the whole game in ~6× fewer tokens than a screenshot loop** — and
> up to **10× fewer in the opening**, because every screenshot is a flat 1,365
> tokens while Slug only reads the short move list.
>
> Put another way: **one screenshot (1,365 tokens) costs more than reading the
> entire 40-move game as text (828 tokens).**

## Why Slug also *plays* faster (reasoned, not micro-benchmarked)

- **No model round-trip to "find" a square.** Vision must screenshot → have the
  model locate the source/target squares in pixels → click → screenshot to verify.
  Slug computes the square coordinates itself, so a move is two clicks with **no
  model round-trip to locate them**. At ~1–3 s per round-trip, that is the dominant
  real-time saving in a blitz game.
- **Exact, not inferred.** Slug reads moves as algebraic text (`1. e4 e5`); vision
  must infer the board from pixels, where a themed board or a mid-capture animation
  causes misreads and mis-clicks.

## Honest footnote

A *naïve* Slug loop that re-snapshots the **whole page** every move would cost
~1.03M tokens (worse than vision). That is exactly why Slug **filters server-side**
(`slug_snapshot {roles:["static_text"]}`) and plays the board with clicks — the win
is sending only the relevant nodes, not "text vs image" alone. A regression test
keeps this property true.
