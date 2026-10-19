//! Seeded whole-game workloads on the actual game tick: replay determinism for wall, corner,
//! door, correction, combat, crowd, lifecycle, chase and jump play, and the chase's navigation
//! work against the per-tick reference planner (#110).
use super::*;
use std::time::{Duration, Instant};

// The test-only replay below; `fixture`, `commands` and `Case` also drive the `physics-ab`
// comparison and timing (physics-engine#330).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Case {
    Quiet,
    PartyQuiet,
    Sparse,
    Corner,
    Door,
    Correction,
    Combat,
    Crowd,
    Lifecycle,
    Chase,
    Jump,
    /// The traversal scenario (#208): jumping onto a platform, walking off its ledge and
    /// jumping back on from the other side.
    Ledge,
    /// The traversal scenario's low ceiling (#209): walking under it and jumping into it.
    Ceiling,
    /// The traversal scenario end to end (#210): onto the platform, up onto the bridge,
    /// off its side and back along the floor path beneath it.
    Traversal,
    /// A skirmisher chasing a player who keeps stepping away: its lunges sweep (#90).
    Lunge,
    /// Breaking a crate in the corner of a combat room and walking through its spot (#90).
    Crates,
    /// Walking up the traversal scenario's stairs onto the platform without jumping (#222).
    Stairs,
    /// Three players inside a bruiser's telegraphed ground burst (#64): an area query.
    Slam,
    /// Walking into a combat room's poison pool, lingering and walking out (#64): an area
    /// query every tick.
    Poison,
}
impl Case {
    pub(super) const ALL: [Self; 19] = [
        Self::Quiet,
        Self::PartyQuiet,
        Self::Sparse,
        Self::Corner,
        Self::Door,
        Self::Correction,
        Self::Combat,
        Self::Crowd,
        Self::Lifecycle,
        Self::Chase,
        Self::Jump,
        Self::Ledge,
        Self::Ceiling,
        Self::Traversal,
        Self::Lunge,
        Self::Crates,
        Self::Stairs,
        Self::Slam,
        Self::Poison,
    ];
    /// The workloads whose saves restore: the others move monsters out of their generated
    /// rooms, which a save rejects.
    #[cfg(test)]
    pub(super) const SAVED: [Self; 14] = [
        Self::Sparse,
        Self::Corner,
        Self::Door,
        Self::Correction,
        Self::Combat,
        Self::Lifecycle,
        Self::Jump,
        Self::Ledge,
        Self::Ceiling,
        Self::Traversal,
        Self::Crates,
        Self::Stairs,
        Self::Slam,
        Self::Poison,
    ];
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Quiet => "quiet",
            Self::PartyQuiet => "party-quiet",
            Self::Sparse => "sparse",
            Self::Corner => "corner",
            Self::Door => "door",
            Self::Correction => "correction",
            Self::Combat => "combat",
            Self::Crowd => "crowd",
            Self::Lifecycle => "lifecycle",
            Self::Chase => "chase",
            Self::Jump => "jump",
            Self::Ledge => "ledge",
            Self::Ceiling => "ceiling",
            Self::Traversal => "traversal",
            Self::Lunge => "lunge",
            Self::Crates => "crates",
            Self::Stairs => "stairs",
            Self::Slam => "slam",
            Self::Poison => "poison",
        }
    }
}
pub(super) fn fixture(case: Case, seed: u32) -> ArpgGame {
    let mut game = if matches!(
        case,
        Case::Ledge | Case::Ceiling | Case::Traversal | Case::Stairs
    ) {
        ArpgGame::arranged_scenario(ScenarioId::Traversal, seed).unwrap()
    } else if case == Case::Slam {
        ArpgGame::arranged_scenario(ScenarioId::Heavy, seed).unwrap()
    } else {
        ArpgGame::new_with_seed(seed).unwrap()
    };
    match case {
        // Jumping into the outer wall the corner case presses into: airborne sliding, landing.
        Case::Poison => {
            // East of room 2's pool, walking into it.
            let pool = game
                .pools
                .iter()
                .find(|pool| pool.room_id == 2)
                .copied()
                .unwrap();
            game.player_spawns[0] = Vec3i::new(pool.position.x + 220, PLAYER_Y, pool.position.z);
        }
        Case::Crates => {
            // East of the north-west crate of room 2, facing it.
            let placed = game
                .crates
                .iter()
                .find(|placed| placed.room_id == 2)
                .copied()
                .unwrap();
            game.player_spawns[0] = Vec3i::new(placed.position.x + 90, PLAYER_Y, placed.position.z);
        }
        Case::Lunge => {
            // The player stands at the centre of the first skirmisher's room.
            let room_id = game
                .monsters
                .iter()
                .find(|monster| *game.content.id(monster.definition().id) == *"monster.skirmisher")
                .unwrap()
                .room_id;
            let (x, z) = game
                .rooms
                .iter()
                .find(|room| room.id == room_id)
                .unwrap()
                .center();
            game.player_spawns[0] = Vec3i::new(x, PLAYER_Y, z);
        }
        Case::Corner | Case::Jump => {
            game.player_spawns[0] = Vec3i::new(-2945, PLAYER_Y, -1745);
        }
        Case::Correction => game.player_spawns[0] = Vec3i::new(-2970, PLAYER_Y, -1600),
        Case::Door => {
            let door = game
                .doors
                .iter()
                .find(|door| door.room_a == 1 && door.room_b == 2)
                .unwrap();
            game.player_spawns[0] = Vec3i::new(door.position[0] - 90, PLAYER_Y, door.position[2]);
        }
        Case::Combat | Case::Crowd => {
            let (x, z) = game
                .rooms
                .iter()
                .find(|room| room.id == 2)
                .unwrap()
                .center();
            game.player_spawns[0] = Vec3i::new(x, PLAYER_Y, z);
            if case == Case::Combat {
                game.monsters
                    .iter_mut()
                    .find(|monster| monster.room_id == 2)
                    .unwrap()
                    .position = Vec3i::new(x + 100, PLAYER_Y, z).into();
            } else {
                // Relocate all five generated enemies; use ordinary encounter/attack rules.
                for (index, monster) in game.monsters.iter_mut().enumerate() {
                    monster.room_id = 2;
                    monster.position = Vec3i::new(
                        x + if index % 2 == 0 { 100 } else { -100 },
                        PLAYER_Y,
                        z + (i32::try_from(index).unwrap() - 2) * 50,
                    )
                    .into();
                }
            }
        }
        Case::Chase => {
            // Every generated enemy chases one player across room 2 from its far edges.
            let room = game.rooms.iter().find(|room| room.id == 2).unwrap();
            let (x, z) = room.center();
            let (west, east) = (room.min_x + 120, room.max_x - 120);
            game.player_spawns[0] = Vec3i::new(x, PLAYER_Y, z);
            for (index, monster) in game.monsters.iter_mut().enumerate() {
                let index = i32::try_from(index).unwrap();
                monster.room_id = 2;
                monster.position = Vec3i::new(
                    if index % 2 == 0 { west } else { east },
                    PLAYER_Y,
                    z + (index - 2) * 90,
                )
                .into();
            }
        }
        _ => {}
    }
    let count = if matches!(case, Case::PartyQuiet | Case::Sparse | Case::Lifecycle) {
        4
    } else if case == Case::Slam {
        3
    } else {
        1
    };
    for id in 1..=count {
        game.add_player(id).unwrap();
    }
    game.reconcile_encounters().unwrap();
    game
}
pub(super) fn commands(game: &mut ArpgGame, case: Case, tick: u32) -> (Vec<String>, Duration) {
    let mut outcomes = Vec::new();
    let mut elapsed = Duration::ZERO;
    if case == Case::Lifecycle {
        if tick == 40 {
            let start = Instant::now();
            let removed = game.remove_player(4);
            elapsed += start.elapsed();
            outcomes.push(format!("remove:4:{removed}"));
        }
        if tick == 70 {
            let start = Instant::now();
            let added = game.add_player(4);
            elapsed += start.elapsed();
            outcomes.push(format!("add:4:{added:?}"));
        }
    }
    let command = match case {
        Case::Sparse | Case::Lifecycle if tick.is_multiple_of(24) => {
            let (x, z) = [(1, 0), (0, 1), (-1, 0), (0, -1)][(tick / 24) as usize % 4];
            Some(ArpgCommand::SetMovement { x, z })
        }
        Case::Corner if tick == 0 => Some(ArpgCommand::SetMovement { x: -1, z: -1 }),
        Case::Door if tick == 0 => Some(ArpgCommand::SetMovement { x: 1, z: 0 }),
        Case::Jump if tick == 0 => Some(ArpgCommand::SetMovement { x: -1, z: 1 }),
        Case::Jump if tick % 40 == 5 => Some(ArpgCommand::Jump),
        // Onto the platform, off its south side, and back up onto it from the south.
        Case::Ledge if tick == 0 => Some(ArpgCommand::SetMovement { x: 1, z: 0 }),
        Case::Ledge if tick == 40 => Some(ArpgCommand::SetMovement { x: 0, z: 1 }),
        Case::Ledge if tick == 70 => Some(ArpgCommand::SetMovement { x: 0, z: -1 }),
        Case::Ledge if tick == 7 || tick == 78 => Some(ArpgCommand::Jump),
        // North under the ceiling, a standing jump into it, then jumps while walking out.
        Case::Ceiling if tick == 0 => Some(ArpgCommand::SetMovement { x: 0, z: -1 }),
        Case::Ceiling if tick == 36 => Some(ArpgCommand::SetMovement { x: 0, z: 0 }),
        Case::Ceiling if tick == 70 => Some(ArpgCommand::SetMovement { x: 1, z: 0 }),
        Case::Ceiling if tick == 40 || tick == 72 || tick == 100 => Some(ArpgCommand::Jump),
        // A full-draw arrow east at the room's far crate (#223), then the sword on the near
        // one, and a walk through its spot.
        Case::Crates if tick == 0 => Some(ArpgCommand::EquipWeapon {
            weapon: Weapon::Bow,
        }),
        Case::Crates if tick == 1 => Some(ArpgCommand::SetAim {
            direction: Some([1_000, 0]),
        }),
        Case::Crates if tick == 2 => Some(ArpgCommand::DrawBow),
        Case::Crates if tick == 34 => Some(ArpgCommand::ReleaseBow { aim: None }),
        Case::Crates if tick == 70 => Some(ArpgCommand::EquipWeapon {
            weapon: Weapon::SwordAndShield,
        }),
        Case::Crates if tick == 72 || tick == 88 => Some(ArpgCommand::PrimaryAttack {
            aim: Some([-1_000, 0]),
        }),
        Case::Crates if tick == 104 => Some(ArpgCommand::SetMovement { x: -1, z: 0 }),
        // North, east to the stairs' line, then south up them onto the platform.
        Case::Poison if tick == 0 => Some(ArpgCommand::SetMovement { x: -1, z: 0 }),
        Case::Poison if tick == 30 => Some(ArpgCommand::SetMovement { x: 0, z: 0 }),
        Case::Poison if tick == 70 => Some(ArpgCommand::SetMovement { x: 1, z: -1 }),
        // A raised guard does not block the burst; stepping away makes the next one miss.
        Case::Slam if tick == 0 => Some(ArpgCommand::SetGuard { raised: true }),
        Case::Slam if tick == 50 => Some(ArpgCommand::SetMovement { x: -1, z: 0 }),
        Case::Slam if tick == 90 => Some(ArpgCommand::SetMovement { x: 0, z: 0 }),
        Case::Stairs if tick == 0 => Some(ArpgCommand::SetMovement { x: 0, z: -1 }),
        Case::Stairs if tick == 37 => Some(ArpgCommand::SetMovement { x: 1, z: 0 }),
        Case::Stairs if tick == 74 => Some(ArpgCommand::SetMovement { x: 0, z: 1 }),
        Case::Stairs if tick == 104 => Some(ArpgCommand::SetMovement { x: 0, z: 0 }),
        // Stepping away and pausing, so lunges start both in and out of static reach.
        Case::Lunge if tick.is_multiple_of(20) => {
            let (x, z) =
                [(-1, 0), (0, 0), (0, 1), (0, 0), (0, -1), (0, 0)][(tick / 20) as usize % 6];
            Some(ArpgCommand::SetMovement { x, z })
        }
        Case::Traversal if tick == 0 => Some(ArpgCommand::SetMovement { x: 1, z: 0 }),
        Case::Traversal if tick == 7 || tick == 47 => Some(ArpgCommand::Jump),
        Case::Traversal if tick == 75 => Some(ArpgCommand::SetMovement { x: 0, z: 1 }),
        Case::Traversal if tick == 100 => Some(ArpgCommand::SetMovement { x: 0, z: -1 }),
        Case::Traversal if tick == 115 => Some(ArpgCommand::SetMovement { x: 0, z: 0 }),
        Case::Combat if tick < 64 && tick.is_multiple_of(16) => {
            Some(ArpgCommand::PrimaryAttack { aim: None })
        }
        Case::Combat if tick == 64 => Some(ArpgCommand::SecondaryAttack { aim: None }),
        Case::Combat if tick == 96 => Some(ArpgCommand::Interact),
        // The target walks a square with pauses, crossing navigation cells as it goes.
        Case::Chase if tick.is_multiple_of(15) => {
            let (x, z) = [
                (1, 0),
                (0, 0),
                (0, 1),
                (0, 0),
                (-1, 0),
                (0, 0),
                (0, -1),
                (0, 0),
            ][(tick / 15) as usize % 8];
            Some(ArpgCommand::SetMovement { x, z })
        }
        Case::Crowd if tick.is_multiple_of(24) => Some(if tick < 96 {
            ArpgCommand::PrimaryAttack { aim: None }
        } else {
            ArpgCommand::Interact
        }),
        _ => None,
    };
    if let Some(command) = command {
        let input = PlayerCommand::new(1, tick + 1, command).unwrap();
        let start = Instant::now();
        let result = game.apply_command(input);
        elapsed += start.elapsed();
        outcomes.push(format!("{command:?}:{result:?}"));
    }
    (outcomes, elapsed)
}
#[cfg(test)]
fn timed_tick(game: &mut ArpgGame) -> Duration {
    let start = Instant::now();
    game.advance_tick().unwrap();
    start.elapsed()
}
/// One frame of the trace: the snapshot payload and the private state it does not show.
#[cfg(test)]
pub(super) fn frame(game: &ArpgGame, payload: &[u8], commands: &[String]) -> Vec<u8> {
    let hidden = format!(
        "{:?}",
        (
            &game.players,
            &game.last_sequences,
            &game.monsters,
            &game.ground_loot,
            game.next_ground_loot_id,
            game.player_spawns,
            &game.physics,
            commands
        )
    );
    let mut bytes = Vec::new();
    for part in [payload, hidden.as_bytes()] {
        bytes.extend_from_slice(&(part.len() as u64).to_le_bytes());
        bytes.extend_from_slice(part);
    }
    bytes
}
#[cfg(test)]
struct Measurement {
    trace: Vec<u8>,
    tick: u128,
    end: ArpgSnapshot,
    navigation: NavigationWork,
    /// Largest per-tick navigation work, by expansions.
    peak_navigation_tick: NavigationWork,
}
#[cfg(test)]
fn replay(case: Case, seed: u32, ticks: u32) -> Measurement {
    replay_navigating(case, seed, ticks, NavigationMode::Retained)
}
#[cfg(test)]
fn replay_navigating(case: Case, seed: u32, ticks: u32, mode: NavigationMode) -> Measurement {
    let mut game = fixture(case, seed);
    game.navigation_mode = mode;
    let mut m = Measurement {
        trace: Vec::new(),
        tick: 0,
        end: game.snapshot().unwrap(),
        navigation: NavigationWork::default(),
        peak_navigation_tick: NavigationWork::default(),
    };
    for tick in 0..ticks {
        let (command_outcomes, _) = commands(&mut game, case, tick);
        let before = game.navigation_work();
        let elapsed = timed_tick(&mut game);
        let tick_work = game.navigation_work().since(before);
        if tick_work.expansions > m.peak_navigation_tick.expansions {
            m.peak_navigation_tick = tick_work;
        }
        m.tick += elapsed.as_nanos();
        let snapshot = game.snapshot().unwrap();
        let payload = serde_json::to_vec(&snapshot).unwrap();
        m.trace.extend(frame(&game, &payload, &command_outcomes));
        m.end = snapshot;
    }
    m.navigation = game.navigation_work();
    assert_eq!(m.end.tick, u64::from(ticks));
    match case {
        Case::Corner => {
            assert!(m.end.players[0].position[0] >= -2945);
            assert!(m.end.players[0].position[2] >= -1745);
        }
        Case::Correction => assert!(m.end.players[0].position[0] >= -2945),
        Case::Door if ticks >= 120 => assert!(m.end.doors.iter().any(|door| door.locked)),
        Case::Combat if ticks >= 120 => {
            assert_eq!(m.end.players[0].experience, 50);
            assert_eq!(m.end.players[0].gold, 10);
        }
        Case::Lifecycle if ticks >= 120 => assert_eq!(m.end.players.len(), 4),
        Case::Chase if ticks >= 120 => {
            assert!(m.navigation.pursuit_ticks > u64::from(ticks));
            assert!(m.end.monsters.iter().all(|monster| monster.health > 0));
        }
        _ => {}
    }
    m
}
#[test]
fn actual_seeded_ticks_preserve_wall_door_correction_combat_and_lifecycle_acceptance() {
    for seed in [42, 0xDEAD_BEEF] {
        for case in Case::ALL {
            let first = replay(case, seed, 120);
            let second = replay(case, seed, 120);
            assert_eq!(first.trace, second.trace, "{} seed {seed}", case.name());
        }
    }
}

