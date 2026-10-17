//! Exact contact and shape-cast queries for the shape pairs the character controller meets:
//! an upright capsule against another upright capsule or against an axis-aligned box whose
//! height spans the capsule's axis, moving horizontally.
//!
//! parry 0.31's general GJK/EPA path is not precise enough for them in `f32`:
//!
//! - Two overlapping parallel capsules get zero depth and a normal perpendicular to the true
//!   one (EPA on a degenerate Minkowski difference), so the controller slid one character
//!   into another.
//! - A capsule sliding along a long box (the dungeon's 60 m outer walls) got normals off by
//!   up to a few degrees, so it jumped up to 2 units off the wall and lost or gained a few
//!   percent of its speed from one tick to the next.
//!
//! Both pairs reduce to circles and rectangles on the plane, solved here in closed form.
//! Every other pair, pose or motion falls through to Rapier's default dispatcher.

use rapier3d::parry::query::details::{cast_shapes_ball_ball, contact_ball_ball};
use rapier3d::parry::query::{
    ClosestPoints, Contact, NonlinearRigidMotion, QueryDispatcher, ShapeCastHit, ShapeCastOptions,
    ShapeCastStatus, ShapeDistance, ShapeIntersection, Unsupported,
};
use rapier3d::prelude::*;

/// Below this time of impact (metres along a unit cast) a cast that separates or slides
/// reports nothing, as parry's own ball cast does.
const SMALL_TIME_OF_IMPACT: f32 = 1.0e-4;

pub(super) struct UprightShapes;

/// The vertical span of a capsule's axis, if the capsule is upright at its frame's origin.
fn upright_axis(capsule: &Capsule) -> Option<(f32, f32)> {
    let (a, b) = (capsule.segment.a, capsule.segment.b);
    (a.x == 0.0 && a.z == 0.0 && b.x == 0.0 && b.z == 0.0).then(|| (a.y.min(b.y), a.y.max(b.y)))
}

/// An upright capsule seen from an axis-aligned box: the capsule centre in the box's frame
/// and its radius, when the box's height spans the capsule's axis so that only the box's
/// vertical faces and edges can be nearest.
struct CapsuleBesideBox {
    centre: Vector,
    radius: f32,
    half: Vector,
}

impl CapsuleBesideBox {
    fn new(capsule: &Capsule, cuboid: &Cuboid, capsule_in_box: &Pose) -> Option<Self> {
        if capsule_in_box.rotation != Rotation::IDENTITY {
            return None;
        }
        let (low, high) = upright_axis(capsule)?;
        let centre = capsule_in_box.translation;
        let half = cuboid.half_extents;
        (centre.y + low >= -half.y && centre.y + high <= half.y).then_some(Self {
            centre,
            radius: capsule.radius,
            half,
        })
    }

    /// The nearest point of the box's footprint to `at` (at `at`'s height), and the unit
    /// normal from the box towards `at` with the signed distance between them.
    fn nearest(&self, at: Vector) -> (Vector, Vector, f32) {
        let clamped = Vector::new(
            at.x.clamp(-self.half.x, self.half.x),
            at.y,
            at.z.clamp(-self.half.z, self.half.z),
        );
        let outside = at - clamped;
        let distance = outside.length();
        if distance > 0.0 {
            return (clamped, outside / distance, distance);
        }
        // Inside the footprint: out through the nearest side.
        let (depth_x, depth_z) = (self.half.x - at.x.abs(), self.half.z - at.z.abs());
        if depth_x <= depth_z {
            let side = if at.x < 0.0 { -1.0 } else { 1.0 };
            (
                Vector::new(side * self.half.x, at.y, at.z),
                Vector::new(side, 0.0, 0.0),
                -depth_x,
            )
        } else {
            let side = if at.z < 0.0 { -1.0 } else { 1.0 };
            (
                Vector::new(at.x, at.y, side * self.half.z),
                Vector::new(0.0, 0.0, side),
                -depth_z,
            )
        }
    }

