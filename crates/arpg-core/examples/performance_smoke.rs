//! Blocking performance smoke for `.performance/contract.json`.
//!
//! 1. The four-player journey: wall-clock is advisory, the physics step budget blocks.
//! 2. The operation work contracts (schemaVersion 2, coding-tooling work-complexity syntax):
//!    every declared operation runs at every declared scale point, twice, and must replay
//!    identically. `arpg_core::check_work_contract` then checks the budgets and the
//!    `constant`/`linear` growth bounds with the coding-tooling semantics, failing closed on a
//!    malformed or incomplete contract or measurement.
//!
//! The last stdout line is the `operations` part of a `coding-tooling/work-evidence/v1`
//! document; `scripts/collect_work_evidence.py` adds the suite and the contract hash.
use std::collections::BTreeMap;
use std::time::Instant;

use arpg_core::{
    ArpgCommand, ArpgGame, AuthoritativeGame, OperationWork, PlayerCommand, WorkFixture,
    check_work_contract,
};
use serde_json::Value;

const RUNS: usize = 3;
const TICKS: u64 = 3_600;
/// Blocking budget declared for `physics_steps` in `.performance/contract.json`.
const MAX_PHYSICS_STEPS: u64 = 3_600;
const CONTRACT: &str = include_str!("../../../.performance/contract.json");
const SEED: u32 = 0xA420_0916;
/// Ticks of the ordinary-tick and monster-targeting windows.
const WINDOW_TICKS: u64 = 60;
/// Ticks between the movement commands every player sends in the ordinary-tick window.
const COMMAND_INTERVAL_TICKS: u64 = 15;
/// Monsters the ordinary-tick and requested-snapshot fixtures place in the active encounter.
const ENGAGED_MONSTERS: u32 = 4;
/// Players of the requested-snapshot and player-command fixtures.
const PLAYERS: u32 = 4;

type Dimensions = BTreeMap<String, u64>;
type Metrics = BTreeMap<String, u64>;

fn run_journey() -> (arpg_core::ArpgSnapshot, u64, OperationWork) {
    OperationWork::take();
    let mut game = ArpgGame::new().expect("built-in ARPG fixture must be valid");
    for player_id in 1..=4 {
        game.add_player(player_id)
            .expect("player fixture must be valid");
    }

    let mut sequences = [1_u32; 4];
    for tick in 0..TICKS {
        if tick % 300 == 0 {
            for player_id in 1..=4 {
                let index = usize::try_from(player_id - 1).expect("bounded player index");
                let phase = ((tick / 300) + u64::from(player_id)) % 4;
                let (x, z) = match phase {
                    0 => (1, 0),
                    1 => (0, 1),
                    2 => (-1, 0),
                    _ => (0, -1),
                };
                game.apply_command(
                    PlayerCommand::new(
                        player_id,
                        sequences[index],
                        ArpgCommand::SetMovement { x, z },
                    )
                    .expect("movement command sequence is valid"),
                )
                .expect("movement command must remain valid");
                sequences[index] += 1;
            }
        }

        if tick % 90 == 0 {
            for player_id in 1..=4 {
                let index = usize::try_from(player_id - 1).expect("bounded player index");
                game.apply_command(
                    PlayerCommand::new(player_id, sequences[index], ArpgCommand::PrimaryAttack)
                        .expect("attack command sequence is valid"),
                )
                .expect("attack command must remain valid");
                sequences[index] += 1;
            }
        }

        game.advance_tick().expect("ARPG tick must remain valid");
        if tick % 60 == 0 {
            let snapshot = game.snapshot().expect("ARPG snapshot must remain valid");
            assert_eq!(snapshot.tick, tick + 1);
        }
    }

    let snapshot = game
        .snapshot()
        .expect("final ARPG snapshot must remain valid");
    (snapshot, game.physics_steps(), OperationWork::take())
}

