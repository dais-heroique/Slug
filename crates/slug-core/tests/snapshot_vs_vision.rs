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

    // ---- The FULL play loop for ONE move (not just perception) ----
    // To play a chess move you pick up a piece and drop it = 2 actions.
    pub const ACTIONS_PER_MOVE: f64 = 2.0;
    // A vision agent must look at the board to choose its move (1 screenshot) AND,
    // because computer-use returns a screenshot after every action to verify it,
    // it takes one screenshot per click too. So shots/move = observe + actions.
    pub const VISION_OBSERVE_SHOTS: f64 = 1.0;
    pub const fn vision_shots_per_move() -> f64 {
        VISION_OBSERVE_SHOTS + ACTIONS_PER_MOVE // = 3
    }
    // Slug plays a move with synthetic clicks; each returns a tiny text result
    // ("ok: clicked at x,y") — no image, no perception. It reads the opponent's
    // reply once via a filtered move-list snapshot (the board is an opaque canvas,
    // so it never screenshots/snapshots to *act*).
    pub const SLUG_CLICK_RESULT_TOKENS: f64 = 15.0;

    // Small per-move text (reasoning + tool JSON), same for both approaches.
    pub const TEXT_TOKENS_PER_MOVE: f64 = 200.0;

    // Rough, conventional text tokenization (~4 chars/token for English+markup).
    pub const CHARS_PER_TOKEN: f64 = 4.0;

    // A typical game length (moves per side).
    pub const GAME_TURNS: usize = 40;

    // ---- Click reliability ----
    // Vision targets squares by pixel, inferred from a screenshot, so it mis-clicks
    // a non-trivial share of the time (themed boards, piece a few px off, an
    // animation mid-capture). Each wrong click must be noticed (a screenshot) and
    // redone (a click + its verify screenshot) — a ~2-screenshot recovery.
    pub const VISION_MISCLICK_RATE: f64 = 0.15; // per click (estimate)
    pub const VISION_RECOVERY_SHOTS: f64 = 2.0;
    // Slug clicks computed coordinates / acts on a node ref, so it does not have a
    // perception-induced mis-click class at all.
    pub const SLUG_MISCLICK_RATE: f64 = 0.0;
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
        false,
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

/// Tokens to play ONE full move with vision: observe the board + one verification
/// screenshot per click + reasoning text.
fn vision_tokens_per_move() -> f64 {
    assumptions::vision_shots_per_move() * img_tokens() + assumptions::TEXT_TOKENS_PER_MOVE
}

/// Vision tokens per move INCLUDING the expected cost of recovering from
/// pixel-targeting mis-clicks.
fn vision_tokens_per_move_with_errors() -> f64 {
    let expected_misclicks = assumptions::ACTIONS_PER_MOVE * assumptions::VISION_MISCLICK_RATE;
    let recovery = expected_misclicks * assumptions::VISION_RECOVERY_SHOTS * img_tokens();
    vision_tokens_per_move() + recovery
}

/// Tokens to play ONE full move with Slug: 2 click results (tiny text) + one
/// filtered move-list read of the current position + reasoning text. No images.
fn slug_tokens_per_move(plies: usize) -> f64 {
    let (_n, _full, moves_chars) = snapshot_sizes(plies);
    assumptions::ACTIONS_PER_MOVE * assumptions::SLUG_CLICK_RESULT_TOKENS
        + text_tokens(moves_chars)
        + assumptions::TEXT_TOKENS_PER_MOVE
}

#[test]
fn slug_plays_a_game_in_far_fewer_tokens_than_vision() {
    let vision_game = vision_tokens_per_move() * assumptions::GAME_TURNS as f64;
    let slug_game: f64 =
        (1..=assumptions::GAME_TURNS).map(|t| slug_tokens_per_move(2 * t)).sum();

    // Counting the WHOLE play loop (clicking included), Slug plays the game in
    // dramatically fewer tokens than a screenshot loop.
    assert!(
        slug_game * 4.0 < vision_game,
        "Slug ({slug_game:.0} tok) should be >4x cheaper than vision ({vision_game:.0} tok)"
    );
}