    /// The earliest time in `0..=max` at which the capsule centre moving by `velocity`
    /// comes within `reach` of the footprint, starting outside that reach.
    fn time_of_impact(&self, velocity: Vector, reach: f32, max: f32) -> Option<f32> {
        let (px, pz, vx, vz) = (self.centre.x, self.centre.z, velocity.x, velocity.z);
        // Entry into an axis-aligned rectangle centred on the origin.
        let slab = |half_x: f32, half_z: f32| -> Option<f32> {
            let axis = |p: f32, v: f32, half: f32| -> Option<(f32, f32)> {
                if v == 0.0 {
                    return (p.abs() <= half).then_some((f32::NEG_INFINITY, f32::INFINITY));
                }
                let (a, b) = ((-half - p) / v, (half - p) / v);
                Some((a.min(b), a.max(b)))
            };
            let (enter_x, exit_x) = axis(px, vx, half_x)?;
            let (enter_z, exit_z) = axis(pz, vz, half_z)?;
            let (enter, exit) = (enter_x.max(enter_z), exit_x.min(exit_z));
            (enter <= exit && enter >= 0.0).then_some(enter)
        };
        // Entry into a circle of `reach` around a corner.
        let corner = |cx: f32, cz: f32| -> Option<f32> {
            let (dx, dz) = (px - cx, pz - cz);
            let a = vx * vx + vz * vz;
            let b = dx * vx + dz * vz;
            let c = dx * dx + dz * dz - reach * reach;
            let discriminant = b * b - a * c;
            if a == 0.0 || discriminant < 0.0 {
                return None;
            }
            let t = (-b - discriminant.sqrt()) / a;
            (t >= 0.0).then_some(t)
        };
        let (hx, hz) = (self.half.x, self.half.z);
        [
            slab(hx + reach, hz),
            slab(hx, hz + reach),
            corner(hx, hz),
            corner(hx, -hz),
            corner(-hx, hz),
            corner(-hx, -hz),
        ]
        .into_iter()
        .flatten()
        .filter(|&t| t <= max)
        .min_by(f32::total_cmp)
    }
}

/// Two upright capsules whose axes overlap in height, as two balls at a shared height: their
/// radii, the second ball's pose relative to the first and each ball centre in its own
/// capsule's frame.
fn capsules_as_balls(
    pos12: &Pose,
    c1: &Capsule,
    c2: &Capsule,
) -> Option<(Ball, Ball, Pose, Vector, Vector)> {
    if pos12.rotation != Rotation::IDENTITY {
        return None;
    }
    let ((low1, high1), (low2, high2)) = (upright_axis(c1)?, upright_axis(c2)?);
    let offset = pos12.translation;
    let (low, high) = (low1.max(low2 + offset.y), high1.min(high2 + offset.y));
    if low > high {
        return None;
    }
    let height = (low + high) / 2.0;
    Some((
        Ball::new(c1.radius),
        Ball::new(c2.radius),
        Pose::from_translation(Vector::new(offset.x, 0.0, offset.z)),
        Vector::new(0.0, height, 0.0),
        Vector::new(0.0, height - offset.y, 0.0),
    ))
}

/// The capsule and the box of a capsule-box pair with the capsule's pose in the box's
/// frame, and whether the box is the first shape.
fn capsule_and_box<'a>(
    pos12: &Pose,
    g1: &'a dyn Shape,
    g2: &'a dyn Shape,
) -> Option<(CapsuleBesideBox, bool)> {
    if let (Some(cuboid), Some(capsule)) = (g1.as_cuboid(), g2.as_capsule()) {
        return Some((CapsuleBesideBox::new(capsule, cuboid, pos12)?, true));
    }
    let (capsule, cuboid) = (g1.as_capsule()?, g2.as_cuboid()?);
    Some((
        CapsuleBesideBox::new(capsule, cuboid, &pos12.inverse())?,
        false,
    ))
}

impl QueryDispatcher for UprightShapes {
    fn intersection_test(
        &self,
        _: &Pose,
        _: &dyn Shape,
        _: &dyn Shape,
    ) -> Result<ShapeIntersection, Unsupported> {
        Err(Unsupported)
    }

    fn distance(
        &self,
        _: &Pose,
        _: &dyn Shape,
        _: &dyn Shape,
    ) -> Result<ShapeDistance, Unsupported> {
        Err(Unsupported)
    }

