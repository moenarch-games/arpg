//! Acceptance for per-lane command watermarks (#143).
//!
//! Dedicated play delivers commands on two `game-server` lanes: held movement, aim, guard
//! and bow state as best-effort datagrams, and one-shot commands (attacks, interact,
//! weapon and target changes) on the reliable command lane. `game_server::MatchRuntime`
//! keeps one sequence watermark per (player, lane), so a reliable command may reach the
//! authority after a newer datagram. The core must accept it, and must still apply each
//! sequence of a lane at most once.
//!
//! API contract (arpg-core, transport-neutral):
//!
//! ```ignore
//! /// The ordering lane a command arrived on. Each lane has its own per-player sequence
//! /// watermark. `Datagram` is the default lane: local play, peer play and every caller of
//! /// `apply_command` use it. `Reliable` is the ordered, acknowledged one-shot lane.
//! #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
//! pub enum CommandLane { Datagram, Reliable }
//!
//! pub trait AuthoritativeGame {
//!     // ...existing methods...
//!     /// Applies a command delivered on `lane`. The default ignores the lane.
//!     fn apply_lane_command(
//!         &mut self,
//!         lane: CommandLane,
//!         command: PlayerCommand<Self::Command>,
//!     ) -> Result<(), GameError> {
//!         self.apply_command(command)
//!     }
//! }
//!
//! // ArpgGame overrides `apply_lane_command`. `apply_command(c)` is exactly
//! // `apply_lane_command(CommandLane::Datagram, c)`. A sequence at or below its own lane's
//! // watermark fails with the existing "command sequence is stale" error and changes
//! // nothing; the other lane's watermark never makes a command stale. Gameplay rules do not
//! // depend on the lane. Removing a player drops both watermarks.
//!
//! pub struct PlayerSaveState {
//!     // ...existing fields; `last_sequence` stays the datagram watermark...
//!     /// Reliable-lane watermark. Missing in older saves, which load with 0; the save
//!     /// schema version does not change. `u32::MAX` is rejected like `last_sequence`.
//!     #[serde(default)]
//!     pub last_reliable_sequence: u32, // JSON: "lastReliableSequence"
//! }
//! ```

use arpg_core::{
    ActionKind, ArpgCommand, ArpgGame, ArpgSnapshot, AuthoritativeGame, CommandLane, PlayerCommand,
    PlayerId, PlayerSnapshot,
};

const PLAYER: PlayerId = 1;
const STALE: &str = "command sequence is stale";
/// Longer than any light attack, wind-up to recovery.
const ACTION_TICKS: u64 = 90;

fn game() -> ArpgGame {
    let mut game = ArpgGame::new().expect("game");
    game.add_player(PLAYER).expect("player");
    game
}

fn command(sequence: u32, command: ArpgCommand) -> PlayerCommand<ArpgCommand> {
    PlayerCommand::new(PLAYER, sequence, command).expect("non-zero sequence")
}

/// Goes through the trait, as `arpg-game-server`'s adapter does.
fn apply<G: AuthoritativeGame<Command = ArpgCommand>>(
    game: &mut G,
    lane: CommandLane,
    sequence: u32,
    value: ArpgCommand,
) -> Result<(), String> {
    game.apply_lane_command(lane, command(sequence, value))
        .map_err(|error| error.message().to_owned())
}

fn player(snapshot: &ArpgSnapshot) -> &PlayerSnapshot {
    snapshot
        .players
        .iter()
        .find(|player| player.id == PLAYER)
        .expect("player is in the snapshot")
}

const STAND: ArpgCommand = ArpgCommand::SetMovement { x: 0, z: 0 };

/// The defect behind #143: the held datagram with sequence 2 overtook the attack with
/// sequence 1. On its own lane the attack is not stale and starts.
#[test]
fn a_reliable_one_shot_behind_a_newer_datagram_is_applied() {
    let mut game = game();
    apply(&mut game, CommandLane::Datagram, 2, STAND).unwrap();
    apply(
        &mut game,
        CommandLane::Reliable,
        1,
        ArpgCommand::PrimaryAttack,
    )
    .expect("a reliable command is judged only against the reliable watermark");

    let snapshot = game.snapshot().unwrap();
    let action = player(&snapshot).action.expect("the attack started");
    assert_eq!(action.kind, ActionKind::PrimaryAttack);
    assert_eq!(action.buffered, None);
}

/// A reliable sequence is applied at most once: a resend fails as stale and does not even
/// buffer a combo input into the running attack.
#[test]
fn a_duplicate_reliable_sequence_is_stale_and_changes_nothing() {
    let mut game = game();
    apply(
        &mut game,
        CommandLane::Reliable,
        1,
        ArpgCommand::PrimaryAttack,
    )
    .unwrap();
    let before = game.snapshot().unwrap();

    assert_eq!(
        apply(
            &mut game,
            CommandLane::Reliable,
            1,
            ArpgCommand::PrimaryAttack
        ),
        Err(STALE.to_owned())
    );
    assert_eq!(game.snapshot().unwrap(), before);

    // Once the attack is over, an applied duplicate would start a second one.
    for _ in 0..ACTION_TICKS {
        game.advance_tick().unwrap();
    }
    assert_eq!(player(&game.snapshot().unwrap()).action, None, "fixture");
    assert_eq!(
        apply(
            &mut game,
            CommandLane::Reliable,
            1,
            ArpgCommand::PrimaryAttack
        ),
        Err(STALE.to_owned())
    );
    assert_eq!(
        player(&game.snapshot().unwrap()).action,
        None,
        "a duplicate attack must not start another one"
    );

    // An older reliable sequence is stale too, whatever it carries.
    apply(
        &mut game,
        CommandLane::Reliable,
        5,
        ArpgCommand::CycleTarget,
    )
    .unwrap();
    assert_eq!(
        apply(&mut game, CommandLane::Reliable, 4, ArpgCommand::Interact),
        Err(STALE.to_owned())
    );
}

