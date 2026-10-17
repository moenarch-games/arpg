//! The authoritative physical world, resolved by Rapier (Atlas 2.3.1, 2.3.2).
//!
//! `arpg-core` decides movement intent, acceleration, jumps, gravity and every gameplay
//! outcome; this module asks Rapier where that intent physically leads. Level geometry
//! (walls, pillars, locked doors) is static cuboid colliders on a flat floor, an upward
//! half-space (#59). Players and monsters are kinematic position-based bodies with capsule
//! colliders, moved by Rapier's `KinematicCharacterController`, which resolves contact,
//! sliding, corners and, for players, support: whether the body stands on the ground.
//!
//! The game speaks ARPG world units (`WORLD_UNITS_PER_METER` per metre) in `f32`; Rapier
//! simulates in metres, the scale its default tolerances and parry's geometric epsilons
//! assume (in centimetres, GJK contact normals against long walls were off by degrees). This
//! module converts at its boundary. The game keeps each character's `f32` position as
//! authoritative state and mirrors it here; nothing in this world is saved. A loaded save
//! rebuilds it from the seed and the saved game state.
//!
//! The character controller's contact queries go through [`queries`], exact closed forms for
//! the upright capsules and boxes the game uses, ahead of parry's general GJK/EPA path.

use std::collections::BTreeMap;
use std::fmt;

use rapier3d::control::{CharacterLength, KinematicCharacterController};
use rapier3d::parry::query::{
    DefaultQueryDispatcher, QueryDispatcher, QueryDispatcherChain, RayCast, ShapeCastOptions,
};
use rapier3d::prelude::*;

use queries::UprightShapes;

mod queries;

use crate::{GameError, TICK_HZ, Vec3i};

/// Metres per world unit.
const METRES_PER_UNIT: f32 = 1.0 / crate::WORLD_UNITS_PER_METER as f32;
/// The gap (in metres) Rapier's controller keeps between a character and what it touches:
/// a quarter world unit stays well above `f32` resolution across the dungeon and below the
/// half unit that snapshots round away. Gameplay rules see it (#182): touching bodies are
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

/// One static box of the game and its collider.
struct Fixed {
    center: Vec3i,
    half_extents: Vec3i,
    collider: ColliderHandle,
}

#[derive(Clone, Copy)]
struct Character {
    body: RigidBodyHandle,
    shape: Capsule,
    /// The game's position of the body, in world units.
    position: Vector,
}

/// The flat floor: an upward half-space whose surface lies one contact offset below
/// `foot_height`, so a body Rapier's controller rests on it has its lowest point exactly at
/// `foot_height` (world units).
#[derive(Clone, Copy)]
struct Floor {
    foot_height: f32,
    collider: ColliderHandle,
}

/// Where Rapier's controller moved a player in one tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Motion {
    /// The body's new position, in world units.
    pub to: Vector,
    /// The translation the controller applied, in world units.
    pub moved: Vector,
    /// Whether the body ends supported: Rapier's controller reports it grounded and static
    /// ground lies within its support reach below. Another character's body is not ground.
    pub supported: bool,
}

pub(crate) struct Physics {
    world: PhysicsWorld,
    queries: QueryDispatcherChain<UprightShapes, DefaultQueryDispatcher>,
    /// Planar motion (monsters): no vertical intent, no snapping.
    controller: KinematicCharacterController,
    /// Players' motion with height: the same contact rules, snapping to the ground the
    /// controller found the body grounded on.
    vertical: KinematicCharacterController,
    floor: Option<Floor>,
    fixed: BTreeMap<BodyKey, Fixed>,
    characters: BTreeMap<BodyKey, Character>,
}

