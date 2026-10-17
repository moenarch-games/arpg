//! Acceptance for game-owned custom content (#154: "Own custom content per game instead of
//! process-lifetime `&'static` bundles").
//!
//! # Contract
//!
//! A game owns the content it runs (for example through an `Arc` of the resolved content).
//! Nothing a game publishes borrows content for the process lifetime, so a custom bundle is
//! freed together with the last game (and the last published value) that uses it, and the
//! process no longer bounds or refuses distinct bundles. The acceptance is split over three
//! integration-test binaries, because process-wide state (today's bundle registry, the
//! global heap) must not be shared between unrelated tests:
//!
//! - `tests/content_ownership.rs` (this file): published strike ids are owned values that
//!   outlive their game; determinism and content-revision checks are unchanged by content
//!   being dropped and resolved again.
//! - `tests/content_capacity.rs`: many more than 16 distinct valid bundles, interleaved with
//!   failed restores, all build, run and restore; none is refused for capacity.
//! - `tests/content_release.rs`: creating and dropping games with distinct sizeable bundles
//!   does not grow the process's resident memory with the number of bundles.
//!
//! ## Expected public surface
//!
//! - `ArpgGame::new_with_content(bundle: ContentBundle, run_seed: RunSeed)` and
//!   `ArpgGame::from_save_state_with_content(save: ArpgSaveState, bundle: ContentBundle)`
//!   keep their signatures and validation; the only refusals left are an invalid bundle, an
//!   invalid save and a save from another content revision (and a bundle whose revision
//!   collides with a different bundle, which is not exercised here). No capacity refusal.
//! - `MAX_LOADED_CONTENT_BUNDLES` is removed from the public API (these tests never name
//!   it).
//! - `ArpgGame::strike_outcomes() -> &[StrikeOutcome]` stays. `StrikeOutcome` is
//!   `Clone + Debug + PartialEq + Send + Sync + 'static` (it may stop being `Copy`), and its
//!   `definition` is an owned string type that dereferences to `str`: `String`, `Arc<str>`
//!   or a newtype with `Deref<Target = str>` all satisfy these tests, which read it through
//!   `id(&outcome.definition)` with `fn id<S: Deref<Target = str>>(&S) -> &str`. The
//!   `&'static str` borrowed from leaked content is what the issue removes; it satisfies
//!   these tests only by never freeing content (see `content_release.rs`).
//! - `StrikeEventSnapshot::definition` stays a `String`, so comparisons such as
//!   `event.definition == "sword.counterSlash"` in
//!   `crates/arpg-game-server/tests/counter_latency.rs` keep compiling unchanged.
//! - `content_revision()` and `base_bundle()` keep describing the built-in bundle, whose
//!   revision string is unchanged; a bundle's `revision()` is unchanged too.
//!
//! ## Obsolete in-crate unit tests (to replace, not to keep)
//!
//! These encode the process registry's capacity and refusal contract that #154 removes:
//!
//! - `src/content.rs` `tests::the_registry_is_bounded_and_reuses_loaded_bundles`
//!   (16 admitted bundles, then "16 content bundles" refusal; uses
//!   `MAX_LOADED_CONTENT_BUNDLES` and `Registry`).
//! - `src/lib.rs` `tests::failed_restores_never_use_up_a_content_slot` (loops
//!   `0..=MAX_LOADED_CONTENT_BUNDLES`, asserts `content::is_loaded`), superseded by
//!   `tests/content_capacity.rs`.
//! - `src/lib.rs` `tests::a_save_from_another_bundle_is_refused_before_that_bundle_is_retained`
//!   (its refusal half stays valid and is covered here by
//!   `a_save_is_refused_under_other_content_after_its_own_content_was_dropped`; its
//!   `content::is_loaded` retention assertion is obsolete).
//! - `src/content.rs`
//!   `tests::a_bundle_claiming_the_built_in_revision_is_refused_unless_it_is_the_built_in_bundle`
//!   and `tests::source_order_never_changes_a_loaded_bundle_or_its_revision` use
//!   `Registry`, `load` and pointer identity of `&'static Content` (shared registry
//!   entries); the revision-collision and source-order properties stay, the registry and
//!   pointer-identity mechanics are to be rewritten against the new ownership.
//! - Test-only helpers `content::is_loaded`, `content::load` and `Registry::load`.
//!
//! The encounter definition acceptance (`tests/encounter_definitions.rs`) keeps passing
//! unchanged.

use arpg_core::{
    ActionKind, ActionPhase, ArpgCommand, ArpgGame, ArpgSaveState, ArpgSnapshot, AuthoritativeGame,
    ContentBundle, PlayerActionSnapshot, PlayerCommand, StrikeOutcome, StrikeResult, StrikeTarget,
    base_bundle, content_revision,
};
use serde_json::{Value, json};
use std::ops::Deref;

