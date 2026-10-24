//! The physics-engine backend (physics-engine#330): a thin adapter over
//! `physics_engine::approximate::World`, compiled only with the test/bench-only `physics-ab`
//! feature. Rapier stays the production authority (Atlas 2.3.1).
//!
//! It answers every call of the Rapier backend with the same game-facing meaning:
//!
//! - Units: the same metre conversion as the Rapier backend ([`metres`], [`units`]), with the
//!   world's `Config::length_unit` left at 1 (metres). Both backends therefore see identical
//!   `f32` inputs, and the engine's metre defaults and relative tolerances apply unscaled.
//! - The floor is a keyless static `HalfSpace` one contact offset below the feet, ordered after
//!   every keyed body as Rapier's rebuild inserts it last. Planar moves never see it: a planar
//!   character drops vertical normals, and the floor's normal is vertical. Rays exclude it;
//!   [`Physics::penetrates_fixed`] and [`Physics::ground_height`] include it.
//! - Static boxes are keyed static bodies; doors are added and removed as the game locks them.
//! - Characters are keyed external capsules on their own collision layer, so static-only
//!   queries skip them. Monsters move planar (`CharacterSettings::plane`), players in 3D with
//!   snapping to the ground within [`SUPPORT_REACH`]. The engine's grounded test counts only
//!   walkable static support, so a body on another character is not supported, as in the
//!   Rapier backend. Characters keep the same 0.25 unit contact offset.
//! - Each game move is its own single-element `move_characters` batch, in the game's order
//!   (players by id, then monsters): the engine orders a batch by body key instead.
//! - The game's position after a move is mirrored into the body, as the Rapier backend does,
//!   so a world rebuilt from a save matches the uninterrupted one.
//! - [`Physics::rebuild`] does nothing: the engine's results depend on keys and current
//!   contents, not on insertion or move history, which is what the Rapier backend rebuilds for.
//!
//! Intentional behavioural differences, kept rather than hidden:
//!
//! - A character blocked head-on (within about 5 degrees) by another character sidesteps to
//!   its right; Rapier's controller stops it.
//! - Rapier's controller adds a 0.01 unit normal nudge to the 0.25 unit gap; the engine keeps
//!   the gap exactly.
//! - Overlap is resolved before the move and reported apart from it; the adapter adds it back,
//!   because the game reads Rapier's translation, which includes it. Overlapping characters are
//!   separated horizontally (the mover goes the whole way, since a batch holds one mover).
//! - A character's movement kind is fixed in the engine when it is added. The game's
//!   `add_character` does not say whether the body is a player, so it starts planar and turns
//!   into a 3D mover (re-registered with the player settings) on its first `move_player`.
//! - [`Physics::ground_height`] probes 1 km down; Rapier's ray is unbounded.
//! - A zero-length [`Physics::first_fixed_hit`] meets nothing (the engine rejects zero rays).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;

use physics_engine::approximate::{
    Autostep, Body, CharacterMove, CharacterMoveResult, CharacterSettings, Config, Quaternion,
    QueryFilter, QueryHit, QueryPose, RayCast, Shape, ShapeCast, Vector as EngineVector, World,
};
use physics_engine::{BodyId, CollisionLayers3d};
use rapier3d::math::Vector;

use super::{
    BodyKey, CONTACT_OFFSET, CONTACT_OFFSET_UNITS, FixedShape, Motion, PENETRATION_TOLERANCE,
    STEP_HEIGHT_UNITS, SUPPORT_REACH, capsule_dimensions, metres, missing, units, vector,
};
use crate::{GameError, Vec3i};

/// Characters' layer: static-only queries ([`STATICS`]) skip them; moves see everything.
const CHARACTERS: CollisionLayers3d = CollisionLayers3d::new(2, u32::MAX);
/// The floor's own layer, so wall queries skip it by layer and keep `exclude` for a struck
/// body (#90).
const FLOOR: CollisionLayers3d = CollisionLayers3d::new(4, u32::MAX);
/// Static boxes only: every body but the characters and the floor.
const STATICS: CollisionLayers3d = CollisionLayers3d::new(1, 1);
/// Static ground: the boxes and the floor.
const GROUND: CollisionLayers3d = CollisionLayers3d::new(1, 1 | 4);
/// How far (metres) below a body [`Physics::ground_height`] looks for ground.
const GROUND_PROBE: f32 = 1.0e3;

