//! Acceptance for aimed one-shots carrying their aim (#156).
//!
//! In dedicated play `setAim` travels as a best-effort datagram, but the commands that
//! commit a direction travel on the reliable lane. The two lanes are not ordered against
//! each other. When the aim datagram that comes just before a release or an attack is lost
//! or delayed, the authority used to shoot or strike along the stale held aim. The aim a
//! one-shot commits now travels inside the one-shot.
//!
//! API contract (arpg-core):
//!
//! ```ignore
//! pub enum ArpgCommand {
//!     // ...
//!     /// `aim`: the aim intent this attack commits; see the aim rules below.
//!     PrimaryAttack {
//!         #[serde(default)]
//!         aim: Option<[i16; 2]>,
//!     },
//!     SecondaryAttack {
//!         #[serde(default)]
//!         aim: Option<[i16; 2]>,
//!     },
//!     ReleaseBow {
//!         #[serde(default)]
//!         aim: Option<[i16; 2]>,
//!     },
//!     // SetAim, DrawBow, CancelBow, Interact, ... are unchanged.
//! }
//! ```
//!
//! The variants change from unit variants to struct variants, so this file does not compile
//! until the core implements the contract. That is expected: Rust gives no other way to
//! state a new field. `DrawBow` gets no aim because a draw commits no direction. `Counter`
//! is not a command: a `PrimaryAttack` starts it while a counter opportunity is open, so
//! the counter takes the attack's embedded aim.
//!
//! Aim rules:
//!
//! - `aim: None` (in JSON, the field is absent or `null`) keeps today's behaviour. The
//!   command uses the authority's held aim intent, or the committed facing when no aim is
//!   held.
//! - `aim: Some(d)` is validated exactly like `SetAim { direction: Some(d) }`: non-zero, and
//!   each component within ±`AIM_COMPONENT_LIMIT`. An invalid embedded aim rejects the
//!   whole command with the same `GameError` message `SetAim` returns. A rejected command
//!   has no effect: no action, no change to the held aim, and the lane's sequence watermark
//!   does not move, as with a rejected `SetAim`.
//! - A valid `Some(d)` becomes the held aim, and the facing is refreshed, before the action
//!   commits, in the same command and so in the same tick. In terms of observable state,
//!   `X { aim: Some(d) }` is equivalent to `SetAim { direction: Some(d) }` followed in the
//!   same tick by `X { aim: None }`. The equivalence holds whether or not `X` then starts
//!   anything: a release of an undrawn bow or a sword attack while holding the bow still
//!   sets the held aim. The embedded aim persists as the held aim afterwards, because it is
//!   the player's current aim.
//! - A held target lock still takes precedence over aim intent, as it does today. The
//!   embedded aim sets the held aim intent and does not override the lock.
//! - Gameplay is independent of the lane, as before.
//!
//! Known limitation, by design: `None` cannot say "no aim". If the datagram that *clears*
//! the aim is lost, a following unaimed attack still uses the stale held aim until the
//! held-input refresh (#139) converges.

use arpg_core::{
    AIM_COMPONENT_LIMIT, ActionKind, ArpgCommand, ArpgGame, ArpgSnapshot, AuthoritativeGame,
    CommandLane, CounterOpportunity, PlayerCommand, PlayerId, PlayerSnapshot, ScenarioId,
    StrikeSource, Weapon, base_bundle,
};

const PLAYER: PlayerId = 1;
const SEED: u32 = 42;
/// Scenario targets stand along +x from the player: `TOWARD` points at them and `AWAY` points
/// away from them.
const TOWARD: [i16; 2] = [1000, 0];
const AWAY: [i16; 2] = [-1000, 0];
/// A third direction, distinct from both, for equivalence checks.
const DIAGONAL: [i16; 2] = [-250, 1000];
/// Covers the light swing (5 wind-up ticks) and the heavy thrust (10).
const MELEE_TICKS: u64 = 12;
/// Draw ticks before a release: above `bow.minDrawTicks` (8).
const DRAWN_TICKS: u64 = 12;
/// An arrow at minimum speed crosses the 500-unit archery range well inside this.
const ARROW_TICKS: u64 = 40;