#[test]
fn one_screenshot_costs_more_than_the_whole_move_list() {
    let (_n, _full, moves_chars) = snapshot_sizes(2 * assumptions::GAME_TURNS);
    assert!(
        img_tokens() > text_tokens(moves_chars),
        "one screenshot ({:.0} tok) should cost more than the full move list ({:.0} tok)",
        img_tokens(),
        text_tokens(moves_chars),
    );
}

/// Not an assertion — prints the grounded, tokens-only comparison.
/// `cargo test -p slug-core --test snapshot_vs_vision -- --nocapture report`
#[test]
fn report() {
    use assumptions::*;

    let img = img_tokens();

    println!("\n=== Playing chess.com — full play loop, in TOKENS ===\n");
    println!("Assumptions (sourced):");
    println!("  screenshot {}x{} -> {:.0} image tokens   [(w*h)/750, Anthropic]",
             SHOT_W as u32, SHOT_H as u32, img);
    println!("  one move = {ACTIONS_PER_MOVE:.0} clicks (pick up + drop)");
    println!("  vision: {:.0} screenshots/move (observe + 1 verify per click)",
             vision_shots_per_move());
    println!("  slug:   0 screenshots — clicks return ~{SLUG_CLICK_RESULT_TOKENS:.0} tok of text,");
    println!("          + 1 filtered move-list read of the reply");
    println!("  +{TEXT_TOKENS_PER_MOVE:.0} reasoning tok/move (both); {CHARS_PER_TOKEN:.0} chars/token");
    println!("  game length: {GAME_TURNS} moves/side\n");

    // Running (cumulative) totals — summed the SAME way for BOTH sides, so the
    // comparison is apples-to-apples. Vision's per-move cost is flat (a screenshot
    // is always 1,365 tok) but it is still ADDED every move; Slug's per-move cost
    // grows (the move list it reads gets longer) and is added too.
    println!("PER MOVE (added this move)   and   CUMULATIVE (running total):");
    println!("        |        vision         |          slug          | cumulative");
    println!("  move  |  this move / total    |   this move / total    |  ratio");
    let mut v_cum = 0.0;
    let mut s_cum = 0.0;
    for t in 1..=GAME_TURNS {
        let v = vision_tokens_per_move();
        let s = slug_tokens_per_move(2 * t);
        v_cum += v;
        s_cum += s;
        if matches!(t, 1 | 5 | 10 | 20 | 30 | 40) {
            println!(
                "  {t:>3}   |  {v:>6.0} / {v_cum:>8.0}    |   {s:>5.0} / {s_cum:>7.0}     |  {:.0}x",
                v_cum / s_cum
            );
        }
    }

    let vision_game = vision_tokens_per_move() * GAME_TURNS as f64;
    let vision_game_err = vision_tokens_per_move_with_errors() * GAME_TURNS as f64;
    let slug_game: f64 = (1..=GAME_TURNS).map(|t| slug_tokens_per_move(2 * t)).sum();

    println!("\nWHOLE GAME ({GAME_TURNS} moves) — total input tokens:");
    println!("  vision (screenshot loop)            : {vision_game:>9.0} tok");
    println!("  vision + realistic mis-clicks ({:.0}%) : {vision_game_err:>9.0} tok",
             VISION_MISCLICK_RATE * 100.0);
    println!("  SLUG                                : {slug_game:>9.0} tok   ->  {:.0}x fewer (up to {:.0}x with mis-clicks)",
             vision_game / slug_game, vision_game_err / slug_game);
    println!("  one screenshot                      : {img:>9.0} tok   (> the whole {:.0}-token move list)",
             text_tokens(snapshot_sizes(2 * GAME_TURNS).2));
    println!("\nclick reliability: vision targets squares by pixel and mis-clicks ~{:.0}% of",
             VISION_MISCLICK_RATE * 100.0);
    println!("  the time (each error costs a ~{:.0}-screenshot recovery); Slug clicks computed",
             VISION_RECOVERY_SHOTS);
    println!("  coordinates / a node ref, so it has no pixel mis-click class ({:.0}%).\n",
             SLUG_MISCLICK_RATE * 100.0);
}