impl fmt::Debug for Physics {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Physics")
            .field("fixed", &self.fixed.keys().collect::<Vec<_>>())
            .field("characters", &self.characters.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// `value` in world units as Rapier's vector type, still in world units.
pub(crate) fn vector(value: Vec3i) -> Vector {
    Vector::new(value.x as f32, value.y as f32, value.z as f32)
}

/// A world-unit position or extent in Rapier's metres.
fn metres(value: Vector) -> Vector {
    value * METRES_PER_UNIT
}

/// A Rapier length or position in world units.
fn units(value: Vector) -> Vector {
    value * crate::WORLD_UNITS_PER_METER as f32
}

/// An empty Rapier world.
fn empty_world() -> PhysicsWorld {
    let mut world = PhysicsWorld::new();
    // Gravity and jumps are movement intent the game owns (#59): no Rapier body is dynamic,
    // so nothing falls by itself.
    world.gravity = Vector::ZERO;
    world.integration_parameters.dt = 1.0 / f32::from(TICK_HZ);
    world
}

/// Inserts a static box collider.
fn insert_fixed(world: &mut PhysicsWorld, center: Vec3i, half_extents: Vec3i) -> ColliderHandle {
    let half = metres(vector(half_extents));
    world.colliders.insert(
        ColliderBuilder::cuboid(half.x, half.y, half.z)
            .translation(metres(vector(center)))
            .build(),
    )
}

/// Inserts the floor's upward half-space with its surface at `height` (world units).
fn insert_floor(world: &mut PhysicsWorld, height: f32) -> ColliderHandle {
    world.colliders.insert(
        ColliderBuilder::new(SharedShape::halfspace(Vector::Y))
            .translation(metres(Vector::new(0.0, height, 0.0)))
            .build(),
    )
}

/// Inserts a kinematic body at `position` (in metres) with its capsule collider.
fn insert_character(world: &mut PhysicsWorld, position: Vector, shape: Capsule) -> RigidBodyHandle {
    let body = world.bodies.insert(
        RigidBodyBuilder::kinematic_position_based()
            .translation(position)
            .build(),
    );
    world.colliders.insert_with_parent(
        ColliderBuilder::new(SharedShape::new(shape)).build(),
        body,
        &mut world.bodies,
    );
    body
}

impl Physics {
    pub(crate) fn new() -> Self {
        let controller = KinematicCharacterController {
            up: Vector::Y,
            offset: CharacterLength::Absolute(CONTACT_OFFSET),
            slide: true,
            // No stairs or ledges exist yet (ADR 0002 traversal follows #59's flat slice).
            autostep: None,
            max_slope_climb_angle: std::f32::consts::FRAC_PI_4,
            min_slope_slide_angle: std::f32::consts::FRAC_PI_4,
            snap_to_ground: None,
            normal_nudge_factor: KinematicCharacterController::default().normal_nudge_factor,
        };
        Self {
            world: empty_world(),
            queries: UprightShapes.chain(DefaultQueryDispatcher),
            controller,
            vertical: KinematicCharacterController {
                // Settles a body the controller found grounded within its support reach;
                // never a step down a ledge (no ledges exist yet).
                snap_to_ground: Some(CharacterLength::Absolute(SUPPORT_REACH)),
                ..controller
            },
            floor: None,
            fixed: BTreeMap::new(),
            characters: BTreeMap::new(),
        }
    }

    /// Lays the flat floor so that a body resting on it has its lowest point at
    /// `foot_height` (world units).
    pub(crate) fn add_floor(&mut self, foot_height: f32) -> Result<(), GameError> {
        if self.floor.is_some() {
            return Err(GameError::new("the floor already exists"));
        }
        let collider = insert_floor(&mut self.world, foot_height - CONTACT_OFFSET_UNITS);
        self.floor = Some(Floor {
            foot_height,
            collider,
        });
        self.refresh();
        Ok(())
    }

    /// Adds a static box collider centred on `center` with `half_extents`.
    pub(crate) fn add_fixed(
        &mut self,
        key: BodyKey,
        center: Vec3i,
        half_extents: Vec3i,
    ) -> Result<(), GameError> {
        if self.fixed.contains_key(&key) {
            return Err(GameError::new(format!("physics body {key} already exists")));
        }
        let collider = insert_fixed(&mut self.world, center, half_extents);
        self.fixed.insert(
            key,
            Fixed {
                center,
                half_extents,
                collider,
            },
        );
        self.refresh();
        Ok(())
    }

    /// Removes a static collider; `false` when there was none.
    pub(crate) fn remove_fixed(&mut self, key: BodyKey) -> bool {
        let Some(fixed) = self.fixed.remove(&key) else {
            return false;
        };
        self.world.remove_collider(fixed.collider);
        self.refresh();
        true
    }

    #[cfg(test)]
    pub(crate) fn has_fixed(&self, key: BodyKey) -> bool {
        self.fixed.contains_key(&key)
    }

    /// Centre and half extents of every static box, in key order.
    pub(crate) fn fixed_boxes(&self) -> impl Iterator<Item = (Vec3i, Vec3i)> + '_ {
        self.fixed
            .values()
            .map(|fixed| (fixed.center, fixed.half_extents))
    }

    /// Adds a character at `position`: a kinematic body with an upright capsule as wide and
    /// tall as `half_extents`.
    pub(crate) fn add_character(
        &mut self,
        key: BodyKey,
        position: Vector,
        half_extents: Vec3i,
    ) -> Result<(), GameError> {
        if self.characters.contains_key(&key) {
            return Err(GameError::new(format!("physics body {key} already exists")));
        }
        let shape = character_shape(half_extents);
        let body = insert_character(&mut self.world, metres(position), shape);
        self.characters.insert(
            key,
            Character {
                body,
                shape,
                position,
            },
        );
        self.refresh();
        Ok(())
    }

    /// Removes a character; `false` when there was none.
    pub(crate) fn remove_character(&mut self, key: BodyKey) -> bool {
        let Some(character) = self.characters.remove(&key) else {
            return false;
        };
        self.world.remove_body(character.body);
        self.refresh();
        true
    }

    pub(crate) fn has_character(&self, key: BodyKey) -> bool {
        self.characters.contains_key(&key)
    }

    /// Where the character's body is, if it has one.
    pub(crate) fn character_position(&self, key: BodyKey) -> Option<Vector> {
        Some(self.characters.get(&key)?.position)
    }

    /// Puts the character's body at `position` without resolving contact: placement by the
    /// game (spawns, restores, workbench arrangements), never movement.
    pub(crate) fn place_character(
        &mut self,
        key: BodyKey,
        position: Vector,
    ) -> Result<(), GameError> {
        self.set_position(key, position)
    }

    /// Mirrors the game's `position` of a character in its body.
    fn set_position(&mut self, key: BodyKey, position: Vector) -> Result<(), GameError> {
        let character = self.characters.get_mut(&key).ok_or_else(|| missing(key))?;
        if character.position == position {
            return Ok(());
        }
        character.position = position;
        self.world
            .bodies
            .get_mut(character.body)
            .ok_or_else(|| missing(key))?
            .set_translation(metres(position), false);
        // Later queries (the next character's motion among them) see the body there.
        self.refresh();
        Ok(())
    }

    /// Rebuilds the Rapier world from the bodies it holds: static boxes and then characters
    /// each in key order, then the floor. Rapier's query structure (its broad-phase BVH) depends on the
    /// history of insertions, removals and moves, and the controller breaks exact ties
    /// between equally near contacts (a character squeezed between two others, or reaching
    /// two walls at once) by its traversal order. Rebuilding before each tick's
    /// motion makes the result a function of game state alone, so a restored save continues
    /// exactly as the uninterrupted game. The floor comes last: inserted first, it changed
    /// the tree, and with it how such ties between characters break.
    pub(crate) fn rebuild(&mut self) {
        let mut world = empty_world();
        for fixed in self.fixed.values_mut() {
            fixed.collider = insert_fixed(&mut world, fixed.center, fixed.half_extents);
        }
        for character in self.characters.values_mut() {
            character.body =
                insert_character(&mut world, metres(character.position), character.shape);
        }
        if let Some(floor) = self.floor.as_mut() {
            floor.collider = insert_floor(&mut world, floor.foot_height - CONTACT_OFFSET_UNITS);
        }
        self.world = world;
        self.refresh();
    }

    /// Moves the character from `from` by the game's `desired` translation for one tick
    /// and returns where Rapier's controller lets it end, with the translation it applied:
    /// blocked by fixed geometry and the other characters, sliding along what it touches.
    /// The motion stays on the game's plane (monsters never leave the ground), so the floor
    /// takes no part in it. Moving nowhere still lets the controller push the character out
    /// of anything placed into it.
    pub(crate) fn move_character(
        &mut self,
        key: BodyKey,
        from: Vector,
        desired: Vector,
    ) -> Result<(Vector, Vector), GameError> {
        let character = self.character(key)?;
        let mut filter = QueryFilter::default().exclude_rigid_body(character.body);
        if let Some(floor) = self.floor {
            filter = filter.exclude_collider(floor.collider);
        }
        let queries = self.world.broad_phase.as_query_pipeline(
            &self.queries,
            &self.world.bodies,
            &self.world.colliders,
            filter,
        );
        let movement = self.controller.move_shape(
            self.world.integration_parameters.dt,
            &queries,
            &character.shape,
            &Pose::from_translation(metres(from)),
            metres(Vector::new(desired.x, 0.0, desired.z)),
            |_| {},
        );
        let translation = units(Vector::new(
            movement.translation.x,
            0.0,
            movement.translation.z,
        ));
        let to = from + translation;
        self.set_position(key, to)?;
        Ok((to, translation))
    }

    /// Moves a player from `from` by the game's `desired` translation for one tick, height
    /// included: Rapier's controller blocks and slides it as for [`Self::move_character`],
    /// lands it on the ground, settles it onto ground found within its support reach and
    /// reports whether it ends supported.
    pub(crate) fn move_player(
        &mut self,
        key: BodyKey,
        from: Vector,
        desired: Vector,
    ) -> Result<Motion, GameError> {
        let character = self.character(key)?;
        let filter = QueryFilter::default().exclude_rigid_body(character.body);
        let queries = self.world.broad_phase.as_query_pipeline(
            &self.queries,
            &self.world.bodies,
            &self.world.colliders,
            filter,
        );
        let movement = self.vertical.move_shape(
            self.world.integration_parameters.dt,
            &queries,
            &character.shape,
            &Pose::from_translation(metres(from)),
            metres(desired),
            |_| {},
        );
        let moved = units(movement.translation);
        let to = from + moved;
        let supported = movement.grounded && self.ground_below(&character.shape, to);
        self.set_position(key, to)?;
        Ok(Motion {
            to,
            moved,
            supported,
        })
    }

    /// Whether static ground lies within the support reach below a body of `shape` at
    /// `position`: Rapier's controller also calls a body grounded on top of another
    /// character, which is no ground (moving supports are not part of #59).
    fn ground_below(&self, shape: &Capsule, position: Vector) -> bool {
        let queries = self.world.broad_phase.as_query_pipeline(
            &self.queries,
            &self.world.bodies,
            &self.world.colliders,
            QueryFilter::only_fixed(),
        );
        queries
            .cast_shape(
                &Pose::from_translation(metres(position)),
                -Vector::Y,
                shape,
                ShapeCastOptions {
                    target_distance: CONTACT_OFFSET,
                    stop_at_penetration: false,
                    max_time_of_impact: SUPPORT_REACH,
                    compute_impact_geometry_on_penetration: true,
                },
            )
            // Ground faces up within the controller's climbable slope.
            .is_some_and(|(_, hit)| {
                hit.normal1.dot(Vector::Y) >= self.vertical.max_slope_climb_angle.cos()
            })
    }

    /// The height (world units) at which a body above `position` would stand on the static
    /// ground directly below it: the ground ring's height under an airborne character.
    pub(crate) fn ground_height(&self, position: Vector) -> Option<f32> {
        let queries = self.world.broad_phase.as_query_pipeline(
            &self.queries,
            &self.world.bodies,
            &self.world.colliders,
            QueryFilter::only_fixed(),
        );
        let ray = Ray::new(metres(position), -Vector::Y);
        let (_, time_of_impact) = queries.cast_ray(&ray, f32::MAX, true)?;
        Some(
            position.y - time_of_impact * crate::WORLD_UNITS_PER_METER as f32
                + CONTACT_OFFSET_UNITS,
        )
    }

    /// Whether static geometry lies on the segment from `from` towards `to`, strictly
    /// before `to`. A segment starting inside static geometry is obstructed.
    pub(crate) fn segment_obstructed(&self, from: Vector, to: Vector) -> bool {
        let direction = to - from;
        if direction == Vector::ZERO {
            return false;
        }
        self.first_fixed_hit(from, direction)
            .is_some_and(|time_of_impact| time_of_impact < 1.0)
    }

    /// The earliest time (in units of `translation`, within `0..=1`) at which the segment
    /// from `origin` along `translation` meets static geometry.
    /// The floor is never an obstacle: strikes, sight and projectiles stay on the plane.
    pub(crate) fn first_fixed_hit(&self, origin: Vector, translation: Vector) -> Option<f32> {
        let mut filter = QueryFilter::only_fixed();
        if let Some(floor) = self.floor {
            filter = filter.exclude_collider(floor.collider);
        }
        let queries = self.world.broad_phase.as_query_pipeline(
            &self.queries,
            &self.world.bodies,
            &self.world.colliders,
            filter,
        );
        queries
            .cast_ray(&Ray::new(metres(origin), metres(translation)), 1.0, true)
            .map(|(_, time_of_impact)| time_of_impact)
    }

    /// Whether a character with `half_extents` at `position` would penetrate static
    /// geometry, the floor included (touching does not count).
    pub(crate) fn penetrates_fixed(&self, position: Vector, half_extents: Vec3i) -> bool {
        let shape = character_shape(half_extents);
        let pose = Pose::from_translation(metres(position));
        let queries = self.world.broad_phase.as_query_pipeline(
            &self.queries,
            &self.world.bodies,
            &self.world.colliders,
            QueryFilter::only_fixed(),
        );
        queries.intersect_shape(pose, &shape).any(|(_, collider)| {
            queries
                .dispatcher
                .contact(
                    &pose.inv_mul(collider.position()),
                    &shape,
                    collider.shape(),
                    0.0,
                )
                .ok()
                .flatten()
                .is_some_and(|contact| contact.dist < -PENETRATION_TOLERANCE)
        })
    }

    fn character(&self, key: BodyKey) -> Result<Character, GameError> {
        self.characters
            .get(&key)
            .copied()
            .ok_or_else(|| missing(key))
    }

    /// Brings Rapier's broad phase (the query acceleration structure) up to date with
    /// added, removed and moved colliders.
    fn refresh(&mut self) {
        self.world.detect_collisions(&(), &());
    }
}

/// The earliest time (in units of `translation`, within `0..=1`) at which the segment from
/// `origin` along `translation` meets a box hurt volume centred on `center`.
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

/// An upright capsule as wide and tall as `half_extents`.
fn character_shape(half_extents: Vec3i) -> Capsule {
    let radius = half_extents.x.max(half_extents.z) as f32;
    let half_height = (half_extents.y as f32 - radius).max(0.0);
    Capsule::new_y(half_height * METRES_PER_UNIT, radius * METRES_PER_UNIT)
}

fn missing(key: BodyKey) -> GameError {
    GameError::new(format!("physics body {key} is missing"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: Vec3i = Vec3i::new(30, 50, 30);
    const Y: i32 = 50;
    /// How far a character pressed against something stands off it: Rapier's controller
    /// keeps a 0.25 unit contact offset (plus its 0.01 unit normal nudge), never overlapping.
    const GAP: std::ops::RangeInclusive<f32> = 0.2..=0.3;

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
    }

    #[test]
    fn sliding_along_a_wall_keeps_the_tangential_motion() {
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
    }

    #[test]
    fn an_inner_corner_holds_a_character_without_sticking_it() {
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
    }

    #[test]
    fn characters_block_each_other_from_every_side_without_overlapping() {
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
    }

    #[test]
    fn overlapping_upright_capsules_separate_along_the_line_between_them() {
        let physics = Physics::new();
        let shape = character_shape(BODY);
        // 50 units apart on a diagonal: 10 units of overlap.
        let pos12 = Pose::from_translation(metres(Vector::new(30.0, 0.0, 40.0)));
        let contact = physics
            .queries
            .contact(&pos12, &shape, &shape, 0.0)
            .unwrap()
            .unwrap();
        assert!((units(Vector::splat(contact.dist)).x + 10.0).abs() < 1.0e-3);
        assert!((contact.normal1 - Vector::new(0.6, 0.0, 0.8)).length() < 1.0e-4);
    }

    #[test]
    fn a_removed_door_collider_no_longer_blocks() {
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
        assert_eq!(moved, Vector::new(7.0, 0.0, 0.0));
    }

    #[test]
    fn segments_are_obstructed_only_by_static_geometry_before_their_end() {
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
    }

    #[test]
    fn a_rising_body_leaves_the_ground_and_lands_back_on_it() {
        let mut physics = floored();
        physics.add_character(1, at(0, 0), BODY).unwrap();
        let gravity = 0.5;
        let mut velocity = 10.0;
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
    }

    #[test]
    fn an_airborne_body_slides_along_a_wall_as_precisely_as_on_the_ground() {
        let mut physics = floored();
        // A 60 m wall reaching 1.5 m above the plane's feet; its face is at x = 100.
        physics
            .add_fixed(1, Vec3i::new(125, Y, 0), Vec3i::new(25, 100, 3_000))
            .unwrap();
        let start = at(60, -1_000);
        physics.add_character(2, start, BODY).unwrap();
        let mut velocity = 9.0;
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
    }

    #[test]
    fn a_body_falling_beside_another_keeps_its_distance_and_lands() {
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
    }

    #[test]
    fn standing_on_another_body_is_not_support() {
        let mut physics = floored();
        physics.add_character(1, at(0, 0), BODY).unwrap();
        // Directly above the other body, its lowest point touching the other's top.
        let above = Vector::new(0.0, Y as f32 + 100.3, 0.0);
        physics.add_character(2, above, BODY).unwrap();
        let motion = physics
            .move_player(2, above, Vector::new(0.0, -1.0, 0.0))
            .unwrap();
        assert!(!motion.supported, "{motion:?}");
    }

    #[test]
    fn the_floor_never_obstructs_segments_and_carries_the_ground_height() {
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
    }
}