/// Revision of the built-in bundle at the baseline of #154 (origin/main 435c9ea).
const BUILT_IN_REVISION: &str = "3a14b8be9ae981e4";
/// Revision of `paying(4_242)` at the same baseline.
const PAYING_4242_REVISION: &str = "336c4220cb330869";

const BUILT_IN_SWING: &str = "sword.lightSwing";
/// Distance from a monster at which an arranged light swing lands next tick.
const SIDE_DISTANCE: i32 = 120;
const WALL_MARGIN: i32 = 80;

// ---------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------

fn base_json() -> Value {
    serde_json::to_value(base_bundle()).expect("base bundle serialises")
}

fn parse(value: &Value) -> ContentBundle {
    ContentBundle::from_json(&value.to_string()).expect("fixture parses")
}

/// The built-in bundle with every reward paying `gold`: a distinct, valid bundle.
fn paying(gold: u32) -> ContentBundle {
    let mut value = base_json();
    for reward in value["rewards"].as_array_mut().expect("rewards") {
        reward["gold"] = json!(gold);
    }
    parse(&value)
}

/// The built-in bundle with its light swing strike renamed to `id` (by data only), so the
/// id strike events publish exists only in this bundle's content.
fn renamed_swing(id: &str) -> ContentBundle {
    let text = base_json()
        .to_string()
        .replace(&format!("\"{BUILT_IN_SWING}\""), &format!("\"{id}\""));
    ContentBundle::from_json(&text).expect("renamed bundle parses")
}

fn custom_game(bundle: ContentBundle, seed: u32) -> ArpgGame {
    let mut game = ArpgGame::new_with_content(bundle, seed).expect("a valid bundle runs a game");
    game.add_player(1).expect("player joins");
    game
}

fn restore(save: &ArpgSaveState, bundle: Option<ContentBundle>) -> Result<ArpgGame, String> {
    // Through the serialised form, exactly as a stored save comes back.
    let json = serde_json::to_string(save).expect("save serialises");
    let save: ArpgSaveState = serde_json::from_str(&json).expect("save deserialises");
    match bundle {
        None => ArpgGame::from_save_state(save),
        Some(bundle) => ArpgGame::from_save_state_with_content(save, bundle),
    }
    .map_err(|error| error.message().to_owned())
}

/// The published id as a `str`, for any owned id type that dereferences to `str`
/// (`String`, `Arc<str>`, a newtype) as well as today's `&'static str`.
fn id<S: Deref<Target = str>>(definition: &S) -> &str {
    definition
}

/// Requires a value that is independent of any game, borrow and thread.
fn assert_owned<T: Clone + Send + Sync + 'static>(_: &T) {}

