//! The authoritative physical world, resolved by Rapier (Atlas 2.3.1, 2.3.2).
//!
//! `arpg-core` decides movement intent, acceleration, jumps, gravity and every gameplay
//! outcome; this module asks the physics backend where that intent physically leads. Level
//! geometry (walls, pillars, locked doors) is static boxes on a flat floor, an upward
//! half-space (#59). Players and monsters are kinematic upright capsules moved by a character
//! controller, which resolves contact, sliding, corners and, for players, support: whether the
//! body stands on the ground.
//!
//! The game speaks ARPG world units (`WORLD_UNITS_PER_METER` per metre) in `f32`; the backend
//! simulates in metres, the scale its default tolerances and geometric epsilons assume (in
//! centimetres, parry's GJK contact normals against long walls were off by degrees). This
//! module converts at its boundary. The game keeps each character's `f32` position as
//! authoritative state and mirrors it here; nothing in this world is saved. A loaded save
//! rebuilds it from the seed and the saved game state.
//!
//! The backend is [`rapier`] in every product and WASM build. The test/bench-only `physics-ab`
//! feature (physics-engine#330) also compiles [`engine`], a thin adapter over
//! `physics_engine::approximate::World`, and makes [`Physics`] an enum over both, chosen per
//! thread by [`with_backend`] when a world is created. Call sites never see the difference.

use rapier3d::math::Vector;
use rapier3d::prelude::{Cuboid, Pose, Ray, RayCast};

mod queries;
mod rapier;

#[cfg(feature = "physics-ab")]
mod engine;

use crate::{GameError, Vec3i};

/// Metres per world unit.
const METRES_PER_UNIT: f32 = 1.0 / crate::WORLD_UNITS_PER_METER as f32;
/// The gap (in metres) the character controller keeps between a character and what it
/// touches: a quarter world unit stays well above `f32` resolution across the dungeon and below
/// the half unit that snapshots round away. Gameplay rules see it (#182): touching bodies are
/// a quarter unit apart.
const CONTACT_OFFSET: f32 = CONTACT_OFFSET_UNITS * METRES_PER_UNIT;
/// [`CONTACT_OFFSET`] in world units.
const CONTACT_OFFSET_UNITS: f32 = 0.25;
/// Overlap (in metres) below this depth counts as touching, not penetrating.
const PENETRATION_TOLERANCE: f32 = 1.0e-3 * METRES_PER_UNIT;
/// How far (in metres) below a body Rapier's controller still finds the ground: its contact
/// offset plus the fixed 5 cm its grounded test predicts. Support queries and snapping to
/// the ground use the same reach, so a body that the controller calls grounded settles onto
/// the ground in the next tick instead of hovering above it.
const SUPPORT_REACH: f32 = CONTACT_OFFSET + 0.05;

/// A game-chosen identity for a physical body (walls, doors, players and monsters each
/// take their own id range).
pub(crate) type BodyKey = u64;

/// Where the character controller moved a player in one tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Motion {
    /// The body's new position, in world units.
    pub to: Vector,
    /// The translation the controller applied, in world units.
    pub moved: Vector,
    /// Whether the body ends supported: the controller reports it grounded on static ground
    /// within its support reach below. Another character's body is not ground.
    pub supported: bool,
}

/// `value` in world units as the game's vector type, still in world units.
pub(crate) fn vector(value: Vec3i) -> Vector {
    Vector::new(value.x as f32, value.y as f32, value.z as f32)
}

/// A world-unit position or extent in the backend's metres.
fn metres(value: Vector) -> Vector {
    value * METRES_PER_UNIT
}

/// A backend length or position (metres) in world units.
fn units(value: Vector) -> Vector {
    value * crate::WORLD_UNITS_PER_METER as f32
}

/// The radius and half segment height (world units) of the upright capsule as wide and tall as
/// `half_extents`.
pub(crate) fn capsule_dimensions(half_extents: Vec3i) -> (f32, f32) {
    let radius = half_extents.x.max(half_extents.z) as f32;
    let half_height = (half_extents.y as f32 - radius).max(0.0);
    (radius, half_height)
}

