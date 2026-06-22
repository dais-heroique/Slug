//! Grounded comparison: screenshot/vision vs Slug semantic snapshots for playing
//! chess.com.
//!
//! What is **measured** here (real, from this crate's actual serializer):
//!   * the byte/char size of a full `slug_snapshot` of a realistic web-app page,
//!   * the size of a server-side *filtered* snapshot (the move list only).
//!
//! What is **assumed** (clearly-sourced constants, see `assumptions`): the image
//! token cost formula and a screenshot resolution from Anthropic's docs, a
//! chars→tokens ratio, and a published input price. These are combined with the
//! measured sizes to produce per-move / per-game token and cost figures.
//!
//! Run it to print the table:
//!   cargo test -p slug-core --test snapshot_vs_vision -- --nocapture

use slug_core::{AliasTable, Bounds, SlugDocument, SlugNode, SlugRole, SlugState};

// ----------------------------- assumptions ---------------------------------
mod assumptions {
    // Anthropic documents image token cost as ~ (width * height) / 750.
    // Computer-use is recommended at WXGA (1280x800) or below.
    pub const SHOT_W: f64 = 1280.0;
    pub const SHOT_H: f64 = 800.0;
    pub const IMG_TOKENS_PER_PX: f64 = 1.0 / 750.0;

    // A vision agent re-reads the board by screenshot each turn: once to choose
    // its move, once to read the opponent's reply. (Conservative; many agents
    // also keep prior screenshots in context, which only widens the gap.)
    pub const SHOTS_PER_TURN: f64 = 2.0;
    // Small per-turn text (reasoning + tool JSON), same for both approaches.
    pub const TEXT_TOKENS_PER_TURN: f64 = 200.0;

    // Rough, conventional text tokenization (~4 chars/token for English+markup).
    pub const CHARS_PER_TOKEN: f64 = 4.0;

    // Published Claude Sonnet input price (USD per million input tokens). Image
    // and text input tokens are billed at the same input rate.
    pub const PRICE_PER_MTOK_USD: f64 = 3.0;

    // A typical game length (moves per side).
    pub const GAME_TURNS: usize = 40;
}

fn img_tokens() -> f64 {
    assumptions::SHOT_W * assumptions::SHOT_H * assumptions::IMG_TOKENS_PER_PX
}

fn text_tokens(chars: usize) -> f64 {
    chars as f64 / assumptions::CHARS_PER_TOKEN
}

