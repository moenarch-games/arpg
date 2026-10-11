use std::time::Instant;

use arpg_core::{ArpgCommand, ArpgGame, ArpgSnapshot, AuthoritativeGame, PlayerCommand};

const RUNS: usize = 3;
const TICKS: u64 = 3_600;
const SNAPSHOT_INTERVAL_TICKS: u64 = 60;
const ATTACK_INTERVAL_TICKS: u64 = 90;
const MOVEMENT_INTERVAL_TICKS: u64 = 300;
const PLAYERS: u32 = 4;
const RUN_SEED: u32 = 0xA420_0916;

#[derive(Clone, Debug, Eq, PartialEq)]
struct DeterministicWork {
    snapshot: ArpgSnapshot,
    commands_submitted: u64,
    snapshots_requested: u64,
    snapshot_entities_materialized: u64,
}

fn run_journey() -> DeterministicWork {
    let mut game = ArpgGame::new_with_seed(RUN_SEED).expect("performance fixture must construct");
    for player_id in 1..=PLAYERS {
        game.add_player(player_id)
            .expect("performance player fixture must be valid");
    }

    let mut sequences = [1_u32; PLAYERS as usize];
    let mut commands_submitted = 0_u64;
    let mut snapshots_requested = 0_u64;
    let mut snapshot_entities_materialized = 0_u64;
    let mut final_snapshot = None;

    for tick in 0..TICKS {
        if tick % MOVEMENT_INTERVAL_TICKS == 0 {
            for player_id in 1..=PLAYERS {
                let index = usize::try_from(player_id - 1).expect("bounded player index");
                let phase = ((tick / MOVEMENT_INTERVAL_TICKS) + u64::from(player_id)) % 4;
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
                    .expect("movement sequence must be valid"),
                )
                .expect("movement command must stay valid");
                sequences[index] += 1;
                commands_submitted += 1;
            }
        }

        if tick % ATTACK_INTERVAL_TICKS == 0 {
            for player_id in 1..=PLAYERS {
                let index = usize::try_from(player_id - 1).expect("bounded player index");
                game.apply_command(
                    PlayerCommand::new(
                        player_id,
                        sequences[index],
                        ArpgCommand::PrimaryAttack { aim: None },
                    )
                    .expect("attack sequence must be valid"),
                )
                .expect("attack command must stay valid");
                sequences[index] += 1;
                commands_submitted += 1;
            }
        }

        game.advance_tick()
            .expect("performance tick must stay valid");
        if (tick + 1) % SNAPSHOT_INTERVAL_TICKS == 0 {
            let snapshot = game.snapshot().expect("sampled snapshot must stay valid");
            assert_eq!(snapshot.tick, tick + 1);
            snapshots_requested += 1;
            snapshot_entities_materialized += u64::try_from(
                snapshot.rooms.len()
                    + snapshot.doors.len()
                    + snapshot.players.len()
                    + snapshot.monsters.len()
                    + snapshot.static_colliders.len(),
            )
            .expect("snapshot entity count must fit u64");
            final_snapshot = Some(snapshot);
        }
    }

    DeterministicWork {
        snapshot: final_snapshot.expect("final interval must produce a snapshot"),
        commands_submitted,
        snapshots_requested,
        snapshot_entities_materialized,
    }
}

fn main() {
    let mut elapsed_ns = Vec::with_capacity(RUNS);
    let mut expected = None;

    for _ in 0..RUNS {
        let started = Instant::now();
        let work = run_journey();
        elapsed_ns.push(started.elapsed().as_nanos());
        if let Some(reference) = &expected {
            assert_eq!(
                &work, reference,
                "ARPG performance workload became nondeterministic"
            );
        } else {
            expected = Some(work);
        }
    }

    elapsed_ns.sort_unstable();
    let work = expected.expect("performance probe needs at least one run");
    let median_elapsed_ns = elapsed_ns[RUNS / 2];

    println!(
        "{{\"scenario\":\"arpg/product-composition-v1\",\"seed\":{RUN_SEED},\"players\":{PLAYERS},\"ticks\":{TICKS},\"runs\":{RUNS},\"commands_submitted\":{},\"snapshots_requested\":{},\"snapshot_entities_materialized\":{},\"final_players\":{},\"final_monsters\":{},\"median_elapsed_ns\":{median_elapsed_ns},\"deterministic\":true}}",
        work.commands_submitted,
        work.snapshots_requested,
        work.snapshot_entities_materialized,
        work.snapshot.players.len(),
        work.snapshot.monsters.len(),
    );
}
