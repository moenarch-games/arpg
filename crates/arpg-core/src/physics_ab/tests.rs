//! Merge-blocking A/B acceptance of the physics-engine backend on the real game
//! (physics-engine#330). Run with `cargo test -p arpg-core --features physics-ab physics_ab`.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use rapier3d::math::Vector;

use super::*;
use crate::physics::BodyKey;
use crate::physics::vector;
use crate::seeded_workloads::frame;
use crate::{FLOOR_FOOT_Y, MONSTER_BODY_HALF_EXTENTS, PLAYER_HALF_EXTENTS, PLAYER_Y, Vec3i};

const SEEDS: [u32; 2] = [42, 0xDEAD_BEEF];
const TICKS: u32 = 120;

/// One physical body of the game after a tick.
#[derive(Clone, Copy, Debug)]
struct Body {
    position: Vector,
    half_extents: Vec3i,
    /// Players' support; monsters move on the plane.
    grounded: Option<bool>,
}

/// Every body the physics world moves: players, and monsters while they have a body.
fn bodies(game: &ArpgGame) -> BTreeMap<BodyKey, Body> {
    let players = game.players.iter().map(|(&id, state)| {
        (
            ArpgGame::player_body_id(id),
            Body {
                position: state.position,
                half_extents: PLAYER_HALF_EXTENTS,
                grounded: Some(state.grounded),
            },
        )
    });
    let monsters = game
        .monsters
        .iter()
        .map(|monster| (ArpgGame::monster_body_id(monster.id), monster.position))
        .filter(|&(key, _)| game.physics.has_character(key))
        .map(|(key, position)| {
            (
                key,
                Body {
                    position,
                    half_extents: MONSTER_BODY_HALF_EXTENTS,
                    grounded: None,
                },
            )
        });
    players.chain(monsters).collect()
}

/// Distance from `point` to the box centred on `center` (0 inside it).
fn point_box_distance(point: Vector, center: Vector, half: Vector) -> f32 {
    let outside = ((point - center).abs() - half).max(Vector::ZERO);
    outside.length()
}

/// Gap (world units) between an upright capsule body and a static box; negative when the
/// axis enters the box. The distance from a point moving along the axis to a convex box is
/// convex, so a ternary search finds its minimum.
fn capsule_box_gap(body: &Body, center: Vec3i, half_extents: Vec3i) -> f32 {
    let (radius, half_height) = crate::physics::capsule_dimensions(body.half_extents);
    let (center, half) = (vector(center), vector(half_extents));
    let at = |t: f32| point_box_distance(body.position + Vector::Y * t, center, half);
    let (mut low, mut high) = (-half_height, half_height);
    for _ in 0..64 {
        let (a, b) = (low + (high - low) / 3.0, high - (high - low) / 3.0);
        if at(a) <= at(b) {
            high = b;
        } else {
            low = a;
        }
    }
    let distance = at((low + high) / 2.0);
    if distance == 0.0 {
        -radius
    } else {
        distance - radius
    }
}

/// Gap (world units) between two upright capsule bodies.
fn capsule_gap(a: &Body, b: &Body) -> f32 {
    let (ra, ha) = crate::physics::capsule_dimensions(a.half_extents);
    let (rb, hb) = crate::physics::capsule_dimensions(b.half_extents);
    let d = b.position - a.position;
    let planar = Vector::new(d.x, 0.0, d.z).length();
    let vertical = (d.y.abs() - ha - hb).max(0.0);
    planar.hypot(vertical) - ra - rb
}

/// Whether the segment from `from` to `to` passes through the box's interior (slab test).
fn segment_crosses_box(from: Vector, to: Vector, center: Vec3i, half_extents: Vec3i) -> bool {
    let (center, half) = (vector(center), vector(half_extents));
    let (low, high) = (center - half, center + half);
    let direction = to - from;
    let (mut enter, mut leave) = (0.0_f32, 1.0_f32);
    for axis in 0..3 {
        let (origin, step) = (from[axis], direction[axis]);
        if step == 0.0 {
            if origin <= low[axis] || origin >= high[axis] {
                return false;
            }
            continue;
        }
        let (t0, t1) = ((low[axis] - origin) / step, (high[axis] - origin) / step);
        enter = enter.max(t0.min(t1));
        leave = leave.min(t0.max(t1));
    }
    enter < leave
}

