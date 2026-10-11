//! Acceptance for operation work contracts (#97).
//!
//! `.performance/contract.json` (schemaVersion 2, the coding-tooling work-complexity syntax)
//! declares how ARPG-owned orchestration may grow. These tests pin the exact bounds behind
//! those declarations on small worlds; `examples/performance_smoke.rs` measures the declared
//! scale points and checks the contract itself.
//!
//! API contract (arpg-core, feature `operation-work`; absent from product builds):
//!
//! ```ignore
//! /// Deterministic work counted on the current thread since the last `take`.
//! #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
//! pub struct OperationWork {
//!     /// Authoritative snapshots built (`AuthoritativeGame::snapshot`).
//!     pub snapshot_materializations: u64,
//!     /// Entity records written into those snapshots.
//!     pub snapshot_entities_materialized: u64,
//!     /// Engagement-policy decisions of free monsters in active encounters.
//!     pub monster_decisions: u64,
//!     /// Target candidates those decisions scored.
//!     pub monster_target_evaluations: u64,
//!     /// Player validation, lane sequence checks and lane watermark records.
//!     pub command_bookkeeping_steps: u64,
//! }
//! impl OperationWork {
//!     /// Returns the counters of this thread and resets them.
//!     pub fn take() -> Self;
//! }
//!
//! /// A seeded dungeon with `players` in combat room 2, `engaged_monsters` in front of them
//! /// and `unrelated_monsters` spread over the dormant combat rooms. Monster ids are
//! /// 1..=engaged, then the unrelated ones.
//! pub struct WorkFixture { pub players: u32, pub engaged_monsters: u32, pub unrelated_monsters: u32 }
//! impl WorkFixture {
//!     pub fn build(&self, seed: RunSeed) -> Result<ArpgGame, GameError>;
//! }
//! ```
//!
//! Physics steps use the existing `ArpgGame::physics_steps`; the cost of a step stays with
//! physics-engine.

use arpg_core::{
    ArpgCommand, ArpgGame, ArpgSnapshot, AuthoritativeGame, OperationWork, PlayerCommand,
    WorkFixture,
};

const SEED: u32 = 0xA420_0916;
const WINDOW_TICKS: u64 = 60;

fn fixture(players: u32, engaged_monsters: u32, unrelated_monsters: u32) -> ArpgGame {
    let game = WorkFixture {
        players,
        engaged_monsters,
        unrelated_monsters,
    }
    .build(SEED)
    .expect("work fixture must build");
    // Building a fixture is not part of any measured operation.
    OperationWork::take();
    game
}

fn exported_entities(snapshot: &ArpgSnapshot) -> u64 {
    u64::try_from(
        snapshot.rooms.len()
            + snapshot.doors.len()
            + snapshot.players.len()
            + snapshot.monsters.len()
            + snapshot.ground_loot.len()
            + snapshot.static_colliders.len()
            + snapshot.arrows.len()
            + snapshot.chests.len()
            + snapshot.strike_events.len()
            + snapshot.interaction_events.len(),
    )
    .unwrap()
}

#[test]
fn fixture_places_engaged_and_unrelated_monsters() {
    let mut game = fixture(3, 5, 40);
    game.advance_tick().unwrap();
    let snapshot = game.snapshot().unwrap();
    assert_eq!(snapshot.players.len(), 3);
    assert_eq!(snapshot.monsters.len(), 45);
    let engaged_room = snapshot.monsters[0].room_id;
    assert!(
        snapshot.monsters[..5]
            .iter()
            .all(|monster| monster.room_id == engaged_room)
    );
    assert!(
        snapshot.monsters[5..]
            .iter()
            .all(|monster| monster.room_id != engaged_room)
    );
    // The players' room is the only active encounter.
    let active = snapshot
        .rooms
        .iter()
        .filter(|room| room.encounter_state == arpg_core::RoomEncounterState::Active)
        .map(|room| room.id)
        .collect::<Vec<_>>();
    assert_eq!(active, vec![engaged_room]);
}

