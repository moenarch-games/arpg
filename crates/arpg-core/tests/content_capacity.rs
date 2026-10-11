//! Acceptance for #154: a process runs any number of distinct custom content bundles.
//!
//! Games own their content, so there is no process-wide bundle registry with a capacity
//! (today `MAX_LOADED_CONTENT_BUNDLES`: 16 admitted plus 16 pending candidates, which this
//! file deliberately never names because #154 removes it). Many more than 16 distinct valid
//! bundles, interleaved with failed restores of further bundles, all build, run and
//! restore; the only refusals are of invalid saves and saves from another revision. See
//! `tests/content_ownership.rs` for the full contract and the list of obsolete unit tests.
//!
//! This is its own test binary, because today's registry is process-wide state that would
//! otherwise leak between unrelated tests.

use arpg_core::{
    ArpgCommand, ArpgGame, ArpgSaveState, AuthoritativeGame, ContentBundle, PlayerCommand,
    base_bundle,
};
use serde_json::{Value, json};

/// Distinct bundles whose games never build: only failed restores are attempted with them.
const NEVER_BUILT: u32 = 40;
/// Distinct bundles that build a game each, all games alive at once.
const BUILT: u32 = 64;

fn paying(gold: u32) -> ContentBundle {
    let mut value: Value = serde_json::to_value(base_bundle()).expect("base bundle serialises");
    for reward in value["rewards"].as_array_mut().expect("rewards") {
        reward["gold"] = json!(gold);
    }
    ContentBundle::from_json(&value.to_string()).expect("fixture parses")
}

/// A save whose revision matches `bundle` but whose room set is broken, so only the restore
/// itself fails (the content is valid).
fn broken_save_for(bundle: &ContentBundle) -> ArpgSaveState {
    let mut save = ArpgGame::new_with_seed(9)
        .expect("built-in game")
        .save_state()
        .expect("save");
    save.content_revision = bundle.revision();
    save.rooms.pop();
    save
}

fn assert_not_a_capacity_refusal(message: &str) {
    assert!(
        !message.contains("content bundles") && !message.contains("no further bundle"),
        "refused for capacity: {message}"
    );
}

fn run(game: &mut ArpgGame, ticks: u32) {
    for tick in 0..ticks {
        if tick == 0 {
            game.apply_command(
                PlayerCommand::new(1, 1, ArpgCommand::SetMovement { x: 1, z: 1 }).expect("command"),
            )
            .expect("command accepted");
        }
        game.advance_tick().expect("tick");
    }
}

#[test]
fn many_distinct_bundles_build_and_restore_among_failed_restores() {
    // Failed restores of bundles that never build any game.
    for gold in 50_000..50_000 + NEVER_BUILT {
        let bundle = paying(gold);
        for _ in 0..2 {
            let Err(error) =
                ArpgGame::from_save_state_with_content(broken_save_for(&bundle), bundle.clone())
            else {
                panic!("a broken save never restores");
            };
            assert_not_a_capacity_refusal(error.message());
        }
    }

    // Distinct bundles that build, each next to a failed restore of yet another bundle, with
    // every game kept alive at once.
    let mut games = Vec::new();
    for index in 0..BUILT {
        let failing = paying(60_000 + index);
        let Err(error) =
            ArpgGame::from_save_state_with_content(broken_save_for(&failing), failing.clone())
        else {
            panic!("a broken save never restores");
        };
        assert_not_a_capacity_refusal(error.message());

        let bundle = paying(70_000 + index);
        let mut game = match ArpgGame::new_with_content(bundle.clone(), 9 + index) {
            Ok(game) => game,
            Err(error) => panic!("distinct bundle {index} is refused: {}", error.message()),
        };
        game.add_player(1).expect("player joins");
        run(&mut game, 8);
        let snapshot = game.snapshot().expect("snapshot");
        assert_eq!(snapshot.content_revision, bundle.revision());
        assert_eq!(snapshot.tick, 8);

        // Its save restores under the same bundle and continues identically.
        let save = game.save_state().expect("save");
        let mut restored = match ArpgGame::from_save_state_with_content(save, bundle.clone()) {
            Ok(game) => game,
            Err(error) => panic!("bundle {index} save is refused: {}", error.message()),
        };
        assert_eq!(restored.snapshot().expect("snapshot"), snapshot);
        game.advance_tick().expect("tick");
        restored.advance_tick().expect("tick");
        assert_eq!(restored.snapshot().unwrap(), game.snapshot().unwrap());
        games.push((bundle, game));
    }

    // Every game still runs its own content.
    for (bundle, game) in &mut games {
        game.advance_tick().expect("tick");
        assert_eq!(game.snapshot().unwrap().content_revision, bundle.revision());
    }

    // The bundles that only failed before still build once they are given a valid save.
    drop(games);
    for gold in 50_000..50_000 + NEVER_BUILT {
        let bundle = paying(gold);
        let game = ArpgGame::new_with_content(bundle.clone(), 1)
            .unwrap_or_else(|error| panic!("bundle {gold} is refused: {}", error.message()));
        let save = game.save_state().expect("save");
        assert!(ArpgGame::from_save_state_with_content(save, bundle).is_ok());
    }
}