fn journey() {
    let mut elapsed_ns = Vec::with_capacity(RUNS);
    let mut expected = None;

    for _ in 0..RUNS {
        let started = Instant::now();
        let (snapshot, steps, work) = run_journey();
        elapsed_ns.push(started.elapsed().as_nanos());
        assert!(
            steps <= MAX_PHYSICS_STEPS,
            "ARPG performance journey stepped physics {steps} times, budget is {MAX_PHYSICS_STEPS}"
        );
        if let Some(reference) = &expected {
            assert_eq!(
                &(snapshot, steps, work),
                reference,
                "ARPG performance journey became nondeterministic"
            );
        } else {
            expected = Some((snapshot, steps, work));
        }
    }

    elapsed_ns.sort_unstable();
    let (snapshot, physics_steps, work) = expected.expect("at least one run");
    println!(
        "scenario=four-player-combat ticks={TICKS} runs={RUNS} median_elapsed_ns={} physics_steps={physics_steps} physics_steps_budget={MAX_PHYSICS_STEPS} snapshot_materializations={} monster_target_evaluations={} players={} monsters={} static_colliders={} deterministic=true timing=advisory-shared-runner",
        elapsed_ns[RUNS / 2],
        work.snapshot_materializations,
        work.monster_target_evaluations,
        snapshot.players.len(),
        snapshot.monsters.len(),
        snapshot.static_colliders.len(),
    );
}

fn dimension(dimensions: &Dimensions, name: &str) -> Result<u32, String> {
    let value = dimensions
        .get(name)
        .ok_or_else(|| format!("scale point lacks dimension {name}"))?;
    u32::try_from(*value).map_err(|_| format!("dimension {name} is out of range"))
}

fn build(players: u32, engaged_monsters: u32, unrelated_monsters: u32) -> Result<ArpgGame, String> {
    let game = WorkFixture {
        players,
        engaged_monsters,
        unrelated_monsters,
    }
    .build(SEED)
    .map_err(|error| error.to_string())?;
    // Building a fixture is not part of any measured operation.
    OperationWork::take();
    Ok(game)
}