/// Gap (world units) between a body and the nearest ground: the floor or any static box,
/// so a body rounding a platform's edge is still on it (#208).
fn ground_gap(body: &Body, boxes: &[(Vec3i, Vec3i)]) -> f32 {
    let feet = body.position.y - body.half_extents.y as f32;
    boxes
        .iter()
        .map(|&(center, half)| capsule_box_gap(body, center, half))
        .fold(feet - FLOOR_FOOT_Y as f32, f32::min)
}

/// Whether a grounded player rests on an edge rather than a top: the ground straight below
/// is further than the support reach (#208).
fn on_an_edge(game: &ArpgGame, body: &Body) -> bool {
    let feet = body.position.y - body.half_extents.y as f32;
    body.grounded == Some(true)
        && game
            .physics
            .ground_height(body.position)
            .is_none_or(|ground| feet - ground > SUPPORT_REACH_UNITS)
}

/// Penetration accepted by the invariants (world units): the backends keep a 0.25 unit gap,
/// so a hundredth of a unit is far below any real overlap and well above `f32` rounding of
/// dungeon-sized coordinates (about 2e-4 units at 3 000 units).
const PENETRATION: f32 = 1.0e-2;
/// How far above the floor a grounded body's feet may be: the support reach in world units.
const SUPPORT_REACH_UNITS: f32 = 5.25;

/// The shared physical invariants after one tick; `previous` holds the bodies of the tick
/// before. Returns every violation found.
fn invariants(
    game: &ArpgGame,
    previous: &BTreeMap<BodyKey, Body>,
    current: &BTreeMap<BodyKey, Body>,
) -> Vec<String> {
    let mut violations = Vec::new();
    let boxes = game.physics.fixed_boxes().collect::<Vec<_>>();
    for (key, body) in current {
        let p = body.position;
        if !p.is_finite() {
            violations.push(format!("body {key} not finite: {p:?}"));
            continue;
        }
        let feet = p.y - body.half_extents.y as f32;
        if feet < FLOOR_FOOT_Y as f32 - PENETRATION {
            violations.push(format!("body {key} below the floor: feet {feet}"));
        }
        match body.grounded {
            Some(true) if ground_gap(body, &boxes) > SUPPORT_REACH_UNITS + PENETRATION => {
                violations.push(format!(
                    "body {key} grounded in the air: {} above the ground",
                    ground_gap(body, &boxes)
                ));
            }
            None if p.y != PLAYER_Y as f32 => {
                violations.push(format!("monster {key} left its plane: {p:?}"));
            }
            _ => {}
        }
        for &(center, half) in &boxes {
            let gap = capsule_box_gap(body, center, half);
            if gap < -PENETRATION {
                violations.push(format!("body {key} penetrates box {center:?} by {gap}"));
            }
            if let Some(before) = previous.get(key)
                && segment_crosses_box(before.position, p, center, half)
            {
                violations.push(format!("body {key} tunnelled through box {center:?}"));
            }
        }
    }
    let all = current.iter().collect::<Vec<_>>();
    for (index, (a_key, a)) in all.iter().enumerate() {
        for (b_key, b) in &all[index + 1..] {
            let gap = capsule_gap(a, b);
            if gap < -PENETRATION {
                violations.push(format!("bodies {a_key} and {b_key} overlap by {gap}"));
            }
        }
    }
    violations
}