fn primary(aim: Option<[i16; 2]>) -> ArpgCommand {
    ArpgCommand::PrimaryAttack { aim }
}

fn secondary(aim: Option<[i16; 2]>) -> ArpgCommand {
    ArpgCommand::SecondaryAttack { aim }
}

fn release(aim: Option<[i16; 2]>) -> ArpgCommand {
    ArpgCommand::ReleaseBow { aim }
}

fn set_aim(direction: [i16; 2]) -> ArpgCommand {
    ArpgCommand::SetAim {
        direction: Some(direction),
    }
}

const EQUIP_BOW: ArpgCommand = ArpgCommand::EquipWeapon {
    weapon: Weapon::Bow,
};

/// One player's command stream into one authority, with its own sequence counter.
struct Session {
    game: ArpgGame,
    sequence: u32,
}

impl Session {
    fn scenario(scenario: ScenarioId) -> Self {
        let mut game = ArpgGame::new_scenario(scenario, SEED).expect("scenario");
        game.add_player(PLAYER).expect("player");
        Self { game, sequence: 0 }
    }

    fn dungeon() -> Self {
        let mut game = ArpgGame::new_with_seed(SEED).expect("game");
        game.add_player(PLAYER).expect("player");
        Self { game, sequence: 0 }
    }

    fn try_send(&mut self, lane: CommandLane, command: ArpgCommand) -> Result<(), String> {
        let sequence = self.sequence + 1;
        self.game
            .apply_lane_command(
                lane,
                PlayerCommand::new(PLAYER, sequence, command).expect("non-zero sequence"),
            )
            .map_err(|error| error.message().to_owned())?;
        self.sequence = sequence;
        Ok(())
    }

    fn send(&mut self, command: ArpgCommand) {
        self.try_send(CommandLane::Datagram, command)
            .unwrap_or_else(|error| panic!("{command:?} is accepted: {error}"));
    }

    fn send_reliable(&mut self, command: ArpgCommand) {
        self.try_send(CommandLane::Reliable, command)
            .unwrap_or_else(|error| panic!("{command:?} is accepted: {error}"));
    }

    /// Advances `ticks` and returns every snapshot produced.
    fn run(&mut self, ticks: u64) -> Vec<ArpgSnapshot> {
        (0..ticks)
            .map(|_| {
                self.game.advance_tick().expect("tick");
                self.game.snapshot().expect("snapshot")
            })
            .collect()
    }

    fn snapshot(&self) -> ArpgSnapshot {
        self.game.snapshot().expect("snapshot")
    }

    fn player(&self) -> PlayerSnapshot {
        player(&self.snapshot()).clone()
    }

    /// Bow equipped and drawn for `ticks`, with `held` as the held aim.
    fn drawn(scenario: ScenarioId, held: [i16; 2], ticks: u64) -> Self {
        let mut session = Self::scenario(scenario);
        session.send(EQUIP_BOW);
        session.send(set_aim(held));
        session.send(ArpgCommand::DrawBow);
        session.run(ticks);
        assert_eq!(
            session.player().draw_ticks,
            Some(u8::try_from(ticks).unwrap()),
            "fixture"
        );
        session
    }
}

fn player(snapshot: &ArpgSnapshot) -> &PlayerSnapshot {
    snapshot
        .players
        .iter()
        .find(|player| player.id == PLAYER)
        .expect("the player is in the snapshot")
}

/// Player strikes resolved over `snapshots` (melee swings and arrow hits).
fn player_strikes(snapshots: &[ArpgSnapshot]) -> Vec<String> {
    snapshots
        .iter()
        .flat_map(|snapshot| &snapshot.strike_events)
        .filter(|event| event.source == StrikeSource::Player(PLAYER))
        .map(|event| event.definition.clone())
        .collect()
}

