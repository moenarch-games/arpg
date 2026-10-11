//! Reliable one-shot commands on the dedicated authority (#143).
//!
//! The browser sends one-shot commands (attacks, interact, weapon and target changes) on
//! the `game-server` reliable command lane and held state (movement, aim, guard, bow) as
//! datagrams with the #139 refresh. `MatchRuntime::submit_lane_command` keeps one
//! watermark per (player, lane) and calls `GameSimulation::apply_lane_command`. These
//! tests drive the real ARPG simulation through `MatchRuntime` and pin what the dedicated
//! authority must do with lost, reordered and re-sent commands.
//!
//! API contract (arpg-game-server):
//!
//! ```ignore
//! impl<G: AuthoritativeGame, P: WireProtocol<..>> GameSimulation for GameServerAdapter<G, P> {
//!     // New: forwards the lane to the core instead of the default `apply_command`
//!     // delegation, mapping `game_server::CommandLane::{Datagram, Reliable}` to
//!     // `arpg_core::CommandLane::{Datagram, Reliable}`.
//!     fn apply_lane_command(&mut self, player_id, lane, sequence, payload)
//!         -> Result<(), SimulationError>;
//!     // `apply_command` keeps delegating to the core's datagram lane.
//! }
//! ```
//!
//! On the wire, `game-server` acknowledges `Ok(Applied)` as `Applied` and
//! `Ok(IgnoredStale)` on the reliable lane as `Duplicate`; an `Err` that is not a
//! `command_rejected` closes the connection. So "applied after a newer datagram" means
//! `Ok(Applied)` here, never a fatal `Err`.

use arpg_core::{
    ActionKind, ActionPhase, ArpgCommand, ArpgGame, ArpgSnapshot, AuthoritativeGame, PlayerId,
    PlayerSnapshot, ScenarioId, StrikeSource, Weapon, WorkbenchOperation,
};
use arpg_game_server::GameServerAdapter;
use arpg_protocol::{JsonProtocol, WireProtocol};
use game_server::{
    CommandLane, CommandOutcome, MatchRuntime, RECONNECT_TOKEN_BYTES, ReconnectToken, RuntimeError,
    SessionLease,
};
use std::collections::BTreeSet;

const SEED: u32 = 0x0143_A11C;
const RECONNECT_GRACE_TICKS: u64 = 120;
/// Longer than any light attack, wind-up to recovery.
const ACTION_TICKS: u64 = 90;
const DATAGRAM: CommandLane = CommandLane::Datagram;
const RELIABLE: CommandLane = CommandLane::Reliable;

type Runtime = MatchRuntime<GameServerAdapter<ArpgGame, JsonProtocol>>;

fn token(byte: u8) -> ReconnectToken {
    ReconnectToken([byte; RECONNECT_TOKEN_BYTES])
}

/// The dummy scenario: the player faces a target inside its light swing.
fn dummy() -> ArpgGame {
    ArpgGame::new_scenario(ScenarioId::Dummy, SEED).unwrap()
}

/// The dummy scenario without its target, so the player can walk freely.
fn open_room() -> ArpgGame {
    let mut game = dummy();
    let room = game.workbench_room().unwrap();
    let targets: Vec<u32> = game
        .snapshot()
        .unwrap()
        .monsters
        .iter()
        .filter(|monster| monster.room_id == room)
        .map(|monster| monster.id)
        .collect();
    for monster_id in targets {
        game.apply_workbench(&WorkbenchOperation::RemoveMonster { monster_id })
            .unwrap();
    }
    game
}

fn decode(snapshot: &game_server::SimulationSnapshot) -> ArpgSnapshot {
    JsonProtocol.decode_snapshot(&snapshot.payload).unwrap()
}

fn player_of(snapshot: &ArpgSnapshot, id: PlayerId) -> &PlayerSnapshot {
    snapshot
        .players
        .iter()
        .find(|player| player.id == id)
        .expect("player is in the snapshot")
}

struct Dedicated {
    runtime: Runtime,
    lease: SessionLease,
    /// Every snapshot published since the start, decoded as a client does.
    published: Vec<ArpgSnapshot>,
}

impl Dedicated {
    fn new(game: ArpgGame) -> Self {
        Self::from_runtime(MatchRuntime::new(
            GameServerAdapter::new(game, JsonProtocol),
            RECONNECT_GRACE_TICKS,
        ))
    }