/// Game-visible outcomes that do not depend on exact positions: doors, encounters, health,
/// progression, support, region membership and every strike and interaction decision.
fn outcomes(game: &ArpgGame) -> String {
    let snapshot = game.snapshot().unwrap();
    let mut out = String::new();
    for door in &snapshot.doors {
        write!(out, "door {} {};", door.id, door.locked).unwrap();
    }
    for room in &snapshot.rooms {
        write!(out, "room {} {:?};", room.id, room.encounter_state).unwrap();
    }
    for player in &snapshot.players {
        write!(
            out,
            "player {} {} {} {} {} {} {:?};",
            player.id,
            player.health,
            player.experience,
            player.gold,
            player.alive,
            player.motion.grounded,
            player.region
        )
        .unwrap();
    }
    for monster in &snapshot.monsters {
        write!(
            out,
            "monster {} {} {};",
            monster.id, monster.health, monster.alive
        )
        .unwrap();
    }
    for event in &snapshot.strike_events {
        write!(
            out,
            "strike {:?} {:?} {:?};",
            event.source, event.target, event.result
        )
        .unwrap();
    }
    for event in &snapshot.interaction_events {
        write!(out, "interaction {event:?};").unwrap();
    }
    write!(out, "loot {}", snapshot.ground_loot.len()).unwrap();
    out
}

/// One run's trace: every snapshot with the private state it does not show.
fn replay(backend: Backend, case: Case, seed: u32) -> Vec<u8> {
    with_backend(backend, || {
        let mut game = fixture(case, seed);
        let mut trace = Vec::new();
        for tick in 0..TICKS {
            let (outcomes, _) = commands(&mut game, case, tick);
            game.advance_tick().unwrap();
            let payload = serde_json::to_vec(&game.snapshot().unwrap()).unwrap();
            trace.extend(frame(&game, &payload, &outcomes));
        }
        trace
    })
}

/// Cross-backend comparison of one workload.
#[derive(Debug, Default)]
struct Comparison {
    /// The first tick after which two characters touch in either game.
    first_contact: Option<u32>,
    /// Largest position difference of any body present in both games (world units) before
    /// the first character contact, and where it was.
    max_position: f32,
    max_position_at: Option<(u32, BodyKey)>,
    /// Highest feet above the floor of any player in either game (world units).
    max_height: f32,
    /// The same after the first character contact, where the backends' character-character
    /// handling intentionally differs.
    max_position_after_contact: f32,
    /// The same at ticks on which either body rounds a platform edge, where Rapier's
    /// normal nudge can skip a snap (bounded by [`EDGE_POSITION`]).
    max_position_on_edge: f32,
    /// Ticks on which a player of either game belongs to a region above the floor.
    raised_region_ticks: u32,
    /// Ticks on which an airborne player's head touches a ceiling's underside, in either game.
    ceiling_touches: u32,
    /// First tick whose game-visible outcomes differ.
    first_outcome_difference: Option<(u32, String, String)>,
    /// Ticks whose static geometry (locked doors) differs, so queries are not compared.
    geometry_differs: u32,
    queries: u32,
    query_differences: Vec<String>,
    violations: BTreeMap<&'static str, Vec<String>>,
}

/// Deterministic placements across the dungeon for one tick.
fn placements(game: &ArpgGame, tick: u32) -> Vec<Vector> {
    let (min_x, max_x, min_z, max_z) = game.rooms.iter().fold(
        (i32::MAX, i32::MIN, i32::MAX, i32::MIN),
        |(a, b, c, d), room| {
            (
                a.min(room.min_x),
                b.max(room.max_x),
                c.min(room.min_z),
                d.max(room.max_z),
            )
        },
    );
    let mut state = 0x9e37_79b9_u64 ^ u64::from(tick).wrapping_mul(0x5851_f42d_4c95_7f2d);
    let mut next = |low: i32, high: i32| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        low + ((state >> 33) % u64::from((high - low) as u32 + 1)) as i32
    };
    (0..8)
        .map(|_| vector(Vec3i::new(next(min_x, max_x), PLAYER_Y, next(min_z, max_z))))
        .collect()
}

