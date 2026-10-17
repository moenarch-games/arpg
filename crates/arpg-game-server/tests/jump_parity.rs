//! Jumping (#59) gives the same outcomes locally and on the dedicated authority.
//!
//! Locally every command takes the datagram lane, as in local and peer play. The dedicated
//! client sends held state (movement, guard) as datagrams and one-shots (jump, attacks) on
//! the reliable command lane, all under one strictly increasing sequence counter per
//! player. Rules never depend on the lane, so both runs publish identical snapshots, tick
//! for tick, through takeoffs, buffered jumps, airborne attacks, a lowered guard and
//! landings.

use arpg_core::{
    ArpgCommand, ArpgGame, ArpgSnapshot, AuthoritativeGame, PlayerCommand, PlayerId, ScenarioId,
};
use arpg_game_server::GameServerAdapter;
use arpg_protocol::{JsonProtocol, WireProtocol};
use game_server::{CommandLane, MatchRuntime, RECONNECT_TOKEN_BYTES, ReconnectToken};

const SEED: u32 = 0x0059_1EAF;
const TICKS: u64 = 150;

/// The commands each player sends before a tick.
fn script(tick: u64) -> Vec<(PlayerId, ArpgCommand)> {
    match tick {
        0 => vec![
            (1, ArpgCommand::SetMovement { x: 1, z: 0 }),
            (1, ArpgCommand::Jump),
            (2, ArpgCommand::SetGuard { raised: true }),
        ],
        // Player 1 attacks at the top of its jump; player 2 jumps with its guard held.
        12 => vec![
            (1, ArpgCommand::PrimaryAttack { aim: None }),
            (2, ArpgCommand::Jump),
        ],
        // A press shortly before player 1 lands (sliding off the monster's shoulder, it
        // lands at tick 38) is buffered into a second jump.
        35 => vec![(1, ArpgCommand::Jump)],
        40 => vec![(2, ArpgCommand::SetMovement { x: -1, z: 1 })],
        // Repeated presses in one tick jump once.
        60 => vec![(2, ArpgCommand::Jump), (2, ArpgCommand::Jump)],
        70 => vec![
            (1, ArpgCommand::SetMovement { x: 0, z: 0 }),
            (1, ArpgCommand::SecondaryAttack { aim: None }),
        ],
        110 => vec![
            (1, ArpgCommand::Jump),
            (2, ArpgCommand::SetGuard { raised: false }),
        ],
        _ => Vec::new(),
    }
}

fn lane(command: &ArpgCommand) -> CommandLane {
    match command {
        ArpgCommand::SetMovement { .. }
        | ArpgCommand::SetGuard { .. }
        | ArpgCommand::SetAim { .. } => CommandLane::Datagram,
        _ => CommandLane::Reliable,
    }
}

#[test]
fn jumping_matches_between_local_and_dedicated_execution() {
    let mut local = ArpgGame::new_scenario(ScenarioId::Enemy, SEED).unwrap();
    local.add_player(1).unwrap();
    local.add_player(2).unwrap();
    let mut sequences = [0_u32; 3];
    let mut local_snapshots = Vec::new();
    for tick in 0..TICKS {
        for (player_id, command) in script(tick) {
            let slot = player_id as usize;
            sequences[slot] += 1;
            local
                .apply_command(PlayerCommand::new(player_id, sequences[slot], command).unwrap())
                .unwrap();
        }
        local.advance_tick().unwrap();
        local_snapshots.push(local.snapshot().unwrap());
    }

    let adapter = GameServerAdapter::new(
        ArpgGame::new_scenario(ScenarioId::Enemy, SEED).unwrap(),
        JsonProtocol,
    );
    let mut runtime = MatchRuntime::new(adapter, 120);
    let leases = [7, 8].map(|byte| {
        runtime
            .admit(ReconnectToken([byte; RECONNECT_TOKEN_BYTES]))
            .unwrap()
    });
    assert_eq!(leases.map(|lease| lease.player_id), [1, 2]);
    let mut sequences = [0_u32; 3];
    let mut server_snapshots: Vec<ArpgSnapshot> = Vec::new();
    for tick in 0..TICKS {
        for (player_id, command) in script(tick) {
            let slot = player_id as usize;
            sequences[slot] += 1;
            let lease = leases[slot - 1];
            runtime
                .submit_lane_command(
                    lease.player_id,
                    lease.connection_epoch,
                    lane(&command),
                    sequences[slot],
                    &JsonProtocol.encode_command(&command).unwrap(),
                )
                .unwrap();
        }
        runtime.advance_tick().unwrap();
        let snapshot = runtime.snapshot().unwrap();
        server_snapshots.push(JsonProtocol.decode_snapshot(&snapshot.payload).unwrap());
    }

    for (tick, (server, local)) in server_snapshots.iter().zip(&local_snapshots).enumerate() {
        assert_eq!(server, local, "tick {tick}");
    }
    // The script exercised what it claims: both players flew, player 1 twice in a row (the
    // buffered press), and attacked while airborne.
    for player_id in [1, 2] {
        let airborne = local_snapshots
            .iter()
            .filter(|snapshot| {
                snapshot
                    .players
                    .iter()
                    .any(|player| player.id == player_id && !player.motion.grounded)
            })
            .count();
        assert!(
            airborne > 30,
            "player {player_id} airborne for {airborne} ticks"
        );
    }
    let player_one = |tick: usize| {
        local_snapshots[tick]
            .players
            .iter()
            .find(|player| player.id == 1)
            .unwrap()
            .clone()
    };
    assert!(!player_one(14).motion.grounded && player_one(14).action.is_some());
    let takeoffs = (1..local_snapshots.len())
        .filter(|&tick| player_one(tick - 1).motion.grounded && !player_one(tick).motion.grounded)
        .count();
    assert_eq!(
        takeoffs, 2,
        "takeoffs after the first: the buffered and the late jump"
    );
}
