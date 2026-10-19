//! Flat-ground jumping and support (#59, first slice), through the public authority.
//!
//! The built-in content tunes a jump to a 90-unit apex and 36 ticks of airtime, with a
//! 6-tick coyote window and a 6-tick jump buffer. Rapier's character controller lands the
//! body on the floor; snapshots publish its height and support, saves continue a jump
//! exactly.

use arpg_core::{
    ActionPhase, ArpgCommand, ArpgGame, ArpgSaveState, AuthoritativeGame, GuardPhase,
    PlayerCommand, PlayerSnapshot, base_bundle,
};

const APEX: i32 = 90;
const AIRTIME: u64 = 36;
const BUFFER: u64 = 6;

struct Session {
    game: ArpgGame,
    sequence: u32,
}

impl Session {
    fn new() -> Self {
        let mut game = ArpgGame::new_with_seed(0x5EED).unwrap();
        game.add_player(1).unwrap();
        Self { game, sequence: 0 }
    }

    fn send(&mut self, command: ArpgCommand) {
        self.sequence += 1;
        self.game
            .apply_command(PlayerCommand::new(1, self.sequence, command).unwrap())
            .unwrap();
    }

    fn player(&self) -> PlayerSnapshot {
        self.game.snapshot().unwrap().players[0].clone()
    }

    /// Advances one tick and returns the player afterwards.
    fn tick(&mut self) -> PlayerSnapshot {
        self.game.advance_tick().unwrap();
        self.player()
    }

    /// Advances `ticks` ticks and returns the player after each.
    fn run(&mut self, ticks: u64) -> Vec<PlayerSnapshot> {
        (0..ticks).map(|_| self.tick()).collect()
    }
}

/// Indices of the ticks (in `trace`) at which the player took off: airborne after a tick
/// that ended grounded, or after the trace's start for the first entry.
fn takeoffs(start_grounded: bool, trace: &[PlayerSnapshot]) -> Vec<usize> {
    let mut grounded = start_grounded;
    let mut result = Vec::new();
    for (index, player) in trace.iter().enumerate() {
        if grounded && !player.motion.grounded && player.motion.vertical_velocity > 0 {
            result.push(index);
        }
        grounded = player.motion.grounded;
    }
    result
}

#[test]
fn the_built_in_jump_tuning_is_content() {
    let jump = base_bundle().jump;
    assert_eq!(jump.apex_height, APEX as u16);
    assert_eq!(u64::from(jump.airtime_ticks), AIRTIME);
    assert_eq!(jump.coyote_ticks, 6);
    assert_eq!(u64::from(jump.buffer_ticks), BUFFER);
    // Normal air control: the ground's rates.
    assert_eq!(
        (
            jump.air_acceleration_per_tick,
            jump.air_braking_per_tick,
            jump.air_reversal_per_tick
        ),
        (3, 5, 5)
    );

    let mut invalid = base_bundle().clone();
    invalid.jump.apex_height = 101;
    invalid.jump.buffer_ticks = 0;
    invalid.jump.air_acceleration_per_tick = 8;
    let errors = invalid
        .check()
        .unwrap_err()
        .into_iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    for path in [
        "jump.apexHeight",
        "jump.bufferTicks",
        "jump.airAccelerationPerTick",
    ] {
        assert!(
            errors.iter().any(|error| error.starts_with(path)),
            "{path}: {errors:?}"
        );
    }
}

#[test]
fn a_jump_rises_to_its_apex_and_lands_on_the_floor_after_its_airtime() {
    let mut session = Session::new();
    let rest = session.player();
    assert!(rest.motion.grounded);
    assert_eq!(rest.motion.feet_y, 0);
    assert_eq!(rest.motion.ground_y, Some(0));

    session.send(ArpgCommand::Jump);
    let trace = session.run(AIRTIME + 20);
    assert_eq!(takeoffs(true, &trace), [0], "one takeoff on the first tick");
    let apex = trace
        .iter()
        .map(|player| player.motion.feet_y)
        .max()
        .unwrap();
    assert!((APEX - 1..=APEX + 1).contains(&apex), "apex {apex}");
    let landed = trace
        .iter()
        .position(|player| player.motion.grounded)
        .unwrap() as u64;
    assert!(
        (AIRTIME - 2..=AIRTIME).contains(&landed),
        "landed after {landed} ticks"
    );
    for player in &trace {
        // The ground under the body stays the floor, airborne or not.
        assert_eq!(player.motion.ground_y, Some(0));
        assert_eq!(player.motion.feet_y, player.position[1] - 50);
    }
    for player in &trace[landed as usize..] {
        assert!(player.motion.grounded);
        assert_eq!(player.motion.vertical_velocity, 0);
        assert_eq!(player.position[1], 50);
    }
    // The landing settles the body onto the plane to `f32` resolution.
    let save = session.game.save_state().unwrap();
    assert!((save.players[0].position[1] - 50.0).abs() < 1.0e-3);
    assert_eq!(save.players[0].velocity[1], 0.0);
}