/// The earliest time (in units of `translation`, within `0..=1`) at which the segment from
/// `origin` along `translation` meets a box hurt volume centred on `center`. A closed-form
/// shape query on no world: both backends share it.
pub(crate) fn box_hit(
    center: Vector,
    half_extents: Vec3i,
    origin: Vector,
    translation: Vector,
) -> Option<f32> {
    Cuboid::new(metres(vector(half_extents))).cast_ray(
        &Pose::from_translation(metres(center)),
        &Ray::new(metres(origin), metres(translation)),
        1.0,
        true,
    )
}

fn missing(key: BodyKey) -> GameError {
    GameError::new(format!("physics body {key} is missing"))
}

#[cfg(not(feature = "physics-ab"))]
pub(crate) use rapier::Physics;

#[cfg(feature = "physics-ab")]
pub(crate) use ab::Physics;
#[cfg(feature = "physics-ab")]
pub use ab::{Backend, take_physics_time, take_rebuild_time, time_physics, with_backend};

/// The A/B seam (physics-engine#330): one enum over both backends, compiled only with the
/// test/bench-only `physics-ab` feature.
#[cfg(feature = "physics-ab")]
mod ab {
    use std::cell::Cell;
    use std::fmt;
    use std::time::{Duration, Instant};

    use rapier3d::math::Vector;

    use super::{BodyKey, Motion, engine, rapier};
    use crate::{GameError, Vec3i};

    /// Which physics backend a new [`Physics`] world uses.
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub enum Backend {
        /// Rapier, the production authority (Atlas 2.3.1).
        #[default]
        Rapier,
        /// physics-engine's `approximate::World`, through [`engine`]'s thin adapter.
        Engine,
    }

    impl Backend {
        pub const ALL: [Self; 2] = [Self::Rapier, Self::Engine];