/// The query decisions of both backends from the same inputs: sight and strikes between
/// every pair of bodies, rays from every body in eight directions, ground under every body
/// and penetration at fixed placements. Inputs are the Rapier game's positions.
fn compare_queries(
    tick: u32,
    rapier: &ArpgGame,
    engine: &ArpgGame,
    bodies: &BTreeMap<BodyKey, Body>,
    comparison: &mut Comparison,
) {
    let mut differ = |what: String| {
        comparison
            .query_differences
            .push(format!("tick {tick}: {what}"))
    };
    let positions = bodies
        .values()
        .map(|body| body.position)
        .collect::<Vec<_>>();
    let mut queries = 0;
    for &from in &positions {
        for &to in &positions {
            let to = Vector::new(to.x, from.y, to.z);
            queries += 1;
            let (a, b) = (
                rapier.physics.segment_obstructed(from, to),
                engine.physics.segment_obstructed(from, to),
            );
            if a != b {
                differ(format!("sight {from:?} -> {to:?}: rapier {a}, engine {b}"));
            }
        }
        for direction in 0..8 {
            let angle = direction as f32 * std::f32::consts::FRAC_PI_4 + 0.1;
            let translation = Vector::new(angle.cos(), 0.0, angle.sin()) * 400.0;
            queries += 1;
            let (a, b) = (
                rapier.physics.first_fixed_hit(from, translation),
                engine.physics.first_fixed_hit(from, translation),
            );
            let agree = match (a, b) {
                (Some(a), Some(b)) => (a - b).abs() * 400.0 <= RAY_DISTANCE,
                (a, b) => a == b,
            };
            if !agree {
                differ(format!(
                    "ray {from:?} {translation:?}: rapier {a:?}, engine {b:?}"
                ));
            }
        }
        queries += 1;
        let (a, b) = (
            rapier.physics.ground_height(from),
            engine.physics.ground_height(from),
        );
        let agree = match (a, b) {
            (Some(a), Some(b)) => (a - b).abs() <= RAY_DISTANCE,
            (a, b) => a == b,
        };
        if !agree {
            differ(format!("ground {from:?}: rapier {a:?}, engine {b:?}"));
        }
    }
    for position in placements(rapier, tick) {
        for half in [PLAYER_HALF_EXTENTS, MONSTER_BODY_HALF_EXTENTS] {
            queries += 1;
            let (a, b) = (
                rapier.physics.penetrates_fixed(position, half),
                engine.physics.penetrates_fixed(position, half),
            );
            if a != b {
                differ(format!("placement {position:?}: rapier {a}, engine {b}"));
            }
        }
    }
    comparison.queries += queries;
}

/// Accepted difference (world units) between the backends' ray distances: both solve the
/// same closed forms in `f32` metres, so they agree to rounding of dungeon-sized coordinates.
const RAY_DISTANCE: f32 = 1.0e-2;

/// Characters closer than this (world units) touch: the backends keep 0.25 units apart.
const TOUCHING: f32 = 1.0;

/// Whether any two characters touch.
fn touching(bodies: &BTreeMap<BodyKey, Body>) -> bool {
    let all = bodies.values().collect::<Vec<_>>();
    all.iter().enumerate().any(|(index, a)| {
        all[index + 1..]
            .iter()
            .any(|b| capsule_gap(a, b) < TOUCHING)
    })
}

/// Every locked door obstructs sight and strikes through its centre, along both axes.
fn doors_block(game: &ArpgGame, tick: u32, comparison: &mut Comparison) {
    for door in game.doors.iter().filter(|door| door.locked) {
        let center = vector(crate::array_to_vec(door.position));
        for along in [Vector::X, Vector::Z] {
            let (from, to) = (center - along * 200.0, center + along * 200.0);
            if !game.physics.segment_obstructed(from, to) {
                comparison.query_differences.push(format!(
                    "tick {tick}: locked door {} does not block",
                    door.id
                ));
            }
        }
    }
}

