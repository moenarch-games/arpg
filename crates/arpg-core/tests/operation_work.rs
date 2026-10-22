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
//! Physics steps use the existing `ArpgGame::physics_steps`; their cost is part of the
//! complete tick.

use std::collections::BTreeMap;

use arpg_core::{
    ArpgCommand, ArpgGame, ArpgSnapshot, AuthoritativeGame, MAX_ENCOUNTER_SPAWNS, OperationWork,
    PlayerCommand, WorkFixture, check_work_contract,
};
use serde_json::{Value, json};

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
            + snapshot.crates.len()
            + snapshot.strike_events.len()
            + snapshot.interaction_events.len(),
    )
    .unwrap()
}

#[test]
fn fixture_places_engaged_and_unrelated_monsters() {
    let mut game = fixture(3, 5, 30);
    game.advance_tick().unwrap();
    let snapshot = game.snapshot().unwrap();
    assert_eq!(snapshot.players.len(), 3);
    assert_eq!(snapshot.monsters.len(), 35);
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
    for (players, unrelated) in [(1, 8), (4, 8), (4, 32)] {
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
    for unrelated in [8, 16, 32] {
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
    assert_eq!(entities[1] - entities[0], 8);
    assert_eq!(entities[2] - entities[1], 16);
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
        let trace = targeting_trace(players, engaged, 8);
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
    let small = targeting_trace(4, 4, 8);
    assert_eq!(targeting_trace(4, 4, 16), small);
    assert_eq!(targeting_trace(4, 4, 32), small);
}

#[test]
fn player_command_bookkeeping_ignores_unrelated_population() {
    for unrelated in [8, 16, 32] {
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
        let mut game = fixture(4, 8, 32);
        for _ in 0..WINDOW_TICKS {
            game.advance_tick().unwrap();
        }
        let work = OperationWork::take();
        (game.snapshot().unwrap(), work)
    };
    assert_eq!(run(), run());
}

#[test]
fn fixture_rooms_hold_at_most_one_encounter_of_monsters() {
    let max = u32::try_from(MAX_ENCOUNTER_SPAWNS).unwrap();
    let mut game = fixture(4, max, 4 * max);
    game.advance_tick().unwrap();
    let snapshot = game.snapshot().unwrap();
    let mut per_room = BTreeMap::<u32, u32>::new();
    for monster in &snapshot.monsters {
        *per_room.entry(monster.room_id).or_default() += 1;
    }
    assert_eq!(per_room.len(), 5);
    assert!(per_room.values().all(|&count| count == max));
    for over in [
        WorkFixture {
            players: 4,
            engaged_monsters: max + 1,
            unrelated_monsters: 8,
        },
        WorkFixture {
            players: 4,
            engaged_monsters: 4,
            unrelated_monsters: 4 * max + 1,
        },
        WorkFixture {
            players: 0,
            engaged_monsters: 4,
            unrelated_monsters: 8,
        },
    ] {
        assert!(over.build(SEED).is_err(), "{over:?}");
    }
}

/// A valid one-operation contract over `n` and `m`.
fn checker_contract() -> Value {
    json!({
        "schemaVersion": 2,
        "suite": "test",
        "scenarios": [],
        "operations": [{
            "id": "op",
            "description": "test operation",
            "dimensions": { "n": { "description": "n" }, "m": { "description": "m" } },
            "scalePoints": [
                { "n": 1, "m": 1 }, { "n": 2, "m": 1 }, { "n": 4, "m": 1 }, { "n": 1, "m": 4 }
            ],
            "metrics": [
                { "name": "fixed", "unit": "ops", "signal": "operation-count",
                  "budget": { "max": 3 },
                  "growth": [{ "dimension": "n", "bound": "constant" }] },
                { "name": "scan", "unit": "ops", "signal": "operation-count",
                  "growth": [{ "dimension": "n", "bound": "linear" },
                             { "dimension": "m", "bound": "constant" }] }
            ]
        }]
    })
}

fn check(contract: &Value, scan: fn(u64, u64) -> u64) -> Result<Value, Vec<String>> {
    check_work_contract(&contract.to_string(), &mut |id, point| {
        if id != "op" {
            return Err("no runner".to_owned());
        }
        let (n, m) = (point["n"], point["m"]);
        Ok(BTreeMap::from([
            ("fixed".to_owned(), 3),
            ("scan".to_owned(), scan(n, m)),
        ]))
    })
}

fn operation(contract: &mut Value) -> &mut Value {
    &mut contract["operations"][0]
}

fn refused(contract: &Value, expected: &str) {
    let failures = check(contract, |n, _| 2 * n + 7).expect_err(expected);
    assert!(
        failures.iter().any(|failure| failure.contains(expected)),
        "{expected:?} not in {failures:?}"
    );
}

#[test]
fn work_contract_check_passes_declared_bounds_and_emits_evidence() {
    let evidence = check(&checker_contract(), |n, _| 2 * n + 7).unwrap();
    let samples = &evidence["operations"][0]["samples"];
    assert_eq!(samples.as_array().unwrap().len(), 4);
    assert_eq!(
        samples[2],
        json!({ "dimensions": { "m": 1, "n": 4 }, "metrics": { "fixed": 3, "scan": 15 } })
    );
}

#[test]
fn work_contract_check_rejects_violated_bounds() {
    // N² is not linear in n; a scan of m breaks the constant bound.
    let quadratic = check(&checker_contract(), |n, _| n * n).unwrap_err();
    assert!(
        quadratic
            .iter()
            .any(|failure| failure.contains("scan is not linear in n"))
    );
    let world_scan = check(&checker_contract(), |n, m| n + m).unwrap_err();
    assert!(
        world_scan
            .iter()
            .any(|failure| failure.contains("scan is not constant in m"))
    );
    let mut over_budget = checker_contract();
    over_budget["operations"][0]["metrics"][0]["budget"]["max"] = json!(2);
    refused(&over_budget, "fixed=3 exceeds its budget 2");
}

#[test]
fn work_contract_check_fails_closed_on_incomplete_contracts() {
    let base = checker_contract();
    let edit = |change: &dyn Fn(&mut Value)| {
        let mut contract = base.clone();
        change(&mut contract);
        contract
    };
    let cases: Vec<(Value, &str)> = vec![
        (
            edit(&|c| c["schemaVersion"] = json!(1)),
            "not schemaVersion 2",
        ),
        (
            edit(&|c| c["operations"] = json!([])),
            "declares no operations",
        ),
        (
            edit(&|c| {
                c.as_object_mut().unwrap().remove("operations");
            }),
            "declares no operations",
        ),
        (
            edit(&|c| {
                operation(c).as_object_mut().unwrap().remove("scalePoints");
            }),
            "scalePoints must be a non-empty array",
        ),
        (
            edit(&|c| operation(c)["scalePoints"] = json!([])),
            "scalePoints must be a non-empty array",
        ),
        (
            edit(&|c| operation(c)["scalePoints"] = json!({ "n": 1 })),
            "scalePoints must be a non-empty array",
        ),
        (
            edit(&|c| operation(c)["scalePoints"][0] = json!({ "n": 1 })),
            "the declared dimensions are",
        ),
        (
            edit(&|c| operation(c)["scalePoints"][0] = json!({ "n": 1, "m": 1, "k": 1 })),
            "the declared dimensions are",
        ),
        (
            edit(&|c| operation(c)["scalePoints"][0]["n"] = json!(0)),
            "not a positive integer",
        ),
        (
            edit(&|c| operation(c)["scalePoints"][0]["n"] = json!("1")),
            "not a positive integer",
        ),
        (
            edit(&|c| operation(c)["scalePoints"][1] = json!({ "n": 1, "m": 1 })),
            "repeats",
        ),
        (
            edit(&|c| operation(c)["dimensions"] = json!({})),
            "dimensions must be a non-empty object",
        ),
        (
            edit(&|c| {
                operation(c).as_object_mut().unwrap().remove("metrics");
            }),
            "metrics must be a non-empty array",
        ),
        (
            edit(&|c| operation(c)["metrics"] = json!([])),
            "metrics must be a non-empty array",
        ),
        (
            edit(&|c| operation(c)["metrics"] = json!({})),
            "metrics must be a non-empty array",
        ),
        (
            edit(&|c| operation(c)["metrics"][0]["name"] = json!(" ")),
            "has no name",
        ),
        (
            edit(&|c| operation(c)["metrics"][1]["name"] = json!("fixed")),
            "declared twice",
        ),
        (
            edit(&|c| {
                operation(c)["metrics"][1]["growth"] =
                    json!({ "dimension": "n", "bound": "linear" })
            }),
            "growth must be an array",
        ),
        (
            edit(&|c| operation(c)["metrics"][1]["growth"] = json!([])),
            "neither a budget nor a growth bound",
        ),
        (
            edit(&|c| operation(c)["metrics"][1]["growth"][0]["bound"] = json!("quadratic")),
            "bound of constant or linear",
        ),
        (
            edit(&|c| {
                operation(c)["metrics"][1]["growth"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("bound");
            }),
            "bound of constant or linear",
        ),
        (
            edit(&|c| operation(c)["metrics"][1]["growth"][0]["dimension"] = json!("k")),
            "needs a declared dimension",
        ),
        (
            edit(&|c| operation(c)["metrics"][1]["growth"][1]["dimension"] = json!("n")),
            "repeats a growth bound",
        ),
        (
            edit(&|c| operation(c)["metrics"][0]["budget"] = json!({ "max": -1 })),
            "budget must be",
        ),
        (
            edit(&|c| operation(c)["metrics"][0]["budget"] = json!(3)),
            "budget must be",
        ),
        (
            edit(&|c| {
                operation(c)["scalePoints"] = json!([{ "n": 1, "m": 1 }, { "n": 2, "m": 2 }])
            }),
            "no scale points vary",
        ),
        (edit(&|c| operation(c)["id"] = json!("Op")), "no valid id"),
        (
            edit(&|c| {
                let op = operation(c).clone();
                c["operations"].as_array_mut().unwrap().push(op);
            }),
            "operation is declared twice",
        ),
        (edit(&|c| operation(c)["id"] = json!("other")), "no runner"),
    ];
    for (contract, expected) in &cases {
        refused(contract, expected);
    }
}

#[test]
fn work_contract_check_fails_closed_on_incomplete_measurements() {
    let contract = checker_contract().to_string();
    let missing = check_work_contract(&contract, &mut |_, _| {
        Ok(BTreeMap::from([("fixed".to_owned(), 3)]))
    })
    .unwrap_err();
    assert!(
        missing
            .iter()
            .any(|failure| failure.contains("measured metrics"))
    );
    let extra = check_work_contract(&contract, &mut |_, _| {
        Ok(BTreeMap::from([
            ("fixed".to_owned(), 3),
            ("scan".to_owned(), 1),
            ("other".to_owned(), 1),
        ]))
    })
    .unwrap_err();
    assert!(
        extra
            .iter()
            .any(|failure| failure.contains("measured metrics"))
    );
    let failed =
        check_work_contract(&contract, &mut |_, _| Err("did not replay".to_owned())).unwrap_err();
    assert_eq!(failed.len(), 4);
}