/// Build a realistic chess.com-style page: a deep tree of site chrome (nav,
/// menus, panels, ads, links, buttons) — none of it useful to play — plus the
/// board as an opaque Canvas and the move list as `static_text`. `plies` controls
/// how many half-moves are in the move list. Returns the document.
///
/// Node count is in the low thousands, matching a real complex web app's
/// accessibility tree (the dominant driver of full-snapshot size).
fn build_chess_page(plies: usize) -> SlugDocument {
    let mut nodes: Vec<SlugNode> = Vec::new();
    let mut next = 0usize;
    let mut id = || {
        let r = format!("n{next}");
        next += 1;
        r
    };

    let root_ref = id();
    let mut root = SlugNode::new(&root_ref, SlugRole::Window);
    root.name = Some("Play Computer - Chess.com".into());

    let mut root_children: Vec<String> = Vec::new();

    // Site chrome: several regions, each with many links/buttons/labels — the
    // bulk of a real page and exactly what a vision model must visually parse.
    let region_names = ["Top navigation", "Left sidebar", "Right panel", "Footer", "Main"];
    for (ri, rname) in region_names.iter().enumerate() {
        let region_ref = id();
        let mut region = SlugNode::new(&region_ref, SlugRole::Group);
        region.name = Some((*rname).into());
        region.parent_ref = Some(root_ref.clone());
        root_children.push(region_ref.clone());

        let mut region_children: Vec<String> = Vec::new();
        // ~300 filler controls per region. Realistic page chrome is mostly
        // links/buttons/containers — NOT static_text (so a static_text filter
        // isolates the move list, as it does on the real site).
        for k in 0..300 {
            let cref = id();
            let role = match k % 4 {
                0 => SlugRole::Link,
                1 => SlugRole::Button,
                2 => SlugRole::Generic,
                _ => SlugRole::Cell,
            };
            let mut c = SlugNode::new(&cref, role);
            c.name = Some(format!("{rname} item {k} — menu/link/control"));
            c.parent_ref = Some(region_ref.clone());
            if role.is_interactive() {
                c.states = vec![SlugState::Enabled];
            }
            region_children.push(cref.clone());
            nodes.push(c);
        }
        // A handful of genuine static_text labels per region (timers, captions…).
        for k in 0..3 {
            let cref = id();
            let mut c = SlugNode::new(&cref, SlugRole::StaticText);
            c.text_content = Some(format!("{rname} label {k}"));
            c.parent_ref = Some(region_ref.clone());
            region_children.push(cref.clone());
            nodes.push(c);
        }

        // The "Main" region also holds the board and the move list.
        if ri == region_names.len() - 1 {
            // The board: an opaque Canvas with geometry (no useful child nodes).
            let board_ref = id();
            let mut board = SlugNode::new(&board_ref, SlugRole::Canvas);
            board.name = Some("Chess board".into());
            board.bounds = Some(Bounds { x: 352.0, y: 250.0, width: 800.0, height: 800.0 });
            board.parent_ref = Some(region_ref.clone());
            region_children.push(board_ref.clone());
            nodes.push(board);

            // The move list: one static_text per ply, e.g. "12. Nf3" / "Nc6".
            let list_ref = id();
            let mut list = SlugNode::new(&list_ref, SlugRole::List);
            list.name = Some("Move list".into());
            list.parent_ref = Some(region_ref.clone());
            let mut list_children = Vec::new();
            for p in 0..plies {
                let mref = id();
                let mut m = SlugNode::new(&mref, SlugRole::StaticText);
                let movetxt = if p % 2 == 0 {
                    format!("{}. Nf3", p / 2 + 1)
                } else {
                    "Nc6".to_string()
                };
                m.text_content = Some(movetxt);
                m.parent_ref = Some(list_ref.clone());
                list_children.push(mref.clone());
                nodes.push(m);
            }
            list.child_refs = list_children;
            region_children.push(list_ref.clone());
            nodes.push(list);
        }

        region.child_refs = region_children;
        nodes.push(region);
    }

    root.child_refs = root_children;
    nodes.push(root);

    SlugDocument::from_nodes(nodes)
}

/// Chars of the full snapshot and of the filtered (move-list) snapshot.
fn snapshot_sizes(plies: usize) -> (usize, usize, usize) {
    let doc = build_chess_page(plies);
    let mut aliases = AliasTable::new();
    let full = doc.to_yaml_assigning(&mut aliases);
    // The realistic "read the moves" call: filter to static_text only.
    let moves = slug_core::yaml::render_filtered(
        &doc,
        &aliases,
        None,
        &["static_text".to_string()],
        false,
        500,
    );
    (doc.len(), full.len(), moves.len())
}

#[test]
fn filtered_snapshot_is_orders_of_magnitude_smaller() {
    let (nodes, full, moves) = snapshot_sizes(2 * assumptions::GAME_TURNS);
    assert!(nodes > 1000, "expected a realistic node count, got {nodes}");
    assert!(full > 40_000, "full snapshot should be tens of KB, got {full}");
    // The move-list-only read must be at least 10x smaller than the full tree.
    assert!(moves * 10 < full, "filtered ({moves}) should be <10% of full ({full})");
}

#[test]
fn slug_uses_far_fewer_tokens_than_vision_over_a_game() {
    // Vision: SHOTS_PER_TURN screenshots + text, every turn.
    let vision_per_turn = assumptions::SHOTS_PER_TURN * img_tokens()
        + assumptions::TEXT_TOKENS_PER_TURN;
    let vision_game = vision_per_turn * assumptions::GAME_TURNS as f64;

    // Slug: one filtered move-list read per turn (grows with the game) + text.
    // The board is a canvas, so moves are just clicks (≈no perception tokens).
    let mut slug_game = 0.0;
    let mut slug_naive_game = 0.0;
    for turn in 1..=assumptions::GAME_TURNS {
        let plies = 2 * turn;
        let (_n, full_chars, moves_chars) = snapshot_sizes(plies);
        slug_game += text_tokens(moves_chars) + assumptions::TEXT_TOKENS_PER_TURN;
        slug_naive_game += text_tokens(full_chars) + assumptions::TEXT_TOKENS_PER_TURN;
    }

    // Filtered Slug is several times cheaper than vision.
    assert!(
        slug_game * 3.0 < vision_game,
        "Slug filtered ({slug_game:.0}) should be >3x cheaper than vision ({vision_game:.0})"
    );
    // Honest caveat the codebase must keep true: a NAIVE full-snapshot-every-turn
    // loop is actually *worse* than vision — which is exactly why the server-side
    // filtering exists.
    assert!(
        slug_naive_game > vision_game,
        "naive full-snapshot Slug ({slug_naive_game:.0}) is worse than vision ({vision_game:.0}) — filtering is what wins"
    );
}