/// Arranges player 1 beside the first monster with a light swing about to land (through
/// the save boundary), ticks until a strike resolves, and returns every strike outcome and
/// the matching published strike event definitions. The game is consumed and dropped.
fn swing_once(game: ArpgGame, bundle: Option<ContentBundle>) -> (Vec<StrikeOutcome>, Vec<String>) {
    let snapshot = game.snapshot().expect("snapshot");
    let monster = snapshot.monsters[0].clone();
    let room = snapshot
        .rooms
        .iter()
        .find(|room| room.id == monster.room_id)
        .expect("monster room")
        .clone();
    let [x, _, z] = monster.position;
    let (dx, dz) = [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .into_iter()
        .find(|&(dx, dz)| {
            let (px, pz) = (x + dx * SIDE_DISTANCE, z + dz * SIDE_DISTANCE);
            px >= room.min_x + WALL_MARGIN
                && px <= room.max_x - WALL_MARGIN
                && pz >= room.min_z + WALL_MARGIN
                && pz <= room.max_z - WALL_MARGIN
        })
        .expect("room beside the monster");
    let mut save = game.save_state().expect("save");
    drop(game);
    let player = save.players.iter_mut().find(|p| p.id == 1).expect("saved");
    let facing = [-dx as i8, -dz as i8];
    player.position = [
        (x + dx * SIDE_DISTANCE) as f32,
        player.position[1],
        (z + dz * SIDE_DISTANCE) as f32,
    ];
    player.velocity = [0.0; 3];
    player.movement = [0, 0];
    player.facing = facing;
    player.aim = None;
    player.locked_monster_id = None;
    player.draw_ticks = None;
    player.action = Some(PlayerActionSnapshot {
        kind: ActionKind::PrimaryAttack,
        phase: ActionPhase::Windup,
        ticks_remaining: 1,
        facing,
        connected: false,
        buffered: None,
        charge: 0,
        aim: None,
    });
    let mut game = restore(&save, bundle).expect("arranged save restores");
    let mut outcomes = Vec::new();
    let mut published = Vec::new();
    for _ in 0..6 {
        game.advance_tick().expect("tick");
        outcomes.extend(game.strike_outcomes().iter().cloned());
        published.extend(
            game.snapshot()
                .expect("snapshot")
                .strike_events
                .into_iter()
                .map(|event| event.definition),
        );
    }
    let hit_monster = outcomes.iter().any(|outcome| {
        outcome.target == StrikeTarget::Monster(monster.id)
            && matches!(outcome.result, StrikeResult::Hit { .. })
    });
    assert!(hit_monster, "the arranged swing hits: {outcomes:?}");
    (outcomes, published)
}

// ---------------------------------------------------------------------------------------
// Published ids are owned
// ---------------------------------------------------------------------------------------

#[test]
fn strike_outcomes_keep_their_definition_after_the_game_and_its_content_are_dropped() {
    const CUSTOM_SWING: &str = "custom.ownership.swing";
    let bundle = renamed_swing(CUSTOM_SWING);
    let (custom, custom_published) = swing_once(custom_game(bundle.clone(), 7), Some(bundle));
    // The game, its save and the bundle are gone; other content is created and dropped so
    // freed memory is reused before the outcomes are read again.
    for gold in 7_100..7_104 {
        let mut game = custom_game(paying(gold), 7);
        for _ in 0..3 {
            game.advance_tick().expect("tick");
        }
    }
    assert!(!custom.is_empty());
    for outcome in &custom {
        assert_owned(outcome);
        assert_owned(&outcome.definition);
        assert_eq!(id(&outcome.definition), CUSTOM_SWING, "{outcome:?}");
        // Still comparable as a value after its game is gone.
        assert_eq!(outcome, &outcome.clone());
    }
    assert!(
        custom_published.iter().all(|id| id == CUSTOM_SWING),
        "{custom_published:?}"
    );
    assert_eq!(custom_published.len(), custom.len());

    // The built-in content publishes its own id the same way.
    let mut built_in = ArpgGame::new_with_seed(7).expect("built-in game");
    built_in.add_player(1).expect("player joins");
    let (outcomes, published) = swing_once(built_in, None);
    for outcome in &outcomes {
        assert_eq!(id(&outcome.definition), BUILT_IN_SWING, "{outcome:?}");
    }
    assert!(published.iter().all(|id| id == BUILT_IN_SWING));
}

#[test]
fn strike_outcomes_can_leave_the_thread_of_their_game() {
    const CUSTOM_SWING: &str = "custom.ownership.thread";
    let outcomes = std::thread::spawn(|| {
        let bundle = renamed_swing(CUSTOM_SWING);
        swing_once(custom_game(bundle.clone(), 11), Some(bundle)).0
    })
    .join()
    .expect("worker thread");
    assert!(!outcomes.is_empty());
    assert!(
        outcomes
            .iter()
            .all(|outcome| id(&outcome.definition) == CUSTOM_SWING),
        "{outcomes:?}"
    );
}

// ---------------------------------------------------------------------------------------
// Determinism is unchanged by content being dropped and resolved again
// ---------------------------------------------------------------------------------------

/// Deterministic input script for player 1, keyed by tick.
fn scripted(tick: u32) -> Option<ArpgCommand> {
    match tick {
        0 => Some(ArpgCommand::SetMovement { x: 1, z: 0 }),
        14 => Some(ArpgCommand::SetMovement { x: 0, z: 1 }),
        22 => Some(ArpgCommand::PrimaryAttack { aim: None }),
        30 => Some(ArpgCommand::SetMovement { x: -1, z: 1 }),
        41 => Some(ArpgCommand::SecondaryAttack { aim: None }),
        50 => Some(ArpgCommand::SetMovement { x: 0, z: 0 }),
        55 => Some(ArpgCommand::Interact),
        _ => None,
    }
}

/// Snapshots and strike ids of ticks `from..to` under the script.
fn play(game: &mut ArpgGame, from: u32, to: u32) -> Vec<(ArpgSnapshot, Vec<String>)> {
    (from..to)
        .map(|tick| {
            if let Some(command) = scripted(tick) {
                game.apply_command(PlayerCommand::new(1, tick + 1, command).expect("command"))
                    .expect("command accepted");
            }
            game.advance_tick().expect("tick");
            let ids = game
                .strike_outcomes()
                .iter()
                .map(|outcome| id(&outcome.definition).to_owned())
                .collect();
            (game.snapshot().expect("snapshot"), ids)
        })
        .collect()
}

const SPLIT: u32 = 35;
const END: u32 = 70;

#[test]
fn a_bundle_replays_identically_before_and_after_other_content_was_dropped() {
    let seed = 4_242;
    // First run, with a save at the split.
    let (first, save) = {
        let mut game = custom_game(paying(4_242), seed);
        let mut trace = play(&mut game, 0, SPLIT);
        let save = game.save_state().expect("save");
        trace.extend(play(&mut game, SPLIT, END));
        (trace, save)
    };
    assert_eq!(
        first[0].0.content_revision,
        paying(4_242).revision(),
        "snapshots carry the bundle revision"
    );

    // Other bundles run and are dropped in between, including the same seed.
    for gold in 4_300..4_306 {
        let mut other = custom_game(paying(gold), seed);
        let trace = play(&mut other, 0, 10);
        assert_eq!(trace[0].0.content_revision, paying(gold).revision());
    }

    // A fresh game from a fresh copy of the bundle replays identically …
    let mut again = custom_game(paying(4_242), seed);
    assert_eq!(
        play(&mut again, 0, END),
        first,
        "same bundle, seed and inputs"
    );
    drop(again);

    // … and so does a restore of the save once every game of that content is gone.
    let mut restored = restore(&save, Some(paying(4_242))).expect("save restores");
    assert_eq!(
        play(&mut restored, SPLIT, END),
        first[usize::try_from(SPLIT).unwrap()..],
        "a restored save continues like the uninterrupted game"
    );

    // The built-in content keeps replaying identically next to all of this.
    let built_in = || {
        let mut game = ArpgGame::new_with_seed(seed).expect("built-in game");
        game.add_player(1).expect("player joins");
        play(&mut game, 0, END)
    };
    assert_eq!(built_in(), built_in());
}

// ---------------------------------------------------------------------------------------
// Revision checks are unchanged
// ---------------------------------------------------------------------------------------

#[test]
fn revision_strings_are_unchanged() {
    assert_eq!(content_revision(), BUILT_IN_REVISION);
    assert_eq!(base_bundle().revision(), BUILT_IN_REVISION);
    let built_in = ArpgGame::new_with_seed(3).expect("built-in game");
    assert_eq!(
        built_in.snapshot().unwrap().content_revision,
        BUILT_IN_REVISION
    );
    assert_eq!(
        built_in.save_state().unwrap().content_revision,
        BUILT_IN_REVISION
    );
    // The built-in bundle passed as custom content is the built-in content.
    let same = ArpgGame::new_with_content(paying_built_in(), 3).expect("built-in as custom");
    assert_eq!(same.snapshot().unwrap().content_revision, BUILT_IN_REVISION);
    assert_eq!(same.snapshot().unwrap(), built_in.snapshot().unwrap());

    assert_eq!(paying(4_242).revision(), PAYING_4242_REVISION);
    let custom = custom_game(paying(4_242), 3);
    assert_eq!(
        custom.snapshot().unwrap().content_revision,
        PAYING_4242_REVISION
    );
    assert_eq!(
        custom.save_state().unwrap().content_revision,
        PAYING_4242_REVISION
    );
}

/// The built-in bundle as an independent value parsed from its JSON form.
fn paying_built_in() -> ContentBundle {
    parse(&base_json())
}

#[test]
fn a_save_is_refused_under_other_content_after_its_own_content_was_dropped() {
    let seed = 21;
    let save_a = {
        let mut game = custom_game(paying(2_101), seed);
        play(&mut game, 0, 12);
        game.save_state().expect("save")
    };
    let snapshot_a = {
        let restored = restore(&save_a, Some(paying(2_101))).expect("restores under A");
        restored.snapshot().expect("snapshot")
    };
    // Every game of A is gone now.
    let error = restore(&save_a, Some(paying(2_102))).expect_err("refused under B");
    assert!(error.contains("content revision"), "{error}");
    let error = restore(&save_a, None).expect_err("refused under built-in content");
    assert!(error.contains("content revision"), "{error}");
    let error =
        restore(&save_a, Some(paying_built_in())).expect_err("refused under the built-in bundle");
    assert!(error.contains("content revision"), "{error}");

    let built_in_save = {
        let mut game = ArpgGame::new_with_seed(seed).expect("built-in game");
        game.add_player(1).expect("player joins");
        game.save_state().expect("save")
    };
    let error = restore(&built_in_save, Some(paying(2_101))).expect_err("refused under A");
    assert!(error.contains("content revision"), "{error}");
    assert!(restore(&built_in_save, None).is_ok());

    // Refusals change nothing: the save still restores under its own content, identically.
    let again = restore(&save_a, Some(paying(2_101))).expect("restores under A again");
    assert_eq!(again.snapshot().expect("snapshot"), snapshot_a);
}