    fn contact(
        &self,
        pos12: &Pose,
        g1: &dyn Shape,
        g2: &dyn Shape,
        prediction: f32,
    ) -> Result<Option<Contact>, Unsupported> {
        if let (Some(c1), Some(c2)) = (g1.as_capsule(), g2.as_capsule()) {
            let (b1, b2, balls12, centre1, centre2) =
                capsules_as_balls(pos12, c1, c2).ok_or(Unsupported)?;
            return Ok(
                contact_ball_ball(&balls12, &b1, &b2, prediction).map(|contact| Contact {
                    point1: contact.point1 + centre1,
                    point2: contact.point2 + centre2,
                    ..contact
                }),
            );
        }
        let (pair, box_first) = capsule_and_box(pos12, g1, g2).ok_or(Unsupported)?;
        let (on_box, normal, distance) = pair.nearest(pair.centre);
        let dist = distance - pair.radius;
        if dist > prediction {
            return Ok(None);
        }
        // `normal` points from the box towards the capsule; the capsule's frame is centred
        // on its middle, at the box frame's height `pair.centre.y`.
        let on_capsule = -normal * pair.radius;
        Ok(Some(if box_first {
            Contact::new(on_box, on_capsule, normal, -normal, dist)
        } else {
            Contact::new(on_capsule, on_box, -normal, normal, dist)
        }))
    }

    fn closest_points(
        &self,
        _: &Pose,
        _: &dyn Shape,
        _: &dyn Shape,
        _: f32,
    ) -> Result<ClosestPoints, Unsupported> {
        Err(Unsupported)
    }

    fn cast_shapes(
        &self,
        pos12: &Pose,
        local_vel12: Vector,
        g1: &dyn Shape,
        g2: &dyn Shape,
        options: ShapeCastOptions,
    ) -> Result<Option<ShapeCastHit>, Unsupported> {
        if local_vel12.y != 0.0 {
            return Err(Unsupported);
        }
        if let (Some(c1), Some(c2)) = (g1.as_capsule(), g2.as_capsule()) {
            let (b1, b2, balls12, centre1, centre2) =
                capsules_as_balls(pos12, c1, c2).ok_or(Unsupported)?;
            return Ok(
                cast_shapes_ball_ball(&balls12, local_vel12, &b1, &b2, options).map(|hit| {
                    ShapeCastHit {
                        witness1: hit.witness1 + centre1,
                        witness2: hit.witness2 + centre2,
                        ..hit
                    }
                }),
            );
        }
        let (pair, box_first) = capsule_and_box(pos12, g1, g2).ok_or(Unsupported)?;
        // The capsule's motion relative to the box.
        let velocity = if box_first { local_vel12 } else { -local_vel12 };
        let reach = pair.radius + options.target_distance;
        let (_, normal, distance) = pair.nearest(pair.centre);
        let (time_of_impact, status) = if distance <= reach {
            let status = if distance < reach {
                ShapeCastStatus::PenetratingOrWithinTargetDist
            } else {
                ShapeCastStatus::Converged
            };
            (0.0, status)
        } else {
            match pair.time_of_impact(velocity, reach, options.max_time_of_impact) {
                Some(time) => (time, ShapeCastStatus::Converged),
                None => return Ok(None),
            }
        };
        let at = pair.centre + velocity * time_of_impact;
        let (on_box, normal_at, _) = pair.nearest(at);
        let normal = if time_of_impact == 0.0 {
            normal
        } else {
            normal_at
        };
        // Leaving or sliding along what it touches is no impact.
        if !options.stop_at_penetration
            && time_of_impact < SMALL_TIME_OF_IMPACT
            && normal.dot(velocity) >= 0.0
        {
            return Ok(None);
        }
        let on_capsule = -normal * pair.radius;
        Ok(Some(if box_first {
            ShapeCastHit {
                time_of_impact,
                witness1: on_box,
                witness2: on_capsule,
                normal1: normal,
                normal2: -normal,
                status,
                subshape1: 0,
                subshape2: 0,
            }
        } else {
            ShapeCastHit {
                time_of_impact,
                witness1: on_capsule,
                witness2: on_box,
                normal1: -normal,
                normal2: normal,
                status,
                subshape1: 0,
                subshape2: 0,
            }
        }))
    }

    fn cast_shapes_nonlinear(
        &self,
        _: &NonlinearRigidMotion,
        _: &dyn Shape,
        _: &NonlinearRigidMotion,
        _: &dyn Shape,
        _: f32,
        _: f32,
        _: bool,
    ) -> Result<Option<ShapeCastHit>, Unsupported> {
        Err(Unsupported)
    }
}