struct Fixed {
    center: Vec3i,
    half_extents: Vec3i,
    body: BodyId,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Monsters: horizontal motion on their plane.
    Planar,
    /// Players: motion with height, snapping to the ground.
    Vertical,
}

#[derive(Clone, Copy)]
struct Character {
    body: BodyId,
    kind: Kind,
    /// The game's position of the body, in world units.
    position: Vector,
}

pub(crate) struct Physics {
    world: World,
    /// Whether the floor was laid; queries tell it apart by its layer, not its body.
    floor: bool,
    fixed: BTreeMap<BodyKey, Fixed>,
    characters: BTreeMap<BodyKey, Character>,
    results: Vec<CharacterMoveResult>,
    hits: RefCell<Vec<QueryHit>>,
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

/// A world-unit vector in the engine's metres.
fn engine(value: Vector) -> EngineVector {
    let value = metres(value);
    EngineVector(value.x, value.y, value.z)
}

/// An engine length or position (metres) in world units.
fn game(value: EngineVector) -> Vector {
    units(Vector::new(value.0, value.1, value.2))
}

fn failed(error: impl fmt::Debug) -> GameError {
    GameError::new(format!("physics-engine: {error:?}"))
}

/// An upright capsule as wide and tall as `half_extents`.
fn character_shape(half_extents: Vec3i) -> Shape {
    let (radius, half_height) = capsule_dimensions(half_extents);
    Shape::capsule(
        half_height * super::METRES_PER_UNIT,
        radius * super::METRES_PER_UNIT,
    )
}

fn settings(kind: Kind, height: f32) -> CharacterSettings {
    let base = CharacterSettings {
        offset: CONTACT_OFFSET,
        max_slope: std::f32::consts::FRAC_PI_4,
        ..CharacterSettings::default()
    };
    match kind {
        Kind::Planar => CharacterSettings {
            snap_to_ground: None,
            plane: Some(height),
            ..base
        },
        Kind::Vertical => CharacterSettings {
            // Settles a body grounded within the support reach, as the Rapier backend does.
            snap_to_ground: Some(SUPPORT_REACH),
            plane: None,
            // Walking players climb stairs (Atlas 5.1.5, #222), with the Rapier backend's
            // step height and free depth. The engine steps only on walking moves: ones that
            // start on the ground and do not rise.
            autostep: Some(Autostep {
                max_height: STEP_HEIGHT_UNITS * super::METRES_PER_UNIT,
                min_width: 0.1,
            }),
            ..base
        },
    }
}

impl Physics {
    pub(crate) fn new() -> Self {
        let world = World::new(Config {
            // Gravity and jumps are movement intent the game owns (#59).
            gravity: EngineVector(0.0, 0.0, 0.0),
            ..Config::default()
        })
        .expect("the default physics-engine configuration is valid");
        Self {
            world,
            floor: false,
            fixed: BTreeMap::new(),
            characters: BTreeMap::new(),
            results: Vec::with_capacity(1),
            hits: RefCell::new(Vec::new()),
        }
    }

    pub(crate) fn add_floor(&mut self, foot_height: f32) -> Result<(), GameError> {
        if self.floor {
            return Err(GameError::new("the floor already exists"));
        }
        let surface = Vector::new(0.0, foot_height - CONTACT_OFFSET_UNITS, 0.0);
        let mut floor = Body::new(Shape::HalfSpace, engine(surface), 0.0);
        floor.layers = FLOOR;
        self.world.add_body(floor).map_err(failed)?;
        self.floor = true;
        Ok(())
    }

    pub(crate) fn add_fixed(
        &mut self,
        key: BodyKey,
        center: Vec3i,
        half_extents: Vec3i,
    ) -> Result<(), GameError> {
        self.add_static(key, center, half_extents, FixedShape::Box)
    }

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
        let body = match shape {
            FixedShape::Box => Body::keyed(
                key,
                Shape::Box(engine(vector(half_extents))),
                engine(vector(center)),
                0.0,
            ),
            FixedShape::Ramp => {
                // The engine's wedge rises toward its local -X; a quarter turn about Y
                // (local +X to world +Z) makes it rise toward world -Z.
                let mut body = Body::keyed(
                    key,
                    Shape::wedge(engine(vector(Vec3i::new(
                        half_extents.z,
                        half_extents.y,
                        half_extents.x,
                    )))),
                    engine(vector(center)),
                    0.0,
                );
                let half_turn = std::f32::consts::FRAC_PI_4;
                body.orientation = Quaternion(0.0, -half_turn.sin(), 0.0, half_turn.cos());
                body
            }
        };
        let body = self.world.add_body(body).map_err(failed)?;
        self.fixed.insert(
            key,
            Fixed {
                center,
                half_extents,
                body,
            },
        );
        Ok(())
    }