#[test]
fn ordinary_ticks_step_physics_once_and_materialize_no_snapshot() {
    for (players, unrelated) in [(1, 12), (4, 12), (4, 252)] {
        let mut game = fixture(players, 4, unrelated);
        let mut sequence = 1;
        for tick in 0..WINDOW_TICKS {
            if tick % 15 == 0 {
                for player_id in 1..=players {
                    let x = if (tick / 15) % 2 == 0 { 1 } else { -1 };
                    game.apply_command(
                        PlayerCommand::new(
                            player_id,
                            sequence,
                            ArpgCommand::SetMovement { x, z: 0 },
                        )
                        .unwrap(),
                    )
                    .unwrap();
                }
                sequence += 1;
            }
            let before = game.physics_steps();
            game.advance_tick().unwrap();
            assert_eq!(game.physics_steps() - before, 1, "tick {tick}");
        }
        let work = OperationWork::take();
        assert_eq!(work.snapshot_materializations, 0);
        assert_eq!(work.snapshot_entities_materialized, 0);
    }
}

#[test]
fn requested_snapshot_materializes_each_exported_entity_once() {
    let mut entities = Vec::new();
    for unrelated in [12, 60, 252] {
        let mut game = fixture(4, 4, unrelated);
        game.advance_tick().unwrap();
        OperationWork::take();
        let steps = game.physics_steps();
        let snapshot = game.snapshot().unwrap();
        let work = OperationWork::take();
        assert_eq!(work.snapshot_materializations, 1);
        assert_eq!(
            work.snapshot_entities_materialized,
            exported_entities(&snapshot)
        );
        assert_eq!(snapshot.monsters.len(), 4 + unrelated as usize);
        assert_eq!(
            game.physics_steps(),
            steps,
            "a snapshot never steps physics"
        );
        assert_eq!(work.monster_target_evaluations, 0);
        entities.push(work.snapshot_entities_materialized);
    }
    // O(N): exactly one more exported record per added monster.
    assert_eq!(entities[1] - entities[0], 48);
    assert_eq!(entities[2] - entities[1], 192);
}

/// Per-tick (decisions, evaluations) over the window.
fn targeting_trace(players: u32, engaged: u32, unrelated: u32) -> Vec<(u64, u64)> {
    let mut game = fixture(players, engaged, unrelated);
    (0..WINDOW_TICKS)
        .map(|_| {
            game.advance_tick().unwrap();
            let work = OperationWork::take();
            (work.monster_decisions, work.monster_target_evaluations)
        })
        .collect()
}

#[test]
fn monster_targeting_scores_living_players_for_free_engaged_monsters_only() {
    for (players, engaged) in [(1, 4), (2, 4), (4, 4), (4, 2), (4, 8)] {
        let trace = targeting_trace(players, engaged, 16);
        for &(decisions, evaluations) in &trace {
            assert!(decisions <= u64::from(engaged));
            assert!(evaluations <= decisions * u64::from(players));
        }
        // On the first tick every engaged monster is free and scores every player.
        assert_eq!(
            trace[0],
            (u64::from(engaged), u64::from(engaged * players)),
            "players {players}, engaged {engaged}"
        );
    }
}

#[test]
fn monster_targeting_is_independent_of_unrelated_population() {
    let small = targeting_trace(4, 4, 16);
    assert_eq!(targeting_trace(4, 4, 128), small);
    assert_eq!(targeting_trace(4, 4, 512), small);
}

#[test]
fn player_command_bookkeeping_ignores_unrelated_population() {
    for unrelated in [16, 128, 512] {
        let mut game = fixture(4, 4, unrelated);
        game.advance_tick().unwrap();
        OperationWork::take();
        let steps = game.physics_steps();
        game.apply_command(
            PlayerCommand::new(1, 1, ArpgCommand::SetMovement { x: 1, z: 0 }).unwrap(),
        )
        .unwrap();
        let work = OperationWork::take();
        assert_eq!(
            work,
            OperationWork {
                command_bookkeeping_steps: 3,
                ..OperationWork::default()
            }
        );
        assert_eq!(game.physics_steps(), steps);
    }
}

#[test]
fn scaled_operations_replay_identically() {
    let run = || {
        let mut game = fixture(4, 8, 128);
        for _ in 0..WINDOW_TICKS {
            game.advance_tick().unwrap();
        }
        let work = OperationWork::take();
        (game.snapshot().unwrap(), work)
    };
    assert_eq!(run(), run());
}
