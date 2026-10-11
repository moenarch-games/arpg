//! Bow transitions on the reliable command lane (#143, Codex review on #151).
//!
//! A release consumes the draw it shoots. As a datagram, a release that is lost or
//! overtaken by a newer datagram is never applied; if the player draws again before the
//! held refresh repeats it, the authority keeps the first draw and the next release fires
//! one shot for two intended releases. The browser therefore sends `DrawBow`, `ReleaseBow`
//! and `CancelBow` on the reliable lane, where they are lossless and ordered among
//! themselves. These tests drive the real ARPG simulation through `MatchRuntime`.

use arpg_core::{
    ActionKind, ArpgCommand, ArpgGame, ArpgSnapshot, AuthoritativeGame, ScenarioId, Weapon,
    WorkbenchOperation,
};
use arpg_game_server::GameServerAdapter;
use arpg_protocol::{JsonProtocol, WireProtocol};
use game_server::{
    CommandLane, CommandOutcome, MatchRuntime, RECONNECT_TOKEN_BYTES, ReconnectToken, RuntimeError,
    SessionLease,
};

const SEED: u32 = 0x0143_B0B0;
const RECONNECT_GRACE_TICKS: u64 = 120;
/// Longer than the full draw.
const DRAW_TICKS: u64 = 40;
/// Longer than a shot, release to recovery.
const ACTION_TICKS: u64 = 90;
const DATAGRAM: CommandLane = CommandLane::Datagram;
const RELIABLE: CommandLane = CommandLane::Reliable;
const STAND: ArpgCommand = ArpgCommand::SetMovement { x: 0, z: 0 };

fn token(byte: u8) -> ReconnectToken {
    ReconnectToken([byte; RECONNECT_TOKEN_BYTES])
}

struct Dedicated {
    runtime: MatchRuntime<GameServerAdapter<ArpgGame, JsonProtocol>>,
    lease: SessionLease,
    published: Vec<ArpgSnapshot>,
}

impl Dedicated {
    /// A player holding the bow, applied on the reliable lane with sequence 1.
    fn archer() -> Self {
        // The dummy scenario without its target, so nothing interrupts a draw.
        let mut game = ArpgGame::new_scenario(ScenarioId::Dummy, SEED).unwrap();
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
        let mut runtime = MatchRuntime::new(
            GameServerAdapter::new(game, JsonProtocol),
            RECONNECT_GRACE_TICKS,
        );
        let lease = runtime.admit(token(1)).unwrap();
        let mut server = Self {
            runtime,
            lease,
            published: Vec::new(),
        };
        let equip = ArpgCommand::EquipWeapon {
            weapon: Weapon::Bow,
        };
        assert_eq!(
            server.submit(RELIABLE, 1, equip),
            Ok(CommandOutcome::Applied)
        );
        server
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

    fn applied(&mut self, lane: CommandLane, sequence: u32, command: ArpgCommand) {
        assert_eq!(
            self.submit(lane, sequence, command),
            Ok(CommandOutcome::Applied)
        );
    }

    fn advance(&mut self, ticks: u64) {
        for _ in 0..ticks {
            let snapshot = self.runtime.advance_tick().unwrap();
            self.published
                .push(JsonProtocol.decode_snapshot(&snapshot.payload).unwrap());
        }
    }

    /// Shots the player fired: each start of a `Shoot` action in the published snapshots.
    fn shots(&self) -> usize {
        let id = self.lease.player_id;
        let shooting = self.published.iter().map(|snapshot| {
            snapshot
                .players
                .iter()
                .find(|player| player.id == id)
                .and_then(|player| player.action)
                .is_some_and(|action| action.kind == ActionKind::Shoot)
        });
        let mut previous = false;
        let mut shots = 0;
        for now in shooting {
            if now && !previous {
                shots += 1;
            }
            previous = now;
        }
        shots
    }
}

/// Codex's scenario on the reliable lane: the first release is delayed behind a newer held
/// datagram, then the player draws and releases again. The lane delivers the release before
/// the second draw, so two releases fire two shots.
#[test]
fn a_delayed_release_then_a_new_draw_and_release_fires_two_shots() {
    let mut server = Dedicated::archer();
    server.applied(RELIABLE, 2, ArpgCommand::DrawBow);
    server.advance(DRAW_TICKS);
    // Sequence 3 is the release; the held datagram with sequence 4 reaches the authority
    // first.
    server.applied(DATAGRAM, 4, STAND);
    server.applied(RELIABLE, 3, ArpgCommand::ReleaseBow { aim: None });
    server.advance(ACTION_TICKS);
    server.applied(RELIABLE, 5, ArpgCommand::DrawBow);
    server.advance(DRAW_TICKS);
    server.applied(RELIABLE, 6, ArpgCommand::ReleaseBow { aim: None });
    server.advance(ACTION_TICKS);
    assert_eq!(server.shots(), 2);
}

/// The defect the lane fixes: as datagrams, a lost release leaves the first draw in place,
/// the new draw changes nothing, and the second release fires one shot for two releases.
/// A late or reordered copy of the lost release would be stale by then.
#[test]
fn a_lost_release_on_the_datagram_lane_merges_two_shots_into_one() {
    let mut server = Dedicated::archer();
    server.applied(DATAGRAM, 2, ArpgCommand::DrawBow);
    server.advance(DRAW_TICKS);
    // Sequence 3, the release, is lost.
    server.advance(ACTION_TICKS);
    server.applied(DATAGRAM, 4, ArpgCommand::DrawBow);
    server.advance(DRAW_TICKS);
    server.applied(DATAGRAM, 5, ArpgCommand::ReleaseBow { aim: None });
    assert_eq!(
        server.submit(DATAGRAM, 3, ArpgCommand::ReleaseBow { aim: None }),
        Ok(CommandOutcome::IgnoredStale)
    );
    server.advance(ACTION_TICKS);
    assert_eq!(
        server.shots(),
        1,
        "two intended releases merged into one shot"
    );
}

/// A release applied just before an outage is re-sent after the resume because its ack was
/// lost. It is a duplicate, and a following draw and release fire exactly one more shot.
#[test]
fn a_release_re_sent_after_reconnect_shoots_once() {
    let mut server = Dedicated::archer();
    server.applied(RELIABLE, 2, ArpgCommand::DrawBow);
    server.advance(DRAW_TICKS);
    server.applied(RELIABLE, 3, ArpgCommand::ReleaseBow { aim: None });

    let previous = server.lease;
    assert!(
        server
            .runtime
            .disconnect(previous.player_id, previous.connection_epoch)
    );
    server.advance(ACTION_TICKS);
    server.lease = server
        .runtime
        .reconnect(previous.reconnect_token, token(2))
        .unwrap();
    assert_eq!(
        server.submit(RELIABLE, 3, ArpgCommand::ReleaseBow { aim: None }),
        Ok(CommandOutcome::IgnoredStale)
    );
    assert_eq!(server.shots(), 1);

    server.applied(RELIABLE, 4, ArpgCommand::DrawBow);
    server.advance(DRAW_TICKS);
    server.applied(RELIABLE, 5, ArpgCommand::ReleaseBow { aim: None });
    server.advance(ACTION_TICKS);
    assert_eq!(server.shots(), 2);
}