/// Velocity of the player's arrow in flight, when one is.
fn player_arrow_velocity(snapshots: &[ArpgSnapshot]) -> Option<[i32; 3]> {
    snapshots.iter().find_map(|snapshot| {
        snapshot
            .arrows
            .iter()
            .find(|arrow| arrow.source == StrikeSource::Player(PLAYER))
            .map(|arrow| arrow.velocity)
    })
}

/// The eight-way facing the core derives from an axis-aligned or clearly diagonal aim.
fn facing_of(direction: [i16; 2]) -> [i8; 2] {
    let axis = |own: i16, other: i16| -> i8 {
        if i32::from(own).abs() * 100_000 < i32::from(other).abs() * 41_421 {
            0
        } else {
            i8::try_from(own.signum()).unwrap()
        }
    };
    [
        axis(direction[0], direction[1]),
        axis(direction[1], direction[0]),
    ]
}

// --- Melee -----------------------------------------------------------------------------

/// The defect, for the sword: the player holds aim away from the dummy and turns to it, and
/// the turn reaches the authority only inside the attack. The swing goes where the attack
/// says.
#[test]
fn an_attack_strikes_along_its_embedded_aim_not_the_held_aim() {
    // Control: the held aim away from the dummy whiffs.
    let mut control = Session::scenario(ScenarioId::Dummy);
    control.send(set_aim(AWAY));
    control.send(primary(None));
    assert!(
        player_strikes(&control.run(MELEE_TICKS)).is_empty(),
        "fixture: a swing along the held aim away from the dummy whiffs"
    );

    let mut session = Session::scenario(ScenarioId::Dummy);
    session.send(set_aim(AWAY));
    session.send(primary(Some(TOWARD)));
    let action = session.player().action.expect("the attack started");
    assert_eq!(action.kind, ActionKind::PrimaryAttack);
    assert_eq!(
        action.aim,
        Some(TOWARD),
        "the attack commits its embedded aim in the same tick"
    );
    assert_eq!(action.facing, facing_of(TOWARD));
    assert_eq!(
        player_strikes(&session.run(MELEE_TICKS)),
        vec!["sword.lightSwing".to_owned()],
        "the swing along the embedded aim hits the dummy"
    );
}

/// The reverse: the held aim points at the dummy, the attack's own aim away. The attack's
/// aim wins, so nothing is hit.
#[test]
fn an_attack_aimed_away_from_a_target_misses_despite_the_held_aim() {
    let mut control = Session::scenario(ScenarioId::Dummy);
    control.send(set_aim(TOWARD));
    control.send(primary(None));
    assert_eq!(
        player_strikes(&control.run(MELEE_TICKS)).len(),
        1,
        "fixture: a swing along the held aim at the dummy hits"
    );

    let mut session = Session::scenario(ScenarioId::Dummy);
    session.send(set_aim(TOWARD));
    session.send(primary(Some(AWAY)));
    assert_eq!(session.player().action.expect("started").aim, Some(AWAY));
    assert!(player_strikes(&session.run(MELEE_TICKS)).is_empty());
}

#[test]
fn a_heavy_attack_commits_its_embedded_aim() {
    let mut session = Session::scenario(ScenarioId::Dummy);
    session.send(set_aim(AWAY));
    session.send(secondary(Some(DIAGONAL)));
    let action = session.player().action.expect("the heavy attack started");
    assert_eq!(action.kind, ActionKind::SecondaryAttack);
    assert_eq!(action.aim, Some(DIAGONAL));
    assert_eq!(action.facing, facing_of(DIAGONAL));
}