#[test]
fn held_and_repeated_jump_presses_take_off_once() {
    let mut session = Session::new();
    // Three presses before the first tick, then one every tick while airborne, as a held
    // key's repeats would arrive: a press only renews the buffer window, which lapses long
    // before the landing.
    for _ in 0..3 {
        session.send(ArpgCommand::Jump);
    }
    let mut trace = Vec::new();
    for _ in 0..12 {
        trace.push(session.tick());
        session.send(ArpgCommand::Jump);
    }
    trace.extend(session.run(AIRTIME + 30));
    assert_eq!(takeoffs(true, &trace), [0]);
}

#[test]
fn a_stale_jump_command_is_refused_and_changes_nothing() {
    let mut session = Session::new();
    session.send(ArpgCommand::SetMovement { x: 0, z: 0 });
    let stale = session
        .game
        .apply_command(PlayerCommand::new(1, session.sequence, ArpgCommand::Jump).unwrap())
        .unwrap_err();
    assert_eq!(stale.message(), "command sequence is stale");
    let trace = session.run(10);
    assert!(trace.iter().all(|player| player.motion.grounded));
}

#[test]
fn a_press_shortly_before_landing_is_buffered_and_an_earlier_one_lapses() {
    // The landing tick of an undisturbed jump: the body ends that tick on the ground, so
    // the next tick is the first a jump can take off in.
    let mut probe = Session::new();
    probe.send(ArpgCommand::Jump);
    let landing = probe
        .run(AIRTIME + 5)
        .iter()
        .position(|player| player.motion.grounded)
        .unwrap() as u64;
    let takeoff = landing + 1;

    // A press waits through `BUFFER` ticks, its own included: one sent `BUFFER - 1` ticks
    // before the first possible takeoff still jumps, one sent a tick earlier lapses.
    for (lead, jumps_again) in [(BUFFER - 1, true), (BUFFER, false), (BUFFER + 3, false)] {
        let mut session = Session::new();
        session.send(ArpgCommand::Jump);
        let mut trace = session.run(takeoff - lead);
        session.send(ArpgCommand::Jump);
        trace.extend(session.run(lead + 10));
        let expected: &[usize] = if jumps_again {
            &[0, takeoff as usize]
        } else {
            &[0]
        };
        assert_eq!(
            takeoffs(true, &trace),
            expected,
            "pressed {lead} ticks before the first possible takeoff"
        );
    }
}

#[test]
fn a_jump_never_cuts_an_action_short_and_waits_for_it_within_the_buffer() {
    // Primary attack: 5 wind-up, 1 active and 8 recovery ticks.
    let attack = |press_after: u64| {
        let mut session = Session::new();
        session.send(ArpgCommand::PrimaryAttack { aim: None });
        let mut trace = session.run(press_after);
        session.send(ArpgCommand::Jump);
        trace.extend(session.run(20));
        trace
    };

    // Pressed early in the attack: the buffer lapses while the attack runs.
    let early = attack(2);
    assert!(early.iter().all(|player| player.motion.grounded));
    // Pressed during the recovery: the attack completes every phase, then the jump.
    let late = attack(10);
    let phases = late
        .iter()
        .filter_map(|player| player.action.as_ref().map(|action| action.phase))
        .collect::<Vec<_>>();
    assert_eq!(phases.len(), 13, "{phases:?}");
    assert_eq!(phases.last(), Some(&ActionPhase::Recovery));
    // The tick the attack ends in has already moved; the jump takes off in the next one.
    let finished = late
        .iter()
        .position(|player| player.action.is_none())
        .unwrap();
    assert_eq!(takeoffs(true, &late), [finished + 1]);
}

#[test]
fn attacks_start_in_the_air_and_the_fall_continues() {
    let mut session = Session::new();
    session.send(ArpgCommand::Jump);
    session.run(4);
    session.send(ArpgCommand::PrimaryAttack { aim: None });
    let trace = session.run(24);
    assert!(trace[0].action.is_some(), "an airborne attack starts");
    assert!(!trace[0].motion.grounded);
    assert!(trace[..13].iter().all(|player| player.action.is_some()));
    // Gravity keeps acting during and after the action: past the apex the body comes down.
    assert!(trace[23].position[1] < trace[13].position[1]);
    assert!(trace.iter().all(|player| !player.motion.grounded));
}