    fn from_runtime(mut runtime: Runtime) -> Self {
        let lease = runtime.admit(token(1)).unwrap();
        Self {
            runtime,
            lease,
            published: Vec::new(),
        }
    }

    fn submit(
        &mut self,
        lane: CommandLane,
        sequence: u32,
        command: ArpgCommand,
    ) -> Result<CommandOutcome, RuntimeError> {
        let payload = JsonProtocol.encode_command(&command).unwrap();
        self.runtime.submit_lane_command(
            self.lease.player_id,
            self.lease.connection_epoch,
            lane,
            sequence,
            &payload,
        )
    }

    fn advance(&mut self, ticks: u64) {
        for _ in 0..ticks {
            let snapshot = self.runtime.advance_tick().unwrap();
            self.published.push(decode(&snapshot));
        }
    }

    fn view(&self) -> ArpgSnapshot {
        decode(&self.runtime.snapshot().unwrap())
    }

    fn player(&self) -> PlayerSnapshot {
        player_of(&self.view(), self.lease.player_id).clone()
    }

    /// Distinct ticks at which this player's strikes resolved: one per attack.
    fn player_strike_ticks(&self) -> BTreeSet<u64> {
        let id = self.lease.player_id;
        self.published
            .iter()
            .flat_map(|snapshot| &snapshot.strike_events)
            .filter(|event| event.source == StrikeSource::Player(id))
            .map(|event| event.strike_tick)
            .collect()
    }

    /// Whether the player's action ever carried a buffered combo input, which a duplicate
    /// attack applied during the running attack would leave.
    fn ever_buffered(&self) -> bool {
        let id = self.lease.player_id;
        self.published.iter().any(|snapshot| {
            player_of(snapshot, id)
                .action
                .is_some_and(|action| action.buffered.is_some())
        })
    }

    /// Drops the connection, waits out an in-grace outage and resumes the same player.
    fn outage_and_resume(&mut self, ticks: u64, replacement: u8) {
        let previous = self.lease;
        assert!(
            self.runtime
                .disconnect(previous.player_id, previous.connection_epoch)
        );
        self.advance(ticks);
        self.lease = self
            .runtime
            .reconnect(previous.reconnect_token, token(replacement))
            .unwrap();
        assert_eq!(self.lease.player_id, previous.player_id);
    }
}

/// The #143 defect: the held datagram with sequence 2 overtook the attack with sequence 1.
/// On the reliable lane the attack is applied (today the core calls it stale, which the
/// runtime turns into a fatal error that closes the connection) and lands once.
#[test]
fn a_reliable_attack_overtaken_by_a_newer_held_datagram_is_applied() {
    let mut server = Dedicated::new(dummy());
    assert_eq!(
        server.submit(DATAGRAM, 2, ArpgCommand::SetMovement { x: 0, z: 0 }),
        Ok(CommandOutcome::Applied)
    );
    assert_eq!(
        server.submit(RELIABLE, 1, ArpgCommand::PrimaryAttack),
        Ok(CommandOutcome::Applied),
        "a reliable one-shot is judged only against the reliable watermark"
    );
    assert_eq!(
        server.player().action.map(|action| action.kind),
        Some(ActionKind::PrimaryAttack)
    );
    server.advance(ACTION_TICKS);
    assert_eq!(
        server.player_strike_ticks().len(),
        1,
        "the attack struck once"
    );
}

/// An attack applied before an outage is re-sent after the reconnect because its ack was
/// lost. The runtime reports it as a duplicate (acked `Duplicate`), whether the resend
/// arrives during the attack or after it, and the player attacks exactly once. The next
/// one-shot still applies, so the reliable watermark is carried, not stuck.
#[test]
fn a_one_shot_re_sent_after_reconnect_is_a_duplicate_and_attacks_once() {
    let mut server = Dedicated::new(dummy());
    assert_eq!(
        server.submit(RELIABLE, 1, ArpgCommand::PrimaryAttack),
        Ok(CommandOutcome::Applied)
    );
    // Past the wind-up, where a second attack input would buffer a combo follow-up.
    while server.player().action.expect("the attack runs").phase == ActionPhase::Windup {
        server.advance(1);
    }
    server.outage_and_resume(1, 2);
    assert!(
        server
            .player()
            .action
            .is_some_and(|action| action.phase != ActionPhase::Windup),
        "fixture: the attack is still running and accepts combo input"
    );
    assert_eq!(
        server.submit(RELIABLE, 1, ArpgCommand::PrimaryAttack),
        Ok(CommandOutcome::IgnoredStale),
        "a resend during the attack is a duplicate"
    );
    server.advance(ACTION_TICKS);
    assert_eq!(
        server.submit(RELIABLE, 1, ArpgCommand::PrimaryAttack),
        Ok(CommandOutcome::IgnoredStale),
        "a resend after the attack is a duplicate"
    );
    server.advance(ACTION_TICKS);
    assert_eq!(server.player_strike_ticks().len(), 1, "exactly one attack");
    assert!(!server.ever_buffered(), "a duplicate queued a combo input");

    assert_eq!(
        server.submit(RELIABLE, 2, ArpgCommand::PrimaryAttack),
        Ok(CommandOutcome::Applied)
    );
    server.advance(ACTION_TICKS);
    assert_eq!(server.player_strike_ticks().len(), 2);
}