/// Not an assertion — prints the grounded comparison table.
/// `cargo test -p slug-core --test snapshot_vs_vision -- --nocapture report`
#[test]
fn report() {
    use assumptions::*;

    let img = img_tokens();
    let vision_per_turn = SHOTS_PER_TURN * img + TEXT_TOKENS_PER_TURN;

    let price = |toks: f64| toks / 1_000_000.0 * PRICE_PER_MTOK_USD;

    println!("\n=== Playing chess.com: screenshot/vision vs Slug ===\n");
    println!("Assumptions (sourced):");
    println!("  screenshot {}x{} → {:.0} image tokens  [(w*h)/750, Anthropic]",
             SHOT_W as u32, SHOT_H as u32, img);
    println!("  {SHOTS_PER_TURN} screenshots/turn, {TEXT_TOKENS_PER_TURN:.0} text tokens/turn");
    println!("  {CHARS_PER_TOKEN} chars/token; ${PRICE_PER_MTOK_USD}/Mtok input (Claude Sonnet)");
    println!("  game length: {GAME_TURNS} moves/side\n");

    println!("MEASURED snapshot sizes (this crate's serializer):");
    for &t in &[1usize, 10, 20, 40] {
        let (nodes, full, moves) = snapshot_sizes(2 * t);
        println!(
            "  move {t:>2}: page = {nodes} nodes / {full} chars (~{:.0} tok) | \
             move-list filter = {moves} chars (~{:.0} tok)",
            text_tokens(full),
            text_tokens(moves),
        );
    }

    // Per-turn at endgame (move 40, biggest move list).
    let (_n, full_chars, moves_chars) = snapshot_sizes(2 * GAME_TURNS);
    let slug_per_turn_end = text_tokens(moves_chars) + TEXT_TOKENS_PER_TURN;
    let slug_naive_per_turn_end = text_tokens(full_chars) + TEXT_TOKENS_PER_TURN;

    println!("\nPER-TURN input tokens (at move {GAME_TURNS}):");
    println!("  vision (2 screenshots)       : {vision_per_turn:>8.0} tok");
    println!("  slug, full snapshot (naive)  : {slug_naive_per_turn_end:>8.0} tok");
    println!("  slug, filtered move list     : {slug_per_turn_end:>8.0} tok");

    // Whole game.
    let vision_game = vision_per_turn * GAME_TURNS as f64;
    let mut slug_game = 0.0;
    let mut slug_naive_game = 0.0;
    for turn in 1..=GAME_TURNS {
        let (_n, full, moves) = snapshot_sizes(2 * turn);
        slug_game += text_tokens(moves) + TEXT_TOKENS_PER_TURN;
        slug_naive_game += text_tokens(full) + TEXT_TOKENS_PER_TURN;
    }

    println!("\nWHOLE GAME ({GAME_TURNS} moves) input tokens & cost:");
    println!("  vision                       : {vision_game:>9.0} tok   ${:.4}", price(vision_game));
    println!("  slug, full snapshot (naive)  : {slug_naive_game:>9.0} tok   ${:.4}", price(slug_naive_game));
    println!("  slug, filtered move list     : {slug_game:>9.0} tok   ${:.4}", price(slug_game));
    println!("\n  → Slug (filtered) uses {:.0}x fewer input tokens than vision,",
             vision_game / slug_game);
    println!("    and {:.0}x fewer than even a naive full-snapshot Slug loop.\n",
             slug_naive_game / slug_game);
}