    pub(crate) fn remove_fixed(&mut self, key: BodyKey) -> bool {
        let Some(fixed) = self.fixed.remove(&key) else {
            return false;
        };
        self.world.remove_body(fixed.body);
        true
    }

    #[cfg(test)]
    pub(crate) fn has_fixed(&self, key: BodyKey) -> bool {
        self.fixed.contains_key(&key)
    }

    pub(crate) fn fixed_boxes(&self) -> impl Iterator<Item = (Vec3i, Vec3i)> + '_ {
        self.fixed
            .values()
            .map(|fixed| (fixed.center, fixed.half_extents))
    }

    pub(crate) fn add_character(
        &mut self,
        key: BodyKey,
        position: Vector,
        half_extents: Vec3i,
    ) -> Result<(), GameError> {
        if self.characters.contains_key(&key) {
            return Err(GameError::new(format!("physics body {key} already exists")));
        }
        let mut body = Body::keyed(key, character_shape(half_extents), engine(position), 1.0);
        body.external = true;
        body.layers = CHARACTERS;
        let body = self.world.add_body(body).map_err(failed)?;
        let kind = Kind::Planar;
        self.world
            .add_character(body, settings(kind, engine(position).1))
            .map_err(failed)?;
        self.characters.insert(
            key,
            Character {
                body,
                kind,
                position,
            },
        );
        Ok(())
    }

    pub(crate) fn remove_character(&mut self, key: BodyKey) -> bool {
        let Some(character) = self.characters.remove(&key) else {
            return false;
        };
        self.world.remove_body(character.body);
        true
    }

    pub(crate) fn has_character(&self, key: BodyKey) -> bool {
        self.characters.contains_key(&key)
    }

    pub(crate) fn character_position(&self, key: BodyKey) -> Option<Vector> {
        Some(self.characters.get(&key)?.position)
    }

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
            .set_kinematic_position(character.body, engine(position))
            .map_err(failed)
    }

    /// Nothing to rebuild: the engine's queries and moves depend on body keys and current
    /// contents, not on the history Rapier's broad phase keeps.
    pub(crate) fn rebuild(&mut self) {}

    /// Moves the character by `desired` (world units) as one single-element batch from the
    /// game's `from`, with the movement kind `kind`. Returns the applied translation in world
    /// units (overlap resolution included) and whether it ended grounded.
    fn step(
        &mut self,
        key: BodyKey,
        kind: Kind,
        from: Vector,
        desired: Vector,
    ) -> Result<(Vector, bool), GameError> {
        self.set_position(key, from)?;
        let character = self.characters.get_mut(&key).ok_or_else(|| missing(key))?;
        if character.kind != kind {
            character.kind = kind;
            self.world
                .add_character(character.body, settings(kind, engine(from).1))
                .map_err(failed)?;
        }
        let body = character.body;
        self.world
            .move_characters(
                &[CharacterMove {
                    body,
                    desired: engine(desired),
                }],
                &mut self.results,
            )
            .map_err(failed)?;
        let result = self.results[0];
        Ok((
            game(result.translation + result.depenetration),
            result.grounded,
        ))
    }

    /// Mirrors the game's `to`, which the game derives from `from` and the translation in world
    /// units, in the body, as the Rapier backend does. The engine's own metre position differs
    /// from it by `f32` rounding of the unit conversion; keeping it would make a world rebuilt
    /// from a save differ from the uninterrupted one by that rounding.
    fn moved_to(&mut self, key: BodyKey, to: Vector) -> Result<(), GameError> {
        let character = self.characters.get_mut(&key).ok_or_else(|| missing(key))?;
        character.position = to;
        let body = character.body;
        let mirrored = engine(to);
        if self.world.body(body).map(|body| body.position) != Some(mirrored) {
            self.world
                .set_kinematic_position(body, mirrored)
                .map_err(failed)?;
        }
        Ok(())
    }

    pub(crate) fn move_character(
        &mut self,
        key: BodyKey,
        from: Vector,
        desired: Vector,
    ) -> Result<(Vector, Vector), GameError> {
        let (moved, _) = self.step(
            key,
            Kind::Planar,
            from,
            Vector::new(desired.x, 0.0, desired.z),
        )?;
        let translation = Vector::new(moved.x, 0.0, moved.z);
        let to = from + translation;
        self.moved_to(key, to)?;
        Ok((to, translation))
    }

    /// The engine decides walking itself (on the ground at the move's start, not rising),
    /// the game's own rule, so `walking` adds nothing here (#222).
    pub(crate) fn move_player(
        &mut self,
        key: BodyKey,
        from: Vector,
        desired: Vector,
        _walking: bool,
    ) -> Result<Motion, GameError> {
        let (moved, supported) = self.step(key, Kind::Vertical, from, desired)?;
        let to = from + moved;
        self.moved_to(key, to)?;
        Ok(Motion {
            to,
            moved,
            supported,
            // The engine gates its autostep on geometry at the move's start (#222).
            ground_contact: supported,
        })
    }

    /// Static ground: the boxes and the floor.
    fn statics(&self) -> QueryFilter {
        QueryFilter {
            layers: GROUND,
            ..QueryFilter::default()
        }
    }

    /// Static boxes without the floor (strikes, sight and projectiles stay on the plane),
    /// and without the static body `except`.
    fn walls(&self, except: Option<BodyKey>) -> QueryFilter {
        QueryFilter {
            layers: STATICS,
            exclude: except
                .and_then(|key| self.fixed.get(&key))
                .map(|fixed| fixed.body),
            ..QueryFilter::default()
        }
    }

    pub(crate) fn ground_height(&self, position: Vector) -> Option<f32> {
        let ray = RayCast::new(engine(position), EngineVector(0.0, -GROUND_PROBE, 0.0));
        let hit = self
            .world
            .cast_ray_first(ray, self.statics())
            .expect("physics-engine ground ray");
        Some(game(hit?.point).y + CONTACT_OFFSET_UNITS)
    }

    pub(crate) fn segment_obstructed(&self, from: Vector, to: Vector) -> bool {
        self.segment_obstructed_except(from, to, None)
    }

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

    pub(crate) fn first_fixed_hit(&self, origin: Vector, translation: Vector) -> Option<f32> {
        self.first_fixed_hit_except(origin, translation, None)
    }

    fn first_fixed_hit_except(
        &self,
        origin: Vector,
        translation: Vector,
        except: Option<BodyKey>,
    ) -> Option<f32> {
        if translation == Vector::ZERO {
            return None;
        }
        self.world
            .cast_ray_first(
                RayCast::new(engine(origin), engine(translation)),
                self.walls(except),
            )
            .expect("physics-engine ray")
            .map(|hit| hit.fraction)
    }

    pub(crate) fn supported_within(
        &self,
        position: Vector,
        half_extents: Vec3i,
        reach: f32,
    ) -> bool {
        let mut hits = self.hits.borrow_mut();
        let cast = ShapeCast::new(
            QueryPose::new(character_shape(half_extents), engine(position)),
            EngineVector(0.0, -reach * super::METRES_PER_UNIT, 0.0),
        );
        self.world
            .cast_shape(cast, self.statics(), &mut hits)
            .expect("physics-engine support cast");
        !hits.is_empty()
    }

    pub(crate) fn characters_in_column(
        &self,
        center: Vector,
        radius: f32,
        half_height: f32,
    ) -> Vec<BodyKey> {
        let mut hits = self.hits.borrow_mut();
        let shape = Shape::capsule(
            half_height * super::METRES_PER_UNIT,
            radius * super::METRES_PER_UNIT,
        );
        self.world
            .overlap_shape(
                QueryPose::new(shape, engine(center)),
                QueryFilter::default(),
                &mut hits,
            )
            .expect("physics-engine area overlap");
        self.characters
            .iter()
            .filter(|(_, character)| hits.iter().any(|hit| hit.body == character.body))
            .map(|(&key, _)| key)
            .collect()
    }

    pub(crate) fn penetrates_fixed(&self, position: Vector, half_extents: Vec3i) -> bool {
        let mut hits = self.hits.borrow_mut();
        self.world
            .overlap_shape(
                QueryPose::new(character_shape(half_extents), engine(position)),
                self.statics(),
                &mut hits,
            )
            .expect("physics-engine overlap");
        hits.iter()
            .any(|hit| hit.separation < -PENETRATION_TOLERANCE)
    }
}
