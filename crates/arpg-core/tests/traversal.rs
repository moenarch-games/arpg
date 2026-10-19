//! Support on box tops, steps and ledges (#208), through the public authority, in the
//! `traversal` scenario: a platform 60 units high, 160 units east of the player's spawn, that
//! is its own walkable region (2) above the floor (1).

use arpg_core::{
    ArpgCommand, ArpgGame, ArpgSaveState, AuthoritativeGame, PlayerCommand, PlayerSnapshot,
    RegionMembership, ScenarioId, StaticColliderKind,
};

const PLATFORM_TOP: i32 = 60;
const FLOOR: RegionMembership = RegionMembership::Region { id: 1 };
const PLATFORM: RegionMembership = RegionMembership::Region { id: 2 };

struct Session {
    game: ArpgGame,
    sequence: u32,
}

impl Session {
    fn new() -> Self {
        let mut game = ArpgGame::new_scenario(ScenarioId::Traversal, 0x5EED).unwrap();
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

    fn tick(&mut self) -> PlayerSnapshot {
        self.game.advance_tick().unwrap();
        self.player()
    }

    /// Holds east and jumps after `run_up` ticks; returns once grounded again.
    fn jump_east(&mut self, run_up: u32) -> PlayerSnapshot {
        self.send(ArpgCommand::SetMovement { x: 1, z: 0 });
        for _ in 0..run_up {
            self.tick();
        }
        self.send(ArpgCommand::Jump);
        let mut airborne = false;
        for _ in 0..80 {
            let player = self.tick();
            airborne |= !player.motion.grounded;
            if airborne && player.motion.grounded {
                return player;
            }
        }
        panic!("the player never landed");
    }
}

#[test]
fn the_traversal_scenario_lays_a_platform_in_its_own_region() {
    let session = Session::new();
    let snapshot = session.game.snapshot().unwrap();
    let platform = snapshot
        .static_colliders
        .iter()
        .find(|collider| collider.kind == StaticColliderKind::Platform)
        .expect("the scenario has a platform");
    assert_eq!(
        platform.position[1] + platform.half_extents[1],
        PLATFORM_TOP
    );
    assert_eq!(snapshot.players[0].region, FLOOR);
}

#[test]
fn a_jump_lands_on_the_platform_top_and_settles_in_its_region() {
    let mut session = Session::new();
    let landed = session.jump_east(7);
    // Support is predicted within 5 units above the top; the body settles onto it next tick.
    assert!(
        (0..=5).contains(&(landed.motion.feet_y - PLATFORM_TOP)),
        "{landed:?}"
    );
    assert_eq!(landed.motion.ground_y, Some(PLATFORM_TOP));
    assert_eq!(landed.region, PLATFORM);
    // It stays supported on the top while it stands there.
    session.send(ArpgCommand::SetMovement { x: 0, z: 0 });
    session.tick();
    for _ in 0..20 {
        let player = session.tick();
        assert!(player.motion.grounded);
        assert_eq!(player.motion.feet_y, PLATFORM_TOP);
    }
}

#[test]
fn walking_off_the_ledge_drops_back_to_the_floor_region() {
    let mut session = Session::new();
    session.jump_east(7);
    session.send(ArpgCommand::SetMovement { x: 1, z: 0 });
    let mut left_the_ledge = false;
    for _ in 0..120 {
        let player = session.tick();
        if !player.motion.grounded {
            left_the_ledge = true;
            // Airborne motion keeps the membership.
            assert_eq!(player.region, PLATFORM);
        }
        if left_the_ledge && player.motion.grounded {
            assert!((0..=5).contains(&player.motion.feet_y), "{player:?}");
            assert_eq!(player.region, FLOOR);
            assert_eq!(session.tick().motion.feet_y, 0);
            return;
        }
    }
    panic!("the player never dropped off the ledge");
}

#[test]
fn walking_into_the_platform_without_a_jump_is_blocked_by_its_side() {
    let mut session = Session::new();
    let start = session.player().position[0];
    session.send(ArpgCommand::SetMovement { x: 1, z: 0 });
    let mut player = session.player();
    for _ in 0..60 {
        player = session.tick();
    }
    // The near face is 160 units east; the body's radius is 30.
    assert!((player.position[0] - start - 130).abs() <= 1, "{player:?}");
    assert_eq!(player.motion.feet_y, 0);
    assert_eq!(player.region, FLOOR);
}

#[test]
fn saves_on_the_platform_and_during_the_drop_continue_exactly() {
    for save_after in [0_u32, 6] {
        let mut session = Session::new();
        session.jump_east(7);
        session.send(ArpgCommand::SetMovement { x: 1, z: 0 });
        // On the top, then (with `save_after` 6 more ticks of walking) maybe in the air.
        for _ in 0..(save_after * 4) {
            session.tick();
        }
        let save = session.game.save_state().unwrap();
        assert_eq!(save.players[0].grounded, save_after == 0, "{save_after}");
        let json = serde_json::to_string(&save).unwrap();
        let mut restored = Session {
            game: ArpgGame::from_save_state(ArpgSaveState::from_json(&json).unwrap()).unwrap(),
            sequence: session.sequence,
        };
        assert_eq!(restored.game.save_state().unwrap(), save);
        for tick in 0..60 {
            assert_eq!(restored.tick(), session.tick(), "tick {tick}");
            assert_eq!(
                restored.game.save_state().unwrap(),
                session.game.save_state().unwrap(),
                "tick {tick}"
            );
        }
    }
}

#[test]
fn a_saved_player_floating_above_the_platform_or_inside_it_is_rejected() {
    let mut session = Session::new();
    session.jump_east(7);
    let save = session.game.save_state().unwrap();
    for (dy, grounded) in [(20.0, true), (-40.0, false)] {
        let mut tampered = save.clone();
        tampered.players[0].position[1] += dy;
        tampered.players[0].grounded = grounded;
        assert!(ArpgGame::from_save_state(tampered).is_err(), "dy {dy}");
    }
}