/// The chase workload's navigation work against the #65 per-tick planner (#110): the
/// retained planner must give the same game with its cache dropped every tick, and needs
/// far fewer expansions, physics queries and grid builds than the per-tick reference.
#[test]
fn chase_navigation_work_against_the_per_tick_reference() {
    for seed in [42, 0xDEAD_BEEF, 0xA420_0916] {
        let retained = replay_navigating(Case::Chase, seed, 120, NavigationMode::Retained);
        let uncached =
            replay_navigating(Case::Chase, seed, 120, NavigationMode::RetainedWithoutCache);
        let reference = replay_navigating(Case::Chase, seed, 120, NavigationMode::PerTickReference);
        assert_eq!(retained.trace, uncached.trace, "seed {seed}");
        let (work, base) = (retained.navigation, reference.navigation);
        println!(
            "ARPG_NAVIGATION {}",
            serde_json::json!({
                "seed": seed, "ticks": 120,
                "retained": format!("{work:?}"),
                "retained_peak_tick": format!("{:?}", retained.peak_navigation_tick),
                "reference": format!("{base:?}"),
                "reference_peak_tick": format!("{:?}", reference.peak_navigation_tick),
                "uncached_expansions": uncached.navigation.expansions,
                // Advisory: whole ticks, in the profile the test runs in.
                "retained_tick_ms": retained.tick as f64 / 1e6,
                "uncached_tick_ms": uncached.tick as f64 / 1e6,
                "reference_tick_ms": reference.tick as f64 / 1e6,
            })
        );
        assert_eq!(
            base.repaths(),
            base.pursuit_ticks,
            "the reference replans every tick"
        );
        assert!(
            work.expansions * 2 < base.expansions,
            "seed {seed}: {work:?} {base:?}"
        );
        assert!(
            work.physics_queries * 2 < base.physics_queries,
            "seed {seed}"
        );
        assert!(
            work.repaths() * 2 < work.pursuit_ticks,
            "seed {seed}: {work:?}"
        );
        assert_eq!(work.grid_builds, 1, "one room grid, never rebuilt");
    }
}