/// A primary attack inside a counter opportunity starts the counter; the counter strikes
/// along the attack's embedded aim.
#[test]
fn a_counter_started_by_an_aimed_attack_commits_that_aim() {
    let mut session = Session::dungeon();
    session.send(set_aim(AWAY));
    session.run(5);
    let mut save = session.game.save_state().expect("save");
    let window = base_bundle().counter_window_ticks;
    let blocked_at_tick = save.tick - 1;
    save.players[0].counter = Some(CounterOpportunity {
        blocked_monster_id: 1,
        blocked_at_tick,
        usable_from_tick: blocked_at_tick + 1,
        expires_at_tick: blocked_at_tick + 1 + window,
    });
    session.game = ArpgGame::from_save_state(save).expect("a valid open counter restores");

    session.send(primary(Some(DIAGONAL)));
    let action = session.player().action.expect("the counter started");
    assert_eq!(action.kind, ActionKind::Counter);
    assert_eq!(action.aim, Some(DIAGONAL));
    assert_eq!(action.facing, facing_of(DIAGONAL));
}

// --- Bow -------------------------------------------------------------------------------

/// The defect from the issue: the arrow goes along the release's aim, not the stale held
/// aim.
#[test]
fn a_release_shoots_along_its_embedded_aim_not_the_held_aim() {
    let mut control = Session::drawn(ScenarioId::Archery, AWAY, DRAWN_TICKS);
    control.send(release(None));
    let snapshots = control.run(ARROW_TICKS);
    let velocity = player_arrow_velocity(&snapshots).expect("fixture: the control shoots");
    assert!(velocity[0] < 0 && velocity[2] == 0, "fixture: {velocity:?}");
    assert!(player_strikes(&snapshots).is_empty(), "fixture");

    let mut session = Session::drawn(ScenarioId::Archery, AWAY, DRAWN_TICKS);
    session.send(release(Some(TOWARD)));
    let action = session.player().action.expect("the release started a shot");
    assert_eq!(action.kind, ActionKind::Shoot);
    assert_eq!(
        action.aim,
        Some(TOWARD),
        "the shot commits the release's aim"
    );
    assert_eq!(action.facing, facing_of(TOWARD));
    let snapshots = session.run(ARROW_TICKS);
    let velocity = player_arrow_velocity(&snapshots).expect("the release shoots");
    assert!(
        velocity[0] > 0 && velocity[2] == 0,
        "the arrow flies along the embedded aim: {velocity:?}"
    );
    assert_eq!(
        player_strikes(&snapshots),
        vec!["bow.arrow".to_owned()],
        "the arrow hits the archery target"
    );
}

/// The dedicated sequence from the issue, lane by lane: aim A arrives as a datagram, the
/// datagram for aim B is lost, and the release carrying B comes on the reliable lane.
#[test]
fn a_reliable_release_after_a_lost_aim_datagram_shoots_along_the_new_aim() {
    let mut session = Session::drawn(ScenarioId::Archery, AWAY, DRAWN_TICKS);
    // The datagram with sequence `session.sequence + 1` (setAim TOWARD) is lost.
    session.sequence += 1;
    session.send_reliable(release(Some(TOWARD)));
    let snapshots = session.run(ARROW_TICKS);
    let velocity = player_arrow_velocity(&snapshots).expect("the release shoots");
    assert!(velocity[0] > 0 && velocity[2] == 0, "{velocity:?}");
    assert_eq!(player_strikes(&snapshots), vec!["bow.arrow".to_owned()]);
    assert_eq!(session.player().aim, Some(TOWARD));
}

// --- Equivalence, persistence, determinism --------------------------------------------

