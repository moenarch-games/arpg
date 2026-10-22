//! Support on box tops, steps and ledges (#208) and low ceilings (#209), through the public
//! authority, in the `traversal` scenario: a platform 60 units high, 160 units east of the
//! player's spawn, that is its own walkable region (2) above the floor (1), and a ceiling
//! 140 units up, 250 units north of the spawn.

use arpg_core::{
    ArpgCommand, ArpgGame, ArpgSaveState, AuthoritativeGame, PlayerCommand, PlayerSnapshot,
    RegionMembership, ScenarioId, StaticColliderKind,
};

const PLATFORM_TOP: i32 = 60;
const FLOOR: RegionMembership = RegionMembership::Region { id: 1 };
const PLATFORM: RegionMembership = RegionMembership::Region { id: 2 };
const BRIDGE: RegionMembership = RegionMembership::Region { id: 3 };
/// The drop from the platform, and the jump between platform and bridge.
const DROP: RegionMembership = RegionMembership::Transit { connection: 2 };
const UP_TO_BRIDGE: RegionMembership = RegionMembership::Transit { connection: 3 };

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
    // South, off the platform's side (the bridge adjoins its far edge).
    session.send(ArpgCommand::SetMovement { x: 0, z: 1 });
    let mut left_the_ledge = false;
    let mut transit = false;
    for _ in 0..120 {
        let player = session.tick();
        if !player.motion.grounded {
            left_the_ledge = true;
            // Over the platform the membership stays; past its edge the drop is in transit.
            assert!(
                player.region == PLATFORM || player.region == DROP,
                "{player:?}"
            );
            transit |= player.region == DROP;
        }
        if left_the_ledge && player.motion.grounded {
            assert!((0..=5).contains(&player.motion.feet_y), "{player:?}");
            assert!(transit, "the drop never entered transit");
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
        session.send(ArpgCommand::SetMovement { x: 0, z: 1 });
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

const CEILING_BOTTOM: i32 = 140;

/// Walks north under the ceiling and stops there.
fn under_the_ceiling() -> Session {
    let mut session = Session::new();
    session.send(ArpgCommand::SetMovement { x: 0, z: -1 });
    for _ in 0..36 {
        session.tick();
    }
    session.send(ArpgCommand::SetMovement { x: 0, z: 0 });
    for _ in 0..10 {
        session.tick();
    }
    session
}

#[test]
fn a_standing_body_walks_under_the_low_ceiling() {
    let session = under_the_ceiling();
    let snapshot = session.game.snapshot().unwrap();
    let ceiling = snapshot
        .static_colliders
        .iter()
        .find(|collider| collider.kind == StaticColliderKind::Ceiling)
        .expect("the scenario has a ceiling");
    assert_eq!(
        ceiling.position[1] - ceiling.half_extents[1],
        CEILING_BOTTOM
    );
    let player = &snapshot.players[0];
    let (dx, dz) = (
        player.position[0] - ceiling.position[0],
        player.position[2] - ceiling.position[2],
    );
    assert!(
        dx.abs() < ceiling.half_extents[0] && dz.abs() < ceiling.half_extents[2],
        "{player:?}"
    );
    assert!(player.motion.grounded);
    assert_eq!(player.motion.feet_y, 0);
}

#[test]
fn a_jump_under_the_ceiling_stops_at_it_and_falls_back() {
    let mut session = under_the_ceiling();
    session.send(ArpgCommand::Jump);
    let mut highest = 0;
    let mut bumped_at = None;
    for tick in 0..60 {
        let player = session.tick();
        // The head (100 above the feet) never enters the ceiling; it stops a contact offset
        // short of it.
        assert!(
            player.motion.feet_y + 100 <= CEILING_BOTTOM,
            "tick {tick}: {player:?}"
        );
        highest = highest.max(player.motion.feet_y);
        if bumped_at.is_none() && player.motion.vertical_velocity <= 0 && !player.motion.grounded {
            bumped_at = Some(tick);
        }
        if player.motion.grounded && tick > 0 {
            assert_eq!(
                highest,
                CEILING_BOTTOM - 100,
                "the head reached the ceiling"
            );
            // A free jump peaks after 18 ticks; the ceiling ends the rise early.
            assert!(bumped_at.is_some_and(|tick| tick < 12), "{bumped_at:?}");
            return;
        }
    }
    panic!("the player never landed");
}

#[test]
fn a_save_right_after_a_ceiling_bump_continues_exactly() {
    let mut session = under_the_ceiling();
    session.send(ArpgCommand::Jump);
    for _ in 0..8 {
        session.tick();
    }
    let save = session.game.save_state().unwrap();
    assert!(!save.players[0].grounded);
    let mut restored = Session {
        game: ArpgGame::from_save_state(save.clone()).unwrap(),
        sequence: session.sequence,
    };
    for tick in 0..40 {
        assert_eq!(restored.tick(), session.tick(), "tick {tick}");
    }
}

const BRIDGE_TOP: i32 = 140;

impl Session {
    /// On the platform, then east into the bridge's face and a jump up onto it; returns once
    /// grounded on the bridge, with every membership seen in the air.
    fn onto_the_bridge(&mut self) -> Vec<RegionMembership> {
        self.jump_east(7);
        for _ in 0..12 {
            self.tick();
        }
        self.send(ArpgCommand::Jump);
        let mut seen = Vec::new();
        for _ in 0..60 {
            let player = self.tick();
            if !player.motion.grounded {
                seen.push(player.region);
            } else if !seen.is_empty() {
                assert_eq!(player.region, BRIDGE, "{player:?}");
                self.send(ArpgCommand::SetMovement { x: 0, z: 0 });
                self.tick();
                return seen;
            }
        }
        panic!("the player never landed on the bridge");
    }
}

#[test]
fn a_jump_from_the_platform_reaches_the_bridge_in_transit() {
    let mut session = Session::new();
    let seen = session.onto_the_bridge();
    assert!(seen.contains(&UP_TO_BRIDGE), "{seen:?}");
    let player = session.player();
    assert_eq!(player.motion.feet_y, BRIDGE_TOP);
    assert_eq!(player.region, BRIDGE);
}

#[test]
fn the_floor_path_runs_under_the_bridge_in_the_floor_region() {
    let mut session = Session::new();
    let bridge = session
        .game
        .snapshot()
        .unwrap()
        .static_colliders
        .into_iter()
        .filter(|collider| collider.kind == StaticColliderKind::Platform)
        .max_by_key(|collider| collider.position[1])
        .unwrap();
    // South past the platform, east to the bridge's middle, then north beneath it.
    for (x, z, ticks) in [(0, 1, 24), (1, 0, 74), (0, -1, 24)] {
        session.send(ArpgCommand::SetMovement { x, z });
        for _ in 0..ticks {
            session.tick();
        }
    }
    session.send(ArpgCommand::SetMovement { x: 0, z: 0 });
    let player = session.tick();
    let (dx, dz) = (
        player.position[0] - bridge.position[0],
        player.position[2] - bridge.position[2],
    );
    // The same X/Z as the bridge above, on the floor below it.
    assert!(
        dx.abs() < bridge.half_extents[0] && dz.abs() < bridge.half_extents[2],
        "{player:?}"
    );
    assert!(player.motion.grounded);
    assert_eq!(player.motion.feet_y, 0);
    assert_eq!(player.region, FLOOR);
}

#[test]
fn walking_off_the_bridge_side_drops_in_transit_and_a_save_continues_it() {
    let mut session = Session::new();
    session.onto_the_bridge();
    session.send(ArpgCommand::SetMovement { x: 0, z: 1 });
    let off = RegionMembership::Transit { connection: 4 };
    for _ in 0..40 {
        if session.tick().region == off {
            break;
        }
    }
    assert_eq!(session.player().region, off);
    let save = session.game.save_state().unwrap();
    assert_eq!(save.players[0].region, off);
    let mut restored = Session {
        game: ArpgGame::from_save_state(save).unwrap(),
        sequence: session.sequence,
    };
    for tick in 0..60 {
        assert_eq!(restored.tick(), session.tick(), "tick {tick}");
    }
    let player = session.player();
    assert!(player.motion.grounded);
    assert_eq!(player.region, FLOOR);
}

impl Session {
    /// North past the spawn, east to the stairs' line, then south up them (#222); every
    /// player snapshot of the climb.
    fn up_the_stairs(&mut self) -> Vec<PlayerSnapshot> {
        for (x, z, ticks) in [(0, -1, 37), (1, 0, 37)] {
            self.send(ArpgCommand::SetMovement { x, z });
            for _ in 0..ticks {
                self.tick();
            }
        }
        self.send(ArpgCommand::SetMovement { x: 0, z: 1 });
        (0..30).map(|_| self.tick()).collect()
    }
}

#[test]
fn a_walking_player_climbs_the_stairs_onto_the_platform_without_jumping() {
    let mut session = Session::new();
    let climb = session.up_the_stairs();
    let heights = climb
        .iter()
        .map(|player| player.motion.feet_y)
        .collect::<Vec<_>>();
    // Up through both steps (20, 40) to the platform's top, never higher than a step above
    // the ground it stood on, and never jumping.
    for rise in [20, 40, PLATFORM_TOP] {
        assert!(heights.contains(&rise), "{heights:?}");
    }
    assert!(
        heights.windows(2).all(|pair| pair[1] - pair[0] <= 25),
        "{heights:?}"
    );
    assert!(
        climb
            .iter()
            .all(|player| player.motion.vertical_velocity <= 0),
        "{climb:?}"
    );
    let top = climb.last().unwrap();
    assert!(top.motion.grounded);
    assert_eq!(top.motion.feet_y, PLATFORM_TOP);
    assert_eq!(top.region, PLATFORM);
    // On the steps it kept the floor's membership: stairs belong to the region they lead to.
    assert!(
        climb
            .iter()
            .filter(|player| (1..PLATFORM_TOP - 5).contains(&player.motion.feet_y))
            .all(|player| player.region == FLOOR)
    );
}

#[test]
fn a_save_on_a_step_continues_exactly() {
    let mut session = Session::new();
    for (x, z, ticks) in [(0, -1, 37), (1, 0, 37), (0, 1, 12)] {
        session.send(ArpgCommand::SetMovement { x, z });
        for _ in 0..ticks {
            session.tick();
        }
    }
    session.send(ArpgCommand::SetMovement { x: 0, z: 0 });
    for _ in 0..3 {
        session.tick();
    }
    let player = session.player();
    assert!(player.motion.grounded);
    assert!((15..=45).contains(&player.motion.feet_y), "{player:?}");
    let save = session.game.save_state().unwrap();
    let mut restored = Session {
        game: ArpgGame::from_save_state(save).unwrap(),
        sequence: session.sequence,
    };
    session.send(ArpgCommand::SetMovement { x: 0, z: 1 });
    restored.send(ArpgCommand::SetMovement { x: 0, z: 1 });
    for tick in 0..30 {
        assert_eq!(restored.tick(), session.tick(), "tick {tick}");
    }
}