/// A one-shot sent just before the link died was never applied. After the reconnect the
/// client's held refresh may land first with newer sequences; the re-sent one-shot is
/// still applied, and only once.
#[test]
fn an_unacknowledged_one_shot_lands_after_reconnect_behind_newer_held_datagrams() {
    let mut server = Dedicated::new(dummy());
    assert_eq!(
        server.submit(DATAGRAM, 1, ArpgCommand::SetGuard { raised: false }),
        Ok(CommandOutcome::Applied)
    );
    // Sequence 2 (the attack) is lost with the connection.
    server.outage_and_resume(5, 2);
    for (sequence, command) in [
        (3, ArpgCommand::SetGuard { raised: false }),
        (4, ArpgCommand::SetMovement { x: 0, z: 0 }),
        (5, ArpgCommand::SetAim { direction: None }),
    ] {
        assert_eq!(
            server.submit(DATAGRAM, sequence, command),
            Ok(CommandOutcome::Applied)
        );
    }
    assert_eq!(
        server.submit(RELIABLE, 2, ArpgCommand::PrimaryAttack),
        Ok(CommandOutcome::Applied)
    );
    assert_eq!(
        server.submit(RELIABLE, 2, ArpgCommand::PrimaryAttack),
        Ok(CommandOutcome::IgnoredStale)
    );
    server.advance(ACTION_TICKS);
    assert_eq!(server.player_strike_ticks().len(), 1);
}

/// Every one-shot kind is accepted on the reliable lane behind newer datagrams, with its
/// ordinary gameplay effect.
#[test]
fn every_one_shot_kind_is_applied_on_the_reliable_lane_behind_newer_datagrams() {
    let mut server = Dedicated::new(dummy());
    assert_eq!(
        server.submit(DATAGRAM, 100, ArpgCommand::SetMovement { x: 0, z: 0 }),
        Ok(CommandOutcome::Applied)
    );
    // Encounters activate on the first tick; only then is the dummy lockable.
    server.advance(1);
    let mut sequence = 0;
    let mut send = |server: &mut Dedicated, command: ArpgCommand| {
        sequence += 1;
        assert_eq!(
            server.submit(RELIABLE, sequence, command),
            Ok(CommandOutcome::Applied),
            "{command:?}"
        );
    };

    send(&mut server, ArpgCommand::CycleTarget);
    assert!(
        server.player().locked_monster_id.is_some(),
        "the dummy is locked"
    );
    send(&mut server, ArpgCommand::ClearTarget);
    assert_eq!(server.player().locked_monster_id, None);

    send(
        &mut server,
        ArpgCommand::EquipWeapon {
            weapon: Weapon::Bow,
        },
    );
    assert_eq!(server.player().weapon, Weapon::Bow);
    send(
        &mut server,
        ArpgCommand::EquipWeapon {
            weapon: Weapon::SwordAndShield,
        },
    );
    assert_eq!(server.player().weapon, Weapon::SwordAndShield);

    send(&mut server, ArpgCommand::PrimaryAttack);
    assert_eq!(
        server.player().action.map(|action| action.kind),
        Some(ActionKind::PrimaryAttack)
    );
    server.advance(ACTION_TICKS);
    send(&mut server, ArpgCommand::SecondaryAttack);
    assert_eq!(
        server.player().action.map(|action| action.kind),
        Some(ActionKind::SecondaryAttack)
    );
    server.advance(ACTION_TICKS);
    send(&mut server, ArpgCommand::Interact);
    assert_eq!(
        server.player().action.map(|action| action.kind),
        Some(ActionKind::Interact)
    );
}