/// `X { aim: Some(d) }` is `SetAim(d)` followed by `X { aim: None }` in the same tick,
/// whatever X then does.
#[test]
fn an_embedded_aim_is_exactly_a_set_aim_before_the_command() {
    type Arrange = fn() -> Session;
    let cases: [(&str, Arrange, ArpgCommand, ArpgCommand); 7] = [
        (
            "idle light attack",
            || {
                let mut session = Session::scenario(ScenarioId::Dummy);
                session.send(set_aim(AWAY));
                session
            },
            primary(Some(DIAGONAL)),
            primary(None),
        ),
        (
            "idle heavy attack",
            || {
                let mut session = Session::scenario(ScenarioId::Dummy);
                session.send(set_aim(AWAY));
                session
            },
            secondary(Some(DIAGONAL)),
            secondary(None),
        ),
        (
            "light attack buffered into a running light attack",
            || {
                let mut session = Session::scenario(ScenarioId::Dummy);
                session.send(set_aim(AWAY));
                session.send(primary(None));
                session.run(3);
                session
            },
            primary(Some(DIAGONAL)),
            primary(None),
        ),
        (
            "heavy attack buffered into a running light attack",
            || {
                let mut session = Session::scenario(ScenarioId::Dummy);
                session.send(set_aim(AWAY));
                session.send(primary(None));
                session.run(3);
                session
            },
            secondary(Some(DIAGONAL)),
            secondary(None),
        ),
        (
            "release of a drawn bow",
            || Session::drawn(ScenarioId::Archery, AWAY, DRAWN_TICKS),
            release(Some(DIAGONAL)),
            release(None),
        ),
        (
            "release of a bow drawn too briefly to shoot",
            || Session::drawn(ScenarioId::Archery, AWAY, 2),
            release(Some(DIAGONAL)),
            release(None),
        ),
        (
            "sword attack while holding the bow",
            || {
                let mut session = Session::scenario(ScenarioId::Archery);
                session.send(EQUIP_BOW);
                session.send(set_aim(AWAY));
                session
            },
            primary(Some(DIAGONAL)),
            primary(None),
        ),
    ];
    for (name, arrange, aimed, unaimed) in cases {
        let mut embedded = arrange();
        embedded.send(aimed);
        let mut separate = arrange();
        separate.send(set_aim(DIAGONAL));
        separate.send(unaimed);

        assert_eq!(embedded.player().aim, Some(DIAGONAL), "{name}");
        assert_eq!(
            embedded.snapshot(),
            separate.snapshot(),
            "{name}: same state right after the command"
        );
        assert_eq!(
            embedded.run(ARROW_TICKS),
            separate.run(ARROW_TICKS),
            "{name}: same outcome afterwards"
        );
    }
}

/// The embedded aim is the player's current aim: it stays held, and a later unaimed attack
/// follows it.
#[test]
fn an_embedded_aim_persists_as_the_held_aim() {
    let mut session = Session::scenario(ScenarioId::Dummy);
    session.send(set_aim(AWAY));
    session.send(primary(Some(TOWARD)));
    assert_eq!(session.player().aim, Some(TOWARD));
    session.run(30);
    let player = session.player();
    assert_eq!(player.action, None, "fixture: the attack ended");
    assert_eq!(player.aim, Some(TOWARD), "the embedded aim is still held");
    assert_eq!(player.facing, facing_of(TOWARD));

    session.send(primary(None));
    assert_eq!(
        session
            .player()
            .action
            .expect("the next attack started")
            .aim,
        Some(TOWARD)
    );
}

#[test]
fn aimed_one_shots_are_deterministic() {
    let play = || {
        let mut session = Session::drawn(ScenarioId::Archery, AWAY, DRAWN_TICKS);
        session.send_reliable(release(Some(DIAGONAL)));
        let mut snapshots = session.run(ARROW_TICKS);
        session.send(ArpgCommand::EquipWeapon {
            weapon: Weapon::SwordAndShield,
        });
        session.send_reliable(primary(Some(TOWARD)));
        snapshots.extend(session.run(MELEE_TICKS));
        session.send_reliable(secondary(Some(AWAY)));
        snapshots.extend(session.run(30));
        (snapshots, session.game.save_state().expect("save"))
    };
    assert_eq!(play(), play());
}

// --- Absent aim -----------------------------------------------------------------------