/// Runs one workload on both backends in lockstep: the same fixture, commands and ticks.
fn compare(case: Case, seed: u32) -> Comparison {
    let mut comparison = Comparison::default();
    let mut rapier = with_backend(Backend::Rapier, || fixture(case, seed));
    let mut engine = with_backend(Backend::Engine, || fixture(case, seed));
    let (mut rapier_before, mut engine_before) = (bodies(&rapier), bodies(&engine));
    for tick in 0..TICKS {
        let (rapier_commands, _) = commands(&mut rapier, case, tick);
        let (engine_commands, _) = commands(&mut engine, case, tick);
        assert_eq!(
            rapier_commands,
            engine_commands,
            "{} tick {tick}",
            case.name()
        );
        rapier.advance_tick().unwrap();
        engine.advance_tick().unwrap();
        let (rapier_now, engine_now) = (bodies(&rapier), bodies(&engine));
        for (backend, game, before, now) in [
            ("rapier", &rapier, &rapier_before, &rapier_now),
            ("engine", &engine, &engine_before, &engine_now),
        ] {
            for violation in invariants(game, before, now) {
                comparison
                    .violations
                    .entry(backend)
                    .or_default()
                    .push(format!("tick {tick}: {violation}"));
            }
        }
        if comparison.first_contact.is_none()
            && [&rapier_now, &engine_now].iter().any(|now| touching(now))
        {
            comparison.first_contact = Some(tick);
        }
        for (key, body) in &rapier_now {
            if let Some(other) = engine_now.get(key) {
                let difference = (body.position - other.position).length();
                if on_an_edge(&rapier, body) || on_an_edge(&engine, other) {
                    comparison.max_position_on_edge =
                        comparison.max_position_on_edge.max(difference);
                } else if comparison.first_contact.is_some() {
                    comparison.max_position_after_contact =
                        comparison.max_position_after_contact.max(difference);
                } else if difference > comparison.max_position {
                    comparison.max_position = difference;
                    comparison.max_position_at = Some((tick, *key));
                }
            }
        }
        for game in [&rapier, &engine] {
            doors_block(game, tick, &mut comparison);
            if game
                .players
                .values()
                .any(|state| state.region != crate::RegionMembership::Region { id: 1 })
            {
                comparison.raised_region_ticks += 1;
            }
            for state in game.players.values() {
                let head = state.position.y + PLAYER_HALF_EXTENTS.y as f32;
                let touches = game.static_colliders.iter().any(|collider| {
                    let [x, y, z] = collider.position.map(|value| value as f32);
                    let [hx, hy, hz] = collider.half_extents.map(|value| value as f32);
                    collider.kind == crate::StaticColliderKind::Ceiling
                        && (state.position.x - x).abs() < hx
                        && (state.position.z - z).abs() < hz
                        && (y - hy - head).abs() <= 1.0
                });
                if touches && !state.grounded {
                    comparison.ceiling_touches += 1;
                }
                let feet = state.position.y - (PLAYER_HALF_EXTENTS.y + FLOOR_FOOT_Y) as f32;
                comparison.max_height = comparison.max_height.max(feet);
            }
        }
        if comparison.first_outcome_difference.is_none() {
            let (a, b) = (outcomes(&rapier), outcomes(&engine));
            if a != b {
                comparison.first_outcome_difference = Some((tick, a, b));
            }
        }
        let geometry = |game: &ArpgGame| game.physics.fixed_boxes().collect::<Vec<_>>();
        if geometry(&rapier) == geometry(&engine) {
            compare_queries(tick, &rapier, &engine, &rapier_now, &mut comparison);
        } else {
            comparison.geometry_differs += 1;
        }
        (rapier_before, engine_before) = (rapier_now, engine_now);
    }
    comparison
}

/// Bodies rounding a platform edge agree to this (world units): when the contact with the edge
/// nudges Rapier's body up by its 0.01 unit normal nudge, the move counts as rising and
/// Rapier's controller skips that tick's snap to the ground, so the body stays up to the
/// rounded edge's drop (about a unit) higher for a tick; the engine follows the edge.
const EDGE_POSITION: f32 = 2.0;

/// Bodies agree to this (world units) until two characters first touch: Rapier's controller
/// adds a 0.01 unit normal nudge per contact (a body pressed into a corner drifts up to that
/// 0.25 unit offset away) and pushes a body placed into a wall out to touching, where the
/// engine leaves the 0.25 unit offset; `f32` rounding is far smaller.
const POSITION: f32 = 0.5;