/// Held state stays on datagrams with latest-wins semantics: exactly one lost movement
/// datagram is repaired by the next refresh, and a reordered older release that arrives
/// after a newer one is stale and changes nothing. A one-shot between them on the reliable
/// lane is unaffected by either.
#[test]
fn held_movement_converges_through_the_refresh_after_one_lost_and_one_reordered_datagram() {
    let mut server = Dedicated::new(open_room());
    let start = server.player().position;

    // Sequence 1 (press right) is lost. Nothing moves.
    server.advance(10);
    assert_eq!(server.player().position, start);

    // A one-shot on the reliable lane, with a sequence between the held ones.
    assert_eq!(
        server.submit(RELIABLE, 2, ArpgCommand::ClearTarget),
        Ok(CommandOutcome::Applied)
    );
    // The refresh re-asserts the held direction with a fresh sequence.
    assert_eq!(
        server.submit(DATAGRAM, 3, ArpgCommand::SetMovement { x: 1, z: 0 }),
        Ok(CommandOutcome::Applied)
    );
    server.advance(10);
    let moved = server.player().position;
    assert!(moved[0] > start[0], "the refresh repaired the lost press");

    // Release (sequence 4) and its refresh (sequence 5) arrive reordered.
    assert_eq!(
        server.submit(DATAGRAM, 5, ArpgCommand::SetMovement { x: 0, z: 0 }),
        Ok(CommandOutcome::Applied)
    );
    assert_eq!(
        server.submit(DATAGRAM, 4, ArpgCommand::SetMovement { x: 0, z: 0 }),
        Ok(CommandOutcome::IgnoredStale)
    );
    // A stale press replayed by a duplicate datagram cannot restart the walk.
    assert_eq!(
        server.submit(DATAGRAM, 3, ArpgCommand::SetMovement { x: 1, z: 0 }),
        Ok(CommandOutcome::IgnoredStale)
    );
    server.advance(30);
    let stopped = server.player().position;
    server.advance(30);
    assert_eq!(
        server.player().position,
        stopped,
        "the player stays stopped"
    );
}

/// Graceful recovery replays the log in the order commands were applied, including a
/// reliable command behind a newer datagram, and restores both watermarks: a re-sent
/// one-shot is still a duplicate on the recovered match.
#[test]
fn recovery_replays_mixed_lanes_and_keeps_both_watermarks() {
    let runtime = MatchRuntime::new_with_replay_capture(
        GameServerAdapter::new(dummy(), JsonProtocol),
        RECONNECT_GRACE_TICKS,
    );
    let mut server = Dedicated::from_runtime(runtime);
    assert_eq!(
        server.submit(DATAGRAM, 2, ArpgCommand::SetMovement { x: 0, z: 0 }),
        Ok(CommandOutcome::Applied)
    );
    assert_eq!(
        server.submit(RELIABLE, 1, ArpgCommand::PrimaryAttack),
        Ok(CommandOutcome::Applied)
    );
    server.advance(5);
    let before = server.view();

    server.runtime.freeze_for_recovery();
    let image = server.runtime.recovery_image().unwrap();
    let restored =
        MatchRuntime::restore_from_recovery(GameServerAdapter::new(dummy(), JsonProtocol), image)
            .expect("the recovered simulation replays the mixed-lane log");
    let previous = server.lease;
    let mut recovered = Dedicated {
        runtime: restored,
        lease: previous,
        published: Vec::new(),
    };
    assert_eq!(recovered.view(), before);
    recovered.lease = recovered
        .runtime
        .reconnect(previous.reconnect_token, token(9))
        .unwrap();

    assert_eq!(
        recovered.submit(RELIABLE, 1, ArpgCommand::PrimaryAttack),
        Ok(CommandOutcome::IgnoredStale)
    );
    assert_eq!(
        recovered.submit(DATAGRAM, 2, ArpgCommand::SetMovement { x: 0, z: 0 }),
        Ok(CommandOutcome::IgnoredStale)
    );
    assert_eq!(
        recovered.submit(DATAGRAM, 3, ArpgCommand::SetMovement { x: 0, z: 0 }),
        Ok(CommandOutcome::Applied)
    );
    recovered.advance(ACTION_TICKS);
    assert!(recovered.player_strike_ticks().len() <= 1);
    assert!(!recovered.ever_buffered());
}