/// `aim: None` is today's behaviour: the held aim is used and not changed.
#[test]
fn an_unaimed_one_shot_uses_the_held_aim() {
    let mut session = Session::scenario(ScenarioId::Dummy);
    session.send(set_aim(TOWARD));
    session.send(primary(None));
    assert_eq!(session.player().action.expect("started").aim, Some(TOWARD));
    assert_eq!(session.player().aim, Some(TOWARD));
    assert_eq!(player_strikes(&session.run(MELEE_TICKS)).len(), 1);

    let mut bow = Session::drawn(ScenarioId::Archery, TOWARD, DRAWN_TICKS);
    bow.send(release(None));
    assert_eq!(bow.player().action.expect("shot").aim, Some(TOWARD));
    let velocity = player_arrow_velocity(&bow.run(ARROW_TICKS)).expect("shoots");
    assert!(velocity[0] > 0 && velocity[2] == 0, "{velocity:?}");
}

/// Without any aim, the attack keeps the committed facing, as before.
#[test]
fn an_unaimed_one_shot_without_held_aim_keeps_committed_facing() {
    let mut session = Session::scenario(ScenarioId::Dummy);
    let facing = session.player().facing;
    session.send(primary(None));
    let action = session.player().action.expect("started");
    assert_eq!(action.aim, None);
    assert_eq!(action.facing, facing);
    assert_eq!(session.player().aim, None);
}

// --- Rejection -------------------------------------------------------------------------

fn invalid_aims() -> [[i16; 2]; 6] {
    let over = AIM_COMPONENT_LIMIT + 1;
    [
        [0, 0],
        [over, 0],
        [0, -over],
        [-over, over],
        [i16::MIN, 1],
        [1, i16::MAX],
    ]
}

/// A named arrangement: how to set it up and which aimed command it sends.
type Arrangement = (
    &'static str,
    fn() -> Session,
    fn(Option<[i16; 2]>) -> ArpgCommand,
);

/// An invalid embedded aim rejects the whole command exactly as `SetAim` rejects that
/// direction: the same error, no action, no aim change, and the sequence stays usable.
#[test]
fn an_invalid_embedded_aim_is_rejected_like_set_aim_and_has_no_effect() {
    let arrangements: [Arrangement; 3] = [
        (
            "primary attack",
            || {
                let mut session = Session::scenario(ScenarioId::Dummy);
                session.send(set_aim(AWAY));
                session
            },
            primary,
        ),
        (
            "secondary attack",
            || {
                let mut session = Session::scenario(ScenarioId::Dummy);
                session.send(set_aim(AWAY));
                session
            },
            secondary,
        ),
        (
            "bow release",
            || Session::drawn(ScenarioId::Archery, AWAY, DRAWN_TICKS),
            release,
        ),
    ];
    for (name, arrange, make) in arrangements {
        for direction in invalid_aims() {
            for lane in [CommandLane::Datagram, CommandLane::Reliable] {
                let mut reference = arrange();
                let set_aim_error = reference
                    .try_send(lane, set_aim(direction))
                    .expect_err("fixture: SetAim rejects the direction");

                let mut session = arrange();
                let snapshot = session.snapshot();
                let save = session.game.save_state().expect("save");
                assert_eq!(
                    session.try_send(lane, make(Some(direction))),
                    Err(set_aim_error),
                    "{name} {direction:?} {lane:?}: rejected with SetAim's error"
                );
                assert_eq!(
                    session.snapshot(),
                    snapshot,
                    "{name} {direction:?} {lane:?}: no effect"
                );
                assert_eq!(
                    session.game.save_state().expect("save"),
                    save,
                    "{name} {direction:?} {lane:?}: no watermark or held state change"
                );
                // The same sequence is still free: a valid command with it is applied.
                session.try_send(lane, make(None)).unwrap_or_else(|error| {
                    panic!("{name} {direction:?} {lane:?}: sequence still free: {error}")
                });
                assert_eq!(session.player().aim, Some(AWAY), "{name}");
            }
        }
    }
}