#[test]
fn the_guard_needs_the_ground_and_rises_again_after_landing() {
    let mut session = Session::new();
    session.send(ArpgCommand::SetGuard { raised: true });
    let raised = session.run(6);
    assert_eq!(
        raised[5].guard.map(|guard| guard.phase),
        Some(GuardPhase::Raised)
    );

    session.send(ArpgCommand::Jump);
    let trace = session.run(AIRTIME + 10);
    let landed = trace
        .iter()
        .position(|player| player.motion.grounded)
        .unwrap();
    // Taking off lowers the guard at once; it stays down in the air although still held.
    assert!(trace[..landed].iter().all(|player| player.guard.is_none()));
    // Back on the ground the held guard rises from the start.
    assert_eq!(
        trace[landed + 1].guard.map(|guard| guard.phase),
        Some(GuardPhase::Raising)
    );
    assert_eq!(
        trace.last().unwrap().guard.map(|guard| guard.phase),
        Some(GuardPhase::Raised)
    );
}

#[test]
fn air_control_steers_like_the_ground() {
    let mut ground = Session::new();
    ground.send(ArpgCommand::SetMovement { x: 1, z: 0 });
    let walked = ground.run(10);

    let mut air = Session::new();
    air.send(ArpgCommand::Jump);
    air.send(ArpgCommand::SetMovement { x: 1, z: 0 });
    let flown = air.run(10);
    for (walk, flight) in walked.iter().zip(&flown) {
        assert!(!flight.motion.grounded);
        assert_eq!(walk.position[0], flight.position[0]);
        assert_eq!(walk.position[2], flight.position[2]);
    }
}

#[test]
fn a_save_mid_jump_continues_exactly() {
    let mut session = Session::new();
    session.send(ArpgCommand::SetMovement { x: 1, z: 1 });
    session.send(ArpgCommand::Jump);
    session.run(9);
    // A buffered press waits in the save as well.
    session.send(ArpgCommand::Jump);
    session.run(AIRTIME - 9 - 4);

    let save = session.game.save_state().unwrap();
    assert!(!save.players[0].grounded);
    assert!(save.players[0].velocity[1] < 0.0);
    assert_eq!(save.schema_version, 16);
    let json = serde_json::to_string(&save).unwrap();
    let mut restored = Session {
        game: ArpgGame::from_save_state(ArpgSaveState::from_json(&json).unwrap()).unwrap(),
        sequence: session.sequence,
    };
    assert_eq!(restored.game.save_state().unwrap(), save);
    for tick in 0..60 {
        let expected = session.tick();
        assert_eq!(restored.tick(), expected, "tick {tick}");
        assert_eq!(
            restored.game.save_state().unwrap(),
            session.game.save_state().unwrap(),
            "tick {tick}"
        );
    }
}

#[test]
fn schema_14_saves_are_refused_by_their_version() {
    let session = Session::new();
    let mut json = serde_json::to_value(session.game.save_state().unwrap()).unwrap();
    json["schemaVersion"] = 14.into();
    for field in [
        "grounded",
        "coyoteTicksRemaining",
        "jumpBufferTicksRemaining",
    ] {
        json["players"][0].as_object_mut().unwrap().remove(field);
    }
    assert_eq!(
        ArpgSaveState::from_json(&json.to_string())
            .unwrap_err()
            .message(),
        "unsupported ARPG save schema version 14; expected 16"
    );
}

#[test]
fn impossible_jump_state_in_a_save_is_refused() {
    let mut session = Session::new();
    session.send(ArpgCommand::SetGuard { raised: true });
    session.run(6);
    session.send(ArpgCommand::Jump);
    session.run(3);
    let airborne = session.game.save_state().unwrap();
    assert!(ArpgGame::from_save_state(airborne.clone()).is_ok());

    let refused = |edit: &dyn Fn(&mut ArpgSaveState)| {
        let mut save = airborne.clone();
        edit(&mut save);
        ArpgGame::from_save_state(save).is_err()
    };
    // Grounded while moving vertically, or far above the floor.
    assert!(refused(&|save| save.players[0].grounded = true));
    assert!(refused(&|save| {
        save.players[0].grounded = true;
        save.players[0].velocity[1] = 0.0;
    }));
    // Higher than any jump reaches, or below the floor.
    assert!(refused(&|save| save.players[0].position[1] = 50.0 + 200.0));
    assert!(refused(&|save| save.players[0].position[1] = 30.0));
    // Faster than any jump or fall.
    assert!(refused(&|save| save.players[0].velocity[1] = 40.0));
    // Windows longer than the content's.
    assert!(refused(
        &|save| save.players[0].jump_buffer_ticks_remaining = 7
    ));
    assert!(refused(&|save| save.players[0].coyote_ticks_remaining = 7));
    // A guard raised in the air.
    assert!(refused(&|save| {
        save.players[0].guard.stance = Some(arpg_core::GuardStance {
            phase: GuardPhase::Raised,
            ticks_remaining: 0,
        });
    }));
}