/// A save taken at any tick continues exactly as the uninterrupted game (#175): the restored
/// game rebuilds its Rapier world from the seed and the saved game state, and every later
/// tick publishes the same snapshot (with its events) and saves the same `f32` state.
#[test]
fn saves_taken_mid_play_continue_exactly_as_the_uninterrupted_game() {
    for seed in [42, 0xDEAD_BEEF] {
        for case in Case::SAVED {
            for save_at in [1, 25, 60] {
                let mut game = fixture(case, seed);
                for tick in 0..save_at {
                    commands(&mut game, case, tick);
                    game.advance_tick().unwrap();
                }
                let mut restored = ArpgGame::from_save_state(game.save_state().unwrap()).unwrap();
                for tick in save_at..120 {
                    let (expected, _) = commands(&mut game, case, tick);
                    let (outcomes, _) = commands(&mut restored, case, tick);
                    assert_eq!(outcomes, expected);
                    game.advance_tick().unwrap();
                    restored.advance_tick().unwrap();
                    let context = format!(
                        "{} seed {seed} saved at {save_at}, tick {tick}",
                        case.name()
                    );
                    assert_eq!(
                        restored.snapshot().unwrap(),
                        game.snapshot().unwrap(),
                        "{context}"
                    );
                    assert_eq!(
                        restored.save_state().unwrap(),
                        game.save_state().unwrap(),
                        "{context}"
                    );
                }
            }
        }
    }
}