        pub fn name(self) -> &'static str {
            match self {
                Self::Rapier => "rapier",
                Self::Engine => "physics-engine",
            }
        }
    }

    thread_local! {
        static BACKEND: Cell<Backend> = const { Cell::new(Backend::Rapier) };
        /// Whether physics calls add their wall-clock time to [`ELAPSED`].
        static TIMING: Cell<bool> = const { Cell::new(false) };
        static ELAPSED: Cell<Duration> = const { Cell::new(Duration::ZERO) };
        /// The part of [`ELAPSED`] spent in [`Physics::rebuild`].
        static REBUILD: Cell<Duration> = const { Cell::new(Duration::ZERO) };
    }

    /// Runs `run` with every physics world it creates on this thread (games built, restored or
    /// rebuilt from saves) on `backend`.
    pub fn with_backend<R>(backend: Backend, run: impl FnOnce() -> R) -> R {
        let previous = BACKEND.replace(backend);
        let result = run();
        BACKEND.set(previous);
        result
    }

    /// Starts or stops timing every call into the physics module on this thread: the whole
    /// physics path of the game, adapter conversions and bookkeeping included.
    pub fn time_physics(on: bool) {
        TIMING.set(on);
    }

    /// The physics time accumulated on this thread since the last call.
    pub fn take_physics_time() -> Duration {
        ELAPSED.replace(Duration::ZERO)
    }

    /// The part of the physics time spent rebuilding worlds since the last call.
    pub fn take_rebuild_time() -> Duration {
        REBUILD.replace(Duration::ZERO)
    }

    /// Adds the time until it drops to [`ELAPSED`] while timing is on.
    struct Timer(Option<Instant>);

    impl Timer {
        fn start() -> Self {
            Self(TIMING.get().then(Instant::now))
        }

        /// Stops the timer; the time also counts as rebuilding.
        fn rebuilt(mut self) {
            if let Some(start) = self.0.take() {
                let elapsed = start.elapsed();
                ELAPSED.set(ELAPSED.get() + elapsed);
                REBUILD.set(REBUILD.get() + elapsed);
            }
        }
    }

    impl Drop for Timer {
        fn drop(&mut self) {
            if let Some(start) = self.0 {
                let elapsed = start.elapsed();
                ELAPSED.set(ELAPSED.get() + elapsed);
            }
        }
    }

    // One world per game, built once and never moved per tick: boxing would only add an
    // indirection to every call of either backend.
    #[allow(clippy::large_enum_variant)]
    pub(crate) enum Physics {
        Rapier(rapier::Physics),
        Engine(engine::Physics),
    }

    impl fmt::Debug for Physics {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Rapier(physics) => physics.fmt(formatter),
                Self::Engine(physics) => physics.fmt(formatter),
            }
        }
    }

    /// Calls the same method on whichever backend this world uses, timed.
    macro_rules! dispatch {
        ($self:expr, $physics:ident => $call:expr) => {{
            let _timer = Timer::start();
            match $self {
                Physics::Rapier($physics) => $call,
                Physics::Engine($physics) => $call,
            }
        }};
    }

    impl Physics {
        pub(crate) fn new() -> Self {
            let _timer = Timer::start();
            match BACKEND.get() {
                Backend::Rapier => Self::Rapier(rapier::Physics::new()),
                Backend::Engine => Self::Engine(engine::Physics::new()),
            }
        }

        pub(crate) fn add_floor(&mut self, foot_height: f32) -> Result<(), GameError> {
            dispatch!(self, physics => physics.add_floor(foot_height))
        }

        pub(crate) fn add_fixed(
            &mut self,
            key: BodyKey,
            center: Vec3i,
            half_extents: Vec3i,
        ) -> Result<(), GameError> {
            dispatch!(self, physics => physics.add_fixed(key, center, half_extents))
        }

        pub(crate) fn remove_fixed(&mut self, key: BodyKey) -> bool {
            dispatch!(self, physics => physics.remove_fixed(key))
        }

        #[cfg(test)]
        pub(crate) fn has_fixed(&self, key: BodyKey) -> bool {
            dispatch!(self, physics => physics.has_fixed(key))
        }

        pub(crate) fn fixed_boxes(&self) -> impl Iterator<Item = (Vec3i, Vec3i)> + '_ {
            dispatch!(self, physics => physics.fixed_boxes().collect::<Vec<_>>()).into_iter()
        }

        pub(crate) fn add_character(
            &mut self,
            key: BodyKey,
            position: Vector,
            half_extents: Vec3i,
        ) -> Result<(), GameError> {
            dispatch!(self, physics => physics.add_character(key, position, half_extents))
        }

        pub(crate) fn remove_character(&mut self, key: BodyKey) -> bool {
            dispatch!(self, physics => physics.remove_character(key))
        }

        pub(crate) fn has_character(&self, key: BodyKey) -> bool {
            dispatch!(self, physics => physics.has_character(key))
        }

        pub(crate) fn character_position(&self, key: BodyKey) -> Option<Vector> {
            dispatch!(self, physics => physics.character_position(key))
        }

        pub(crate) fn place_character(
            &mut self,
            key: BodyKey,
            position: Vector,
        ) -> Result<(), GameError> {
            dispatch!(self, physics => physics.place_character(key, position))
        }

        pub(crate) fn rebuild(&mut self) {
            let timer = Timer::start();
            match self {
                Self::Rapier(physics) => physics.rebuild(),
                Self::Engine(physics) => physics.rebuild(),
            }
            timer.rebuilt();
        }

        pub(crate) fn move_character(
            &mut self,
            key: BodyKey,
            from: Vector,
            desired: Vector,
        ) -> Result<(Vector, Vector), GameError> {
            dispatch!(self, physics => physics.move_character(key, from, desired))
        }

        pub(crate) fn move_player(
            &mut self,
            key: BodyKey,
            from: Vector,
            desired: Vector,
        ) -> Result<Motion, GameError> {
            dispatch!(self, physics => physics.move_player(key, from, desired))
        }

        pub(crate) fn ground_height(&self, position: Vector) -> Option<f32> {
            dispatch!(self, physics => physics.ground_height(position))
        }

        pub(crate) fn segment_obstructed(&self, from: Vector, to: Vector) -> bool {
            dispatch!(self, physics => physics.segment_obstructed(from, to))
        }

        pub(crate) fn first_fixed_hit(&self, origin: Vector, translation: Vector) -> Option<f32> {
            dispatch!(self, physics => physics.first_fixed_hit(origin, translation))
        }

        pub(crate) fn penetrates_fixed(&self, position: Vector, half_extents: Vec3i) -> bool {
            dispatch!(self, physics => physics.penetrates_fixed(position, half_extents))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: Vec3i = Vec3i::new(30, 50, 30);
    const Y: i32 = 50;
    /// How far a character pressed against something stands off it: both backends keep a 0.25
    /// unit contact offset (Rapier's controller adds its 0.01 unit normal nudge), never
    /// overlapping.
    const GAP: std::ops::RangeInclusive<f32> = 0.2..=0.3;

    /// Runs a contract test on Rapier and, with `physics-ab`, on physics-engine as well.
    fn each_backend(test: impl Fn()) {
        #[cfg(feature = "physics-ab")]
        for backend in Backend::ALL {
            eprintln!("physics contract on {backend:?}");
            with_backend(backend, &test);
        }
        #[cfg(not(feature = "physics-ab"))]
        test();
    }

    fn at(x: i32, z: i32) -> Vector {
        vector(Vec3i::new(x, Y, z))
    }

    /// A character at `from` moved `ticks` times by `step`; its positions after each tick.
    fn walk(
        physics: &mut Physics,
        key: BodyKey,
        from: Vector,
        step: Vector,
        ticks: usize,
    ) -> Vec<Vector> {
        let mut position = from;
        (0..ticks)
            .map(|_| {
                (position, _) = physics.move_character(key, position, step).unwrap();
                position
            })
            .collect()
    }

    #[test]
    fn a_fast_character_stops_at_a_thin_wall_instead_of_tunnelling() {
        each_backend(|| {
            let mut physics = Physics::new();
            // A wall 10 units thick whose near face is at x = 195.
            physics
                .add_fixed(1, Vec3i::new(200, Y, 0), Vec3i::new(5, 100, 500))
                .unwrap();
            physics.add_character(2, at(0, 0), BODY).unwrap();
            let (to, moved) = physics
                .move_character(2, at(0, 0), Vector::new(1_000.0, 0.0, 0.0))
                .unwrap();
            assert!(GAP.contains(&(165.0 - to.x)), "{to:?}");
            assert_eq!(to.y, Y as f32);
            assert!(to.z.abs() < 1.0e-2, "{to:?}");
            assert!((moved - (to - at(0, 0))).length() < 1.0e-3);
        });
    }

    #[test]
    fn sliding_along_a_wall_keeps_the_tangential_motion() {
        each_backend(|| {
            let mut physics = Physics::new();
            // A 60 m wall, like the dungeon's outer ones; its face is at x = 100.
            physics
                .add_fixed(1, Vec3i::new(125, Y, 0), Vec3i::new(25, 100, 3_000))
                .unwrap();
            let start = at(60, -1_000);
            physics.add_character(2, start, BODY).unwrap();
            let path = walk(&mut physics, 2, start, Vector::new(5.0, 0.0, 5.0), 200);
            for (tick, position) in path.iter().enumerate().skip(10) {
                assert!(
                    GAP.contains(&(70.0 - position.x)),
                    "tick {tick}: {position:?}"
                );
                let advanced = position.z - path[tick - 1].z;
                assert!(
                    (advanced - 5.0).abs() < 1.0e-3,
                    "tick {tick}: advanced {advanced}"
                );
            }
            let progress = (path[199].z - path[9].z) / (190.0 * 5.0);
            assert!((progress - 1.0).abs() < 1.0e-4, "progress {progress}");
        });
    }

    #[test]
    fn an_inner_corner_holds_a_character_without_sticking_it() {
        each_backend(|| {
            let mut physics = Physics::new();
            // Walls meeting at the corner (100, 100): faces x = 100 and z = 100.
            physics
                .add_fixed(1, Vec3i::new(125, Y, 0), Vec3i::new(25, 100, 125))
                .unwrap();
            physics
                .add_fixed(2, Vec3i::new(0, Y, 125), Vec3i::new(125, 100, 25))
                .unwrap();
            physics.add_character(3, at(0, 0), BODY).unwrap();
            let pressed = *walk(&mut physics, 3, at(0, 0), Vector::new(7.0, 0.0, 7.0), 60)
                .last()
                .unwrap();
            assert!(GAP.contains(&(70.0 - pressed.x)), "{pressed:?}");
            assert!(GAP.contains(&(70.0 - pressed.z)), "{pressed:?}");
            // Leaving along either wall moves at full speed from the first tick.
            for step in [Vector::new(-7.0, 0.0, 0.0), Vector::new(0.0, 0.0, -7.0)] {
                let mut free = Physics::new();
                free.add_fixed(1, Vec3i::new(125, Y, 0), Vec3i::new(25, 100, 125))
                    .unwrap();
                free.add_fixed(2, Vec3i::new(0, Y, 125), Vec3i::new(125, 100, 25))
                    .unwrap();
                free.add_character(3, pressed, BODY).unwrap();
                let (_, moved) = free.move_character(3, pressed, step).unwrap();
                assert!((moved - step).length() < 1.0e-3, "{moved:?}");
            }
        });
    }

    #[test]
    fn characters_block_each_other_from_every_side_without_overlapping() {
        each_backend(|| {
            for degrees in (0..360).step_by(15) {
                let angle = (degrees as f32).to_radians();
                let (dx, dz) = (angle.cos(), angle.sin());
                let mut physics = Physics::new();
                physics.add_character(1, at(0, 0), BODY).unwrap();
                let start = Vector::new(-200.0 * dx, Y as f32, -200.0 * dz);
                physics.add_character(2, start, BODY).unwrap();
                // Aimed slightly off the other's centre, so it glances and slides around it.
                let step = Vector::new(7.0 * dx - 1.0 * dz, 0.0, 7.0 * dz + 1.0 * dx);
                for (tick, position) in walk(&mut physics, 2, start, step, 80).iter().enumerate() {
                    let distance = Vector::new(position.x, 0.0, position.z).length();
                    assert!(
                        distance >= 60.0 - 1.0e-2,
                        "{degrees}° tick {tick}: {distance}"
                    );
                }
            }
        });
    }

    #[test]
    fn a_removed_door_collider_no_longer_blocks() {
        each_backend(|| {
            let mut physics = Physics::new();
            physics
                .add_fixed(9, Vec3i::new(200, Y, 0), Vec3i::new(25, 100, 90))
                .unwrap();
            physics.add_character(1, at(0, 0), BODY).unwrap();
            let blocked = *walk(&mut physics, 1, at(0, 0), Vector::new(7.0, 0.0, 0.0), 60)
                .last()
                .unwrap();
            assert!(GAP.contains(&(145.0 - blocked.x)), "{blocked:?}");
            assert!(physics.remove_fixed(9));
            let (_, moved) = physics
                .move_character(1, blocked, Vector::new(7.0, 0.0, 0.0))
                .unwrap();
            // Within `f32` rounding: physics-engine reports the translation as a difference of
            // metre positions.
            assert!(
                (moved - Vector::new(7.0, 0.0, 0.0)).length() < 1.0e-3,
                "{moved:?}"
            );
        });
    }

    #[test]
    fn segments_are_obstructed_only_by_static_geometry_before_their_end() {
        each_backend(|| {
            let mut physics = Physics::new();
            physics
                .add_fixed(1, Vec3i::new(100, Y, 0), Vec3i::new(10, 100, 50))
                .unwrap();
            physics.add_character(2, at(50, 0), BODY).unwrap();
            assert!(physics.segment_obstructed(at(0, 0), at(200, 0)));
            // Ending at the face is not obstructed; characters never obstruct.
            assert!(!physics.segment_obstructed(at(0, 0), at(90, 0)));
            assert!(!physics.segment_obstructed(at(0, 0), at(0, 200)));
            let time = physics.first_fixed_hit(at(0, 0), Vector::new(180.0, 0.0, 0.0));
            assert!(
                time.is_some_and(|time| (time - 0.5).abs() < 1.0e-4),
                "{time:?}"
            );
            assert!(physics.penetrates_fixed(at(100, 0), BODY));
            assert!(!physics.penetrates_fixed(at(60, 0), BODY));
        });
    }

    /// A world with the floor laid for bodies resting at `Y - 50` (feet on the plane).
    fn floored() -> Physics {
        let mut physics = Physics::new();
        physics.add_floor((Y - BODY.y) as f32).unwrap();
        physics
    }

    /// A player body moved `ticks` times: each tick's desired motion from `step(tick, last
    /// motion)`; every motion in order.
    fn fly(
        physics: &mut Physics,
        key: BodyKey,
        from: Vector,
        ticks: usize,
        mut step: impl FnMut(usize, Option<Motion>) -> Vector,
    ) -> Vec<Motion> {
        let mut position = from;
        let mut last = None;
        (0..ticks)
            .map(|tick| {
                let motion = physics
                    .move_player(key, position, step(tick, last))
                    .unwrap();
                position = motion.to;
                last = Some(motion);
                motion
            })
            .collect()
    }

    #[test]
    fn a_body_on_the_floor_walks_without_moving_vertically_and_stays_supported() {
        each_backend(|| {
            let mut physics = floored();
            physics.add_character(1, at(0, 0), BODY).unwrap();
            for motion in fly(&mut physics, 1, at(0, 0), 60, |_, _| {
                Vector::new(5.0, 0.0, -5.0)
            }) {
                assert_eq!(motion.to.y, Y as f32, "{motion:?}");
                assert!((motion.moved - Vector::new(5.0, 0.0, -5.0)).length() < 1.0e-3);
                assert_eq!(motion.moved.y, 0.0);
                assert!(motion.supported);
            }
            // Standing still is supported as well.
            let rest = physics.move_player(1, at(300, -300), Vector::ZERO).unwrap();
            assert_eq!(rest.to, at(300, -300));
            assert!(rest.supported);
        });
    }

    #[test]
    fn a_rising_body_leaves_the_ground_and_lands_back_on_it() {
        each_backend(|| {
            let mut physics = floored();
            physics.add_character(1, at(0, 0), BODY).unwrap();
            let gravity = 0.5;
            let mut velocity: f32 = 10.0;
            let path = fly(&mut physics, 1, at(0, 0), 80, |tick, last| {
                if tick > 0 {
                    velocity = if last.is_some_and(|motion| motion.supported) {
                        0.0
                    } else {
                        velocity - gravity
                    };
                }
                Vector::new(3.0, velocity, 0.0)
            });
            assert!(!path[0].supported);
            assert!((path[0].moved - Vector::new(3.0, 10.0, 0.0)).length() < 1.0e-3);
            let apex = path
                .iter()
                .map(|motion| motion.to.y)
                .fold(f32::MIN, f32::max);
            // 10 + 9.5 + ... + 0.5: 105 units above the plane.
            assert!((apex - (Y as f32 + 105.0)).abs() < 1.0e-3, "{apex}");
            let landed = path.iter().position(|motion| motion.supported).unwrap();
            assert!((38..=42).contains(&landed), "{landed}");
            // Settled on the floor a tick later, and it stays there.
            for motion in &path[landed + 1..] {
                assert!((motion.to.y - Y as f32).abs() < 1.0e-3, "{motion:?}");
                assert!(motion.supported);
            }
            // The horizontal motion carried on through the landing.
            assert!((path[79].to.x - 240.0).abs() < 1.0e-2, "{:?}", path[79]);
        });
    }

    #[test]
    fn an_airborne_body_slides_along_a_wall_as_precisely_as_on_the_ground() {
        each_backend(|| {
            let mut physics = floored();
            // A 60 m wall reaching 1.5 m above the plane's feet; its face is at x = 100.
            physics
                .add_fixed(1, Vec3i::new(125, Y, 0), Vec3i::new(25, 100, 3_000))
                .unwrap();
            let start = at(60, -1_000);
            physics.add_character(2, start, BODY).unwrap();
            let mut velocity: f32 = 9.0;
            let path = fly(&mut physics, 2, start, 40, |tick, _| {
                if tick > 0 {
                    velocity -= 0.5;
                }
                Vector::new(5.0, velocity.max(0.0), 5.0)
            });
            for (tick, motion) in path.iter().enumerate().skip(10) {
                assert!(
                    GAP.contains(&(70.0 - motion.to.x)),
                    "tick {tick}: {motion:?}"
                );
                let advanced = motion.to.z - path[tick - 1].to.z;
                assert!((advanced - 5.0).abs() < 1.0e-3, "tick {tick}: {advanced}");
            }
            assert!(path[39].to.y > Y as f32 + 80.0, "{:?}", path[39]);
        });
    }

    #[test]
    fn a_body_falling_beside_another_keeps_its_distance_and_lands() {
        each_backend(|| {
            let mut physics = floored();
            physics.add_character(1, at(0, 0), BODY).unwrap();
            // Its axis overlaps the other's in height: pressing towards it while falling.
            let start = Vector::new(-61.0, Y as f32 + 30.0, 0.0);
            physics.add_character(2, start, BODY).unwrap();
            let path = fly(&mut physics, 2, start, 30, |_, last| {
                if last.is_some_and(|motion| motion.supported) {
                    Vector::new(3.0, 0.0, 0.0)
                } else {
                    Vector::new(3.0, -3.0, 0.0)
                }
            });
            for motion in &path {
                let planar = Vector::new(motion.to.x, 0.0, motion.to.z).length();
                assert!(planar >= 60.0 - 1.0e-2, "{motion:?}");
            }
            assert!(path[29].supported);
            assert!((path[29].to.y - Y as f32).abs() < 1.0e-3, "{:?}", path[29]);
        });
    }

    #[test]
    fn standing_on_another_body_is_not_support() {
        each_backend(|| {
            let mut physics = floored();
            physics.add_character(1, at(0, 0), BODY).unwrap();
            // Directly above the other body, its lowest point touching the other's top.
            let above = Vector::new(0.0, Y as f32 + 100.3, 0.0);
            physics.add_character(2, above, BODY).unwrap();
            let motion = physics
                .move_player(2, above, Vector::new(0.0, -1.0, 0.0))
                .unwrap();
            assert!(!motion.supported, "{motion:?}");
        });
    }

    #[test]
    fn the_floor_never_obstructs_segments_and_carries_the_ground_height() {
        each_backend(|| {
            let physics = floored();
            assert!(!physics.segment_obstructed(at(0, 0), Vector::new(0.0, -200.0, 500.0)));
            assert!(
                physics
                    .first_fixed_hit(at(0, 0), Vector::new(0.0, -500.0, 0.0))
                    .is_none()
            );
            let ground = physics
                .ground_height(Vector::new(10.0, 400.0, 10.0))
                .unwrap();
            assert!(ground.abs() < 1.0e-3, "{ground}");
            // Bodies below the floor penetrate it; resting on it does not.
            assert!(!physics.penetrates_fixed(at(0, 0), BODY));
            assert!(physics.penetrates_fixed(at(0, 0) - Vector::new(0.0, 5.0, 0.0), BODY));
        });
    }
}
