//! The Rapier backend (Atlas 2.3.1, 2.3.2): the production authority.
//!
//! Level geometry (walls, pillars, locked doors) is static cuboid colliders on a flat floor, an
//! upward half-space (#59). Players and monsters are kinematic position-based bodies with
//! capsule colliders, moved by Rapier's `KinematicCharacterController`, which resolves contact,
//! sliding, corners and, for players, support: whether the body stands on the ground.
//!
//! The character controller's contact queries go through [`queries`](super::queries), exact
//! closed forms for the upright capsules and boxes the game uses, ahead of parry's general
//! GJK/EPA path.

use std::collections::BTreeMap;
use std::fmt;

use rapier3d::control::{CharacterAutostep, CharacterLength, KinematicCharacterController};
use rapier3d::parry::query::{
    DefaultQueryDispatcher, QueryDispatcher, QueryDispatcherChain, ShapeCastOptions,
};
use rapier3d::prelude::*;

use super::queries::UprightShapes;
use super::{
    BodyKey, CONTACT_OFFSET, CONTACT_OFFSET_UNITS, FixedShape, METRES_PER_UNIT, Motion,
    PENETRATION_TOLERANCE, STEP_HEIGHT_UNITS, SUPPORT_REACH, capsule_dimensions, metres, missing,
    units, vector,
};
use crate::{GameError, TICK_HZ, Vec3i};