/// The merge-blocking A/B acceptance (physics-engine#330): every seeded workload through the
/// real game on Rapier and on physics-engine, with the same fixture, commands and ticks.
///
/// On both backends: finite state, no floor, wall or door penetration and no tunnelling,
/// planar monsters, grounded players on the ground, characters never overlapping, locked doors
/// blocking, and bit-identical same-backend replay. Across them: identical game-visible
/// outcomes every tick (doors, encounters, health, progression, support, strikes,
/// interactions), identical sight, strike, ray, ground and placement decisions from the same
/// inputs, and positions within [`POSITION`] until two characters first touch. After that,
/// sliding around and sidestepping other characters intentionally differ; the difference is
/// reported, not bounded.
#[test]
fn seeded_workloads_agree_on_rapier_and_physics_engine() {
    let mut report = String::new();
    let mut failures = Vec::new();
    for seed in SEEDS {
        for case in Case::ALL {
            let context = format!("{} seed {seed}", case.name());
            let comparison = compare(case, seed);
            writeln!(
                report,
                "{context}: first character contact {:?}; highest feet {:.1}; max position difference \
                 {:.3} before it (at {:?}), {:.3} after, {:.3} on edges; {} queries compared",
                comparison.first_contact,
                comparison.max_height,
                comparison.max_position,
                comparison.max_position_at,
                comparison.max_position_after_contact,
                comparison.max_position_on_edge,
                comparison.queries,
            )
            .unwrap();
            if comparison.max_position > POSITION {
                failures.push(format!(
                    "{context}: positions differ by {} at {:?}",
                    comparison.max_position, comparison.max_position_at
                ));
            }
            if comparison.max_position_on_edge > EDGE_POSITION {
                failures.push(format!(
                    "{context}: positions on an edge differ by {}",
                    comparison.max_position_on_edge
                ));
            }
            if comparison.geometry_differs > 0 {
                failures.push(format!(
                    "{context}: static geometry differs in {} ticks",
                    comparison.geometry_differs
                ));
            }
            if let Some((tick, rapier, engine)) = &comparison.first_outcome_difference {
                failures.push(format!(
                    "{context}: outcomes differ at tick {tick}\n  rapier {rapier}\n  engine {engine}"
                ));
            }
            for difference in comparison.query_differences.iter().take(5) {
                failures.push(format!("{context}: {difference}"));
            }
            for (backend, violations) in &comparison.violations {
                for violation in violations.iter().take(5) {
                    failures.push(format!("{context} on {backend}: {violation}"));
                }
            }
            if case == Case::Ceiling && comparison.ceiling_touches == 0 {
                failures.push(format!("{context}: no jump reached the ceiling"));
            }
            if case == Case::Ledge && comparison.raised_region_ticks == 0 {
                failures.push(format!("{context}: the player never reached the platform"));
            }
            if case == Case::Jump && comparison.max_height < 50.0 {
                failures.push(format!("{context}: the player never jumped"));
            }
            for backend in Backend::ALL {
                if replay(backend, case, seed) != replay(backend, case, seed) {
                    failures.push(format!("{context}: {} replay differs", backend.name()));
                }
            }
        }
    }
    println!("{report}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A save taken mid-play continues exactly as the uninterrupted game on both backends: each
/// rebuilds its world from the seed and the saved game state (#175).
#[test]
fn saves_continue_exactly_on_both_backends() {
    for backend in Backend::ALL {
        with_backend(backend, || {
            for case in Case::SAVED {
                let mut game = fixture(case, 42);
                for tick in 0..TICKS {
                    if tick == 25 || tick == 60 {
                        let mut restored =
                            ArpgGame::from_save_state(game.save_state().unwrap()).unwrap();
                        let mut original = fixture(case, 42);
                        for replayed in 0..tick {
                            commands(&mut original, case, replayed);
                            original.advance_tick().unwrap();
                        }
                        for later in tick..TICKS {
                            commands(&mut original, case, later);
                            commands(&mut restored, case, later);
                            original.advance_tick().unwrap();
                            restored.advance_tick().unwrap();
                            assert_eq!(
                                restored.save_state().unwrap(),
                                original.save_state().unwrap(),
                                "{} {} saved at {tick}, tick {later}",
                                backend.name(),
                                case.name()
                            );
                        }
                    }
                    commands(&mut game, case, tick);
                    game.advance_tick().unwrap();
                }
            }
        });
    }
}
