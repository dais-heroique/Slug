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

## Per move *and* cumulative — both summed the same way

Both columns are added up the same way. Vision's **per-move** cost is flat (a
screenshot is always 1,365 tokens) but it is still **added every move**; Slug's
per-move cost grows (the move list gets longer) and is added too. The **cumulative**
columns are the running totals you actually pay.

| Move | Vision (this move) | Vision (cumulative) | Slug (this move) | Slug (cumulative) | Cumulative ratio |
|----:|-----:|-----:|-----:|-----:|:--:|
| 1  | 4,296 | 4,296 | 416 | 416 | **10×** |
| 5  | 4,296 | 21,480 | 482 | 2,245 | 10× |
| 10 | 4,296 | 42,960 | 563 | 4,896 | 9× |
| 20 | 4,296 | 85,920 | 728 | 11,434 | 8× |
| 30 | 4,296 | 128,880 | 893 | 19,622 | 7× |
| 40 | 4,296 | **171,840** | 1,058 | **29,459** | **6×** |

## Whole game (40 moves) — total input tokens

| Approach | Input tokens |
|----------|-------------:|
| Vision (screenshot loop) | **171,840** |
| **Slug (filtered)** | **29,459** |

> **Slug plays the whole game in ~6× fewer tokens than a screenshot loop** — and
> up to **10× fewer in the opening**, because every screenshot is a flat 1,365
> tokens while Slug only reads the short move list.
>
> Put another way: **one screenshot (1,365 tokens) costs more than reading the
> entire 40-move game as text (828 tokens).**

## Click accuracy — where vision *also* loses tokens

A screenshot agent aims at squares **by pixel**, inferred from the image. It
mis-clicks a meaningful share of the time — a themed board, a piece rendered a few
pixels off, or an animation caught mid-capture — and **every wrong click costs a
recovery**: a screenshot to notice it, then another click and screenshot to redo it.

| | Vision | Slug |
|---|---|---|
| how a square is targeted | inferred from pixels | computed coordinate / node ref |
| mis-click rate (estimate) | ~15% per click | **0% (no pixel class)** |
| cost of a mis-click | ~2 extra screenshots (~2,730 tok) | n/a |
| game total with mis-clicks | **~204,600 tok** | **29,459 tok** |

With realistic mis-clicks the gap widens to **~7×**. Slug simply does not have this
error class: it clicks coordinates it computed, or acts on a node `ref` — so it is
both cheaper **and** more reliable.

## Why Slug also *plays* faster

- **No model round-trip to "find" a square.** Vision must screenshot → have the
  model locate the source/target squares in pixels → click → screenshot to verify.
  Slug computes the coordinates itself, so a move is two clicks with **no model
  round-trip to locate them**. At ~1–3 s per round-trip, that is the dominant
  real-time saving in a blitz game.
- **Exact, not inferred.** Slug reads moves as algebraic text (`1. e4 e5`); vision
  must infer the board from pixels.

## Summary

Counting the **whole** play loop, in tokens: **Slug plays a full game of chess.com
in ~6× fewer input tokens than a screenshot agent (~7× once mis-clicks are
included), and up to ~10× fewer in the opening** — while being more reliable,
because it never targets by pixel.