/// One static box or ramp of the game and its collider.
struct Fixed {
    center: Vec3i,
    half_extents: Vec3i,
    shape: FixedShape,
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

pub(crate) struct Physics {
    world: PhysicsWorld,
    queries: QueryDispatcherChain<UprightShapes, DefaultQueryDispatcher>,
    /// Planar motion (monsters): no vertical intent, no snapping.
    controller: KinematicCharacterController,
    /// Players' motion with height: the same contact rules, snapping to the ground the
    /// controller found the body grounded on.
    vertical: KinematicCharacterController,
    /// [`Self::vertical`] for a walking player, which also climbs steps up to
    /// [`STEP_HEIGHT_UNITS`] (Rapier's `autostep`). Jumps and falls never step: Rapier's
    /// autostep would otherwise lift an airborne body onto any ledge within a step of it.
    walking: KinematicCharacterController,
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

/// An empty Rapier world.
fn empty_world() -> PhysicsWorld {
    let mut world = PhysicsWorld::new();
    // Gravity and jumps are movement intent the game owns (#59): no Rapier body is dynamic,
    // so nothing falls by itself.
    world.gravity = Vector::ZERO;
    world.integration_parameters.dt = 1.0 / f32::from(TICK_HZ);
    world
}

/// Inserts a static box or ramp collider. A ramp is the convex hull of its wedge: the bottom
/// of its bounding box and the top edge at its -Z end.
fn insert_fixed(
    world: &mut PhysicsWorld,
    center: Vec3i,
    half_extents: Vec3i,
    shape: FixedShape,
) -> ColliderHandle {
    let half = metres(vector(half_extents));
    let builder = match shape {
        FixedShape::Box => ColliderBuilder::cuboid(half.x, half.y, half.z),
        FixedShape::Ramp => {
            let points = [
                Vector::new(-half.x, -half.y, -half.z),
                Vector::new(half.x, -half.y, -half.z),
                Vector::new(-half.x, -half.y, half.z),
                Vector::new(half.x, -half.y, half.z),
                Vector::new(-half.x, half.y, -half.z),
                Vector::new(half.x, half.y, -half.z),
            ];
            ColliderBuilder::convex_hull(&points).expect("a ramp's wedge has a convex hull")
        }
    };
    world
        .colliders
        .insert(builder.translation(metres(vector(center))).build())
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
            // No autostep: a step or platform is climbed by jumping (#208).
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
            walking: KinematicCharacterController {
                // Walking players climb stairs without jumping (Atlas 5.1.5, #222).
                autostep: Some(CharacterAutostep {
                    max_height: CharacterLength::Absolute(STEP_HEIGHT_UNITS * METRES_PER_UNIT),
                    min_width: CharacterLength::Absolute(0.1),
                    include_dynamic_bodies: false,
                }),
                snap_to_ground: Some(CharacterLength::Absolute(SUPPORT_REACH)),
                ..controller
            },
            vertical: KinematicCharacterController {
                // Settles a body the controller found grounded within its support reach, also
                // over a platform's rounded edge (#208).
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
        self.add_static(key, center, half_extents, FixedShape::Box)
    }

    /// Adds a static ramp inside the box centred on `center` with `half_extents`, rising
    /// from its +Z end on the box's bottom to its -Z end on the box's top.
    pub(crate) fn add_ramp(
        &mut self,
        key: BodyKey,
        center: Vec3i,
        half_extents: Vec3i,
    ) -> Result<(), GameError> {
        self.add_static(key, center, half_extents, FixedShape::Ramp)
    }

    fn add_static(
        &mut self,
        key: BodyKey,
        center: Vec3i,
        half_extents: Vec3i,
        shape: FixedShape,
    ) -> Result<(), GameError> {
        if self.fixed.contains_key(&key) {
            return Err(GameError::new(format!("physics body {key} already exists")));
        }
        let collider = insert_fixed(&mut self.world, center, half_extents, shape);
        self.fixed.insert(
            key,
            Fixed {
                center,
                half_extents,
                shape,
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

    /// Centre and half extents of every static box (a ramp's bounding box), in key order.
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
            fixed.collider =
                insert_fixed(&mut world, fixed.center, fixed.half_extents, fixed.shape);
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
    /// reports whether it ends supported. A `walking` player (grounded, not taking off)
    /// climbs steps.
    pub(crate) fn move_player(
        &mut self,
        key: BodyKey,
        from: Vector,
        desired: Vector,
        walking: bool,
    ) -> Result<Motion, GameError> {
        let character = self.character(key)?;
        let filter = QueryFilter::default().exclude_rigid_body(character.body);
        let queries = self.world.broad_phase.as_query_pipeline(
            &self.queries,
            &self.world.bodies,
            &self.world.colliders,
            filter,
        );
        let controller = if walking {
            &self.walking
        } else {
            &self.vertical
        };
        let movement = controller.move_shape(
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
            // Rapier's grounded, on static geometry: a step's edge counts, a character
            // never (it would let a walk step onto another body).
            ground_contact: movement.grounded
                && self
                    .static_below(&character.shape, to)
                    .is_some_and(|normal| normal.y > 0.0),
        })
    }

    /// Whether static ground lies within the support reach below a body of `shape` at
    /// `position`: Rapier's controller also calls a body grounded on top of another
    /// character, which is no ground (moving supports are not part of #59).
    fn ground_below(&self, shape: &Capsule, position: Vector) -> bool {
        self.static_below(shape, position).is_some_and(|normal| {
            normal.dot(Vector::Y) >= self.vertical.max_slope_climb_angle.cos()
        })
    }

    /// The normal of the static geometry within the support reach below a body of `shape`
    /// at `position`, at any angle: a step's rounded edge counts, another character never.
    fn static_below(&self, shape: &Capsule, position: Vector) -> Option<Vector> {
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
            .map(|(_, hit)| hit.normal1)
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
        self.segment_obstructed_except(from, to, None)
    }

    /// [`Self::segment_obstructed`] ignoring the static body `except`: a strike at a
    /// damageable prop is not obstructed by the prop itself (#90).
    pub(crate) fn segment_obstructed_except(
        &self,
        from: Vector,
        to: Vector,
        except: Option<BodyKey>,
    ) -> bool {
        let direction = to - from;
        if direction == Vector::ZERO {
            return false;
        }
        self.first_fixed_hit_except(from, direction, except)
            .is_some_and(|time_of_impact| time_of_impact < 1.0)
    }

    /// The earliest time (in units of `translation`, within `0..=1`) at which the segment
    /// from `origin` along `translation` meets static geometry.
    /// The floor is never an obstacle: strikes, sight and projectiles stay on the plane.
    pub(crate) fn first_fixed_hit(&self, origin: Vector, translation: Vector) -> Option<f32> {
        self.first_fixed_hit_except(origin, translation, None)
    }

    /// [`Self::first_fixed_hit`] ignoring the static body `except`.
    fn first_fixed_hit_except(
        &self,
        origin: Vector,
        translation: Vector,
        except: Option<BodyKey>,
    ) -> Option<f32> {
        let mut filter = QueryFilter::only_fixed();
        if let Some(floor) = self.floor {
            filter = filter.exclude_collider(floor.collider);
        }
        let excluded = except
            .and_then(|key| self.fixed.get(&key))
            .map(|fixed| fixed.collider);
        let other = |handle: ColliderHandle, _: &Collider| Some(handle) != excluded;
        if excluded.is_some() {
            filter = filter.predicate(&other);
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

    /// Whether static ground (the floor or a box) lies within `reach` world units below a
    /// character with `half_extents` at `position`, edges included: what a saved grounded
    /// body must stand on (#222).
    pub(crate) fn supported_within(
        &self,
        position: Vector,
        half_extents: Vec3i,
        reach: f32,
    ) -> bool {
        let shape = character_shape(half_extents);
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
                &shape,
                ShapeCastOptions {
                    target_distance: 0.0,
                    stop_at_penetration: true,
                    max_time_of_impact: reach * METRES_PER_UNIT,
                    compute_impact_geometry_on_penetration: false,
                },
            )
            .is_some()
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

    /// The characters whose bodies overlap an upright column (a capsule of `radius` around a
    /// segment `half_height` above and below `center`, world units), in key order: the
    /// candidates of an area effect (#64). Touching counts.
    pub(crate) fn characters_in_column(
        &self,
        center: Vector,
        radius: f32,
        half_height: f32,
    ) -> Vec<BodyKey> {
        let shape = Capsule::new_y(half_height * METRES_PER_UNIT, radius * METRES_PER_UNIT);
        let pose = Pose::from_translation(metres(center));
        let queries = self.world.broad_phase.as_query_pipeline(
            &self.queries,
            &self.world.bodies,
            &self.world.colliders,
            QueryFilter::exclude_fixed(),
        );
        let bodies = queries
            .intersect_shape(pose, &shape)
            .filter_map(|(_, collider)| collider.parent())
            .collect::<Vec<_>>();
        self.characters
            .iter()
            .filter(|(_, character)| bodies.contains(&character.body))
            .map(|(&key, _)| key)
            .collect()
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

/// An upright capsule as wide and tall as `half_extents`.
fn character_shape(half_extents: Vec3i) -> Capsule {
    let (radius, half_height) = capsule_dimensions(half_extents);
    Capsule::new_y(half_height * METRES_PER_UNIT, radius * METRES_PER_UNIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: Vec3i = Vec3i::new(30, 50, 30);

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
}