fn metrics<const N: usize>(entries: [(&str, u64); N]) -> Metrics {
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

/// Sixty ticks without a snapshot request, with explicit command cadence.
fn ordinary_tick(dimensions: &Dimensions) -> Result<(Metrics, Value), String> {
    let players = dimension(dimensions, "players")?;
    let monsters = dimension(dimensions, "monsters")?;
    let unrelated = monsters
        .checked_sub(ENGAGED_MONSTERS)
        .ok_or("ordinary-tick needs at least the engaged monsters")?;
    let mut game = build(players, ENGAGED_MONSTERS, unrelated)?;
    let mut sequence = 1;
    let mut most_steps = 0;
    let mut mismatched_ticks = 0;
    for tick in 0..WINDOW_TICKS {
        if tick % COMMAND_INTERVAL_TICKS == 0 {
            let x = if (tick / COMMAND_INTERVAL_TICKS).is_multiple_of(2) {
                1
            } else {
                -1
            };
            for player_id in 1..=players {
                let command =
                    PlayerCommand::new(player_id, sequence, ArpgCommand::SetMovement { x, z: 0 })
                        .map_err(|error| error.to_string())?;
                game.apply_command(command)
                    .map_err(|error| error.to_string())?;
            }
            sequence += 1;
        }
        let before = game.physics_steps();
        game.advance_tick().map_err(|error| error.to_string())?;
        let steps = game.physics_steps() - before;
        most_steps = most_steps.max(steps);
        mismatched_ticks += u64::from(steps != 1);
    }
    let work = OperationWork::take();
    let outcome = serde_json::to_value(game.snapshot().map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    Ok((
        metrics([
            ("physics_steps_per_tick", most_steps),
            ("ticks_without_one_physics_step", mismatched_ticks),
            ("snapshot_materializations", work.snapshot_materializations),
            (
                "snapshot_entities_materialized",
                work.snapshot_entities_materialized,
            ),
        ]),
        outcome,
    ))
}

/// One explicitly requested snapshot.
fn requested_snapshot(dimensions: &Dimensions) -> Result<(Metrics, Value), String> {
    let monsters = dimension(dimensions, "monsters")?;
    let unrelated = monsters
        .checked_sub(ENGAGED_MONSTERS)
        .ok_or("requested-snapshot needs at least the engaged monsters")?;
    let mut game = build(PLAYERS, ENGAGED_MONSTERS, unrelated)?;
    game.advance_tick().map_err(|error| error.to_string())?;
    OperationWork::take();
    let before = game.physics_steps();
    let snapshot = game.snapshot().map_err(|error| error.to_string())?;
    let work = OperationWork::take();
    let outcome = serde_json::to_value(snapshot).map_err(|error| error.to_string())?;
    Ok((
        metrics([
            ("snapshot_materializations", work.snapshot_materializations),
            (
                "snapshot_entities_materialized",
                work.snapshot_entities_materialized,
            ),
            ("physics_steps", game.physics_steps() - before),
            (
                "monster_target_evaluations",
                work.monster_target_evaluations,
            ),
        ]),
        outcome,
    ))
}

/// Sixty ticks of engagement decisions; the largest per-tick work is the sample.
fn monster_targeting(dimensions: &Dimensions) -> Result<(Metrics, Value), String> {
    let players = dimension(dimensions, "players")?;
    let engaged = dimension(dimensions, "engagedMonsters")?;
    let unrelated = dimension(dimensions, "unrelatedMonsters")?;
    let mut game = build(players, engaged, unrelated)?;
    let mut decisions = 0;
    let mut evaluations = 0;
    for tick in 0..WINDOW_TICKS {
        game.advance_tick().map_err(|error| error.to_string())?;
        let work = OperationWork::take();
        // The intended boundary: free monsters of the active encounter score living players.
        if work.monster_decisions > u64::from(engaged)
            || work.monster_target_evaluations > work.monster_decisions * u64::from(players)
        {
            return Err(format!(
                "tick {tick}: {} decisions scored {} candidates; the boundary is {engaged} engaged monsters times {players} players",
                work.monster_decisions, work.monster_target_evaluations
            ));
        }
        decisions = decisions.max(work.monster_decisions);
        evaluations = evaluations.max(work.monster_target_evaluations);
    }
    let outcome = serde_json::to_value(game.snapshot().map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    Ok((
        metrics([
            ("monster_decisions_per_tick", decisions),
            ("monster_target_evaluations_per_tick", evaluations),
        ]),
        outcome,
    ))
}

/// One movement command for one player.
fn player_command(dimensions: &Dimensions) -> Result<(Metrics, Value), String> {
    let unrelated = dimension(dimensions, "unrelatedMonsters")?;
    let mut game = build(PLAYERS, ENGAGED_MONSTERS, unrelated)?;
    game.advance_tick().map_err(|error| error.to_string())?;
    OperationWork::take();
    let before = game.physics_steps();
    let command = PlayerCommand::new(1, 1, ArpgCommand::SetMovement { x: 1, z: 0 })
        .map_err(|error| error.to_string())?;
    game.apply_command(command)
        .map_err(|error| error.to_string())?;
    let work = OperationWork::take();
    let outcome = serde_json::to_value(game.snapshot().map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    Ok((
        metrics([
            ("command_bookkeeping_steps", work.command_bookkeeping_steps),
            ("physics_steps", game.physics_steps() - before),
            ("snapshot_materializations", work.snapshot_materializations),
            (
                "monster_target_evaluations",
                work.monster_target_evaluations,
            ),
        ]),
        outcome,
    ))
}

type Runner = fn(&Dimensions) -> Result<(Metrics, Value), String>;

fn runner(operation: &str) -> Option<Runner> {
    Some(match operation {
        "ordinary-tick" => ordinary_tick,
        "requested-snapshot" => requested_snapshot,
        "monster-targeting" => monster_targeting,
        "player-command" => player_command,
        _ => return None,
    })
}

/// Runs a scale point twice: the counters and the authoritative outcome must replay.
fn measure(operation: &str, dimensions: &Dimensions) -> Result<Metrics, String> {
    let run = runner(operation).ok_or("no runner measures this operation")?;
    let first = run(dimensions)?;
    let second = run(dimensions)?;
    if first != second {
        return Err("scale point did not replay identically".to_owned());
    }
    Ok(first.0)
}

fn main() {
    journey();
    match check_work_contract(CONTRACT, &mut measure) {
        Ok(evidence) => {
            for operation in evidence["operations"].as_array().into_iter().flatten() {
                for sample in operation["samples"].as_array().into_iter().flatten() {
                    println!(
                        "operation={} dimensions={} metrics={}",
                        operation["id"].as_str().unwrap_or_default(),
                        sample["dimensions"],
                        sample["metrics"]
                    );
                }
            }
            println!("{evidence}");
        }
        Err(failures) => {
            for failure in failures {
                eprintln!("operation work contract violated: {failure}");
            }
            std::process::exit(1);
        }
    }
}