/// The two watermarks are independent in both directions, and the datagram lane keeps its
/// latest-wins behaviour.
#[test]
fn lanes_keep_independent_watermarks() {
    let mut game = game();
    apply(
        &mut game,
        CommandLane::Reliable,
        9,
        ArpgCommand::CycleTarget,
    )
    .unwrap();
    apply(&mut game, CommandLane::Datagram, 1, STAND)
        .expect("a reliable sequence never makes a datagram stale");
    apply(&mut game, CommandLane::Datagram, 3, STAND).unwrap();
    assert_eq!(
        apply(&mut game, CommandLane::Datagram, 2, STAND),
        Err(STALE.to_owned()),
        "an overtaken datagram stays stale"
    );
    apply(
        &mut game,
        CommandLane::Reliable,
        10,
        ArpgCommand::ClearTarget,
    )
    .unwrap();
}

/// `apply_command` is the datagram lane, so local and peer play are unchanged.
#[test]
fn apply_command_is_the_datagram_lane() {
    let mut game = game();
    game.apply_command(command(3, STAND)).unwrap();
    assert_eq!(
        apply(&mut game, CommandLane::Datagram, 3, STAND),
        Err(STALE.to_owned())
    );
    apply(
        &mut game,
        CommandLane::Reliable,
        1,
        ArpgCommand::ClearTarget,
    )
    .expect("apply_command does not advance the reliable watermark");

    apply(&mut game, CommandLane::Datagram, 4, STAND).unwrap();
    assert_eq!(
        game.apply_command(command(4, STAND))
            .map_err(|error| error.message().to_owned()),
        Err(STALE.to_owned())
    );
}

/// Removal drops both watermarks, like `MatchRuntime` does for a removed player.
#[test]
fn removing_a_player_resets_both_watermarks() {
    let mut game = game();
    apply(&mut game, CommandLane::Datagram, 7, STAND).unwrap();
    apply(
        &mut game,
        CommandLane::Reliable,
        7,
        ArpgCommand::ClearTarget,
    )
    .unwrap();
    assert!(game.remove_player(PLAYER));
    game.add_player(PLAYER).unwrap();
    apply(&mut game, CommandLane::Datagram, 1, STAND).unwrap();
    apply(
        &mut game,
        CommandLane::Reliable,
        1,
        ArpgCommand::ClearTarget,
    )
    .unwrap();
}

/// A save keeps both watermarks: `last_sequence` (datagram) and `last_reliable_sequence`.
#[test]
fn save_and_restore_keep_both_watermarks() {
    let mut game = game();
    apply(&mut game, CommandLane::Datagram, 7, STAND).unwrap();
    apply(
        &mut game,
        CommandLane::Reliable,
        4,
        ArpgCommand::ClearTarget,
    )
    .unwrap();

    let save = game.save_state().unwrap();
    assert_eq!(save.players[0].last_sequence, 7);
    assert_eq!(save.players[0].last_reliable_sequence, 4);
    let json = serde_json::to_value(&save).unwrap();
    assert_eq!(json["players"][0]["lastReliableSequence"], 4);

    let mut restored = ArpgGame::from_save_state(serde_json::from_value(json).unwrap()).unwrap();
    assert_eq!(restored.save_state().unwrap(), save);
    assert_eq!(
        apply(
            &mut restored,
            CommandLane::Reliable,
            4,
            ArpgCommand::ClearTarget
        ),
        Err(STALE.to_owned())
    );
    assert_eq!(
        apply(&mut restored, CommandLane::Datagram, 7, STAND),
        Err(STALE.to_owned())
    );
    apply(
        &mut restored,
        CommandLane::Reliable,
        5,
        ArpgCommand::ClearTarget,
    )
    .unwrap();
    apply(&mut restored, CommandLane::Datagram, 8, STAND).unwrap();
}

/// Saves written before the reliable lane have no `lastReliableSequence`. They still load,
/// under the same schema version, with a reliable watermark of zero.
#[test]
fn an_older_save_without_a_reliable_watermark_still_loads() {
    let mut game = game();
    apply(&mut game, CommandLane::Datagram, 7, STAND).unwrap();
    let mut json = serde_json::to_value(game.save_state().unwrap()).unwrap();
    let removed = json["players"][0]
        .as_object_mut()
        .unwrap()
        .remove("lastReliableSequence");
    assert!(removed.is_some(), "new saves write the reliable watermark");

    let mut restored = ArpgGame::from_save_state(serde_json::from_value(json).unwrap())
        .expect("an older save loads");
    let save = restored.save_state().unwrap();
    assert_eq!(save.players[0].last_sequence, 7);
    assert_eq!(save.players[0].last_reliable_sequence, 0);
    apply(
        &mut restored,
        CommandLane::Reliable,
        1,
        ArpgCommand::ClearTarget,
    )
    .unwrap();
}

/// Like `last_sequence`, an exhausted reliable watermark leaves no valid next command.
#[test]
fn an_exhausted_reliable_watermark_is_rejected() {
    let mut save = game().save_state().unwrap();
    save.players[0].last_reliable_sequence = u32::MAX;
    assert!(ArpgGame::from_save_state(save).is_err());
}
