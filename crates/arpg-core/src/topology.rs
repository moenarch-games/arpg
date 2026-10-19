//! Walkable topology (#207, #210, ADR 0002): stable walkable regions, typed traversal
//! connections between them, and each actor's region membership or transit.
//!
//! The physics backend owns physical height and support; this module owns the semantic
//! connectivity gameplay rules read. Ordinary combat, targeting and interaction are
//! region-local by default ([`Topology::local`]). The generated dungeon is one region per flat
//! floor (Atlas 5.1.2): a single region whose doors are gated passages inside it. Authored
//! elevation (platforms, bridges) adds regions with their own floor height and area, which may
//! overlap others vertically: a bridge above a path.
//!
//! An airborne actor keeps its region until it traverses a declared connection: it enters
//! transit when it rises over a higher region it could land on, or leaves its own region's
//! area, and lands in the region whose floor supports it. A level whose geometry permits a
//! crossing no connection declares is invalid ([`Topology::validate_crossings`]).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::GameError;

pub type RegionId = u32;
pub type ConnectionId = u32;

/// A region's horizontal extent (world units, inclusive): its walkable surface. A body whose
/// centre is up to its radius beyond it still stands on it, rounding the edge.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegionArea {
    pub min_x: i32,
    pub max_x: i32,
    pub min_z: i32,
    pub max_z: i32,
}

impl RegionArea {
    fn contains(self, x: f32, z: f32, margin: f32) -> bool {
        (self.min_x as f32 - margin..=self.max_x as f32 + margin).contains(&x)
            && (self.min_z as f32 - margin..=self.max_z as f32 + margin).contains(&z)
    }

    /// Horizontal gap to `other` (0 when they touch or overlap).
    fn gap(self, other: Self) -> f32 {
        let axis = |low: i32, high: i32, other_low: i32, other_high: i32| {
            (other_low - high).max(low - other_high).max(0) as f32
        };
        axis(self.min_x, self.max_x, other.min_x, other.max_x).hypot(axis(
            self.min_z,
            self.max_z,
            other.min_z,
            other.max_z,
        ))
    }
}

/// A connected walkable surface at `foot_y`, over `area` (`None`: the whole level, as the
/// dungeon floor is).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WalkableRegion {
    pub id: RegionId,
    /// Height (world units) at which a body supported on this floor has its feet.
    pub foot_y: i32,
    pub area: Option<RegionArea>,
}

impl WalkableRegion {
    fn contains(&self, x: f32, z: f32, margin: f32) -> bool {
        self.area.is_none_or(|area| area.contains(x, z, margin))
    }

    fn gap(&self, other: &Self) -> f32 {
        match (self.area, other.area) {
            (Some(area), Some(other)) => area.gap(other),
            _ => 0.0,
        }
    }
}

/// The traversal a connection permits.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionKind {
    /// Walking or a ramp, both ways.
    Walk,
    /// A jump, both ways.
    Jump,
    /// A one-way drop from `from` to `to`.
    Drop,
    /// A door or another gated passage, both ways.
    Gated,
}

/// A typed connection between two distinct regions.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TraversalConnection {
    pub id: ConnectionId,
    pub kind: ConnectionKind,
    pub from: RegionId,
    pub to: RegionId,
}

impl TraversalConnection {
    /// Whether it permits going from `a` to `b`.
    fn leads(&self, a: RegionId, b: RegionId) -> bool {
        (self.from == a && self.to == b)
            || (self.kind != ConnectionKind::Drop && self.from == b && self.to == a)
    }
}

/// Where an actor belongs: a region, or the connection it is traversing. Ordinary airborne
/// movement inside one region keeps its membership; transit is neither invulnerability nor an
/// action cancellation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum RegionMembership {
    Region { id: RegionId },
    Transit { connection: ConnectionId },
}

/// A level's walkable regions and the typed connections between them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Topology {
    regions: Vec<WalkableRegion>,
    connections: Vec<TraversalConnection>,
}

/// The id of the generated dungeon's single floor region.
pub const DUNGEON_FLOOR_REGION: RegionId = 1;

impl Topology {
    /// Validated topology: at least one region, unique region and connection ids, sound areas,
    /// and every connection between two distinct existing regions.
    pub fn new(
        regions: Vec<WalkableRegion>,
        connections: Vec<TraversalConnection>,
    ) -> Result<Self, GameError> {
        let invalid =
            |reason: String| GameError::new(format!("invalid walkable topology: {reason}"));
        if regions.is_empty() {
            return Err(invalid("a level needs a walkable region".into()));
        }
        let mut ids = BTreeSet::new();
        for region in &regions {
            if !ids.insert(region.id) {
                return Err(invalid(format!("region {} is declared twice", region.id)));
            }
            if region
                .area
                .is_some_and(|area| area.min_x > area.max_x || area.min_z > area.max_z)
            {
                return Err(invalid(format!("region {} has an empty area", region.id)));
            }
        }
        let mut connection_ids = BTreeSet::new();
        for connection in &connections {
            if !connection_ids.insert(connection.id) {
                return Err(invalid(format!(
                    "connection {} is declared twice",
                    connection.id
                )));
            }
            if connection.from == connection.to {
                return Err(invalid(format!(
                    "connection {} joins region {} to itself",
                    connection.id, connection.from
                )));
            }
            for end in [connection.from, connection.to] {
                if !ids.contains(&end) {
                    return Err(invalid(format!(
                        "connection {} references unknown region {end}",
                        connection.id
                    )));
                }
            }
        }
        Ok(Self {
            regions,
            connections,
        })
    }

    /// One flat floor whose bodies stand with their feet at `foot_y`, without connections
    /// (Atlas 5.1.2).
    pub fn single_floor(foot_y: i32) -> Self {
        Self {
            regions: vec![WalkableRegion {
                id: DUNGEON_FLOOR_REGION,
                foot_y,
                area: None,
            }],
            connections: Vec::new(),
        }
    }

    pub fn regions(&self) -> &[WalkableRegion] {
        &self.regions
    }

    pub fn connections(&self) -> &[TraversalConnection] {
        &self.connections
    }

    /// Rejects a level whose geometry permits a crossing no connection declares (ADR 0002):
    /// from every region to every other one a body can reach (a rise of at most `rise` over
    /// a horizontal gap of at most `reach` world units, or any drop within that reach).
    pub fn validate_crossings(&self, rise: f32, reach: f32) -> Result<(), GameError> {
        for from in &self.regions {
            for to in &self.regions {
                let reachable = from.id != to.id
                    && (to.foot_y - from.foot_y) as f32 <= rise
                    && from.gap(to) <= reach;
                if reachable && self.connection(from.id, to.id).is_none() {
                    return Err(GameError::new(format!(
                        "invalid walkable topology: region {} can reach region {} without a \
                         declared connection",
                        from.id, to.id
                    )));
                }
            }
        }
        Ok(())
    }

    /// The highest walkable floor's foot height (world units).
    pub fn highest_foot(&self) -> i32 {
        self.regions
            .iter()
            .map(|region| region.foot_y)
            .max()
            .unwrap_or(0)
    }

    /// The lowest walkable floor's foot height (world units).
    pub fn lowest_foot(&self) -> i32 {
        self.regions
            .iter()
            .map(|region| region.foot_y)
            .min()
            .unwrap_or(0)
    }

    /// The base floor: where planar actors (monsters), loot and chests belong.
    pub fn base(&self) -> RegionId {
        self.regions[0].id
    }

    fn region(&self, id: RegionId) -> Option<&WalkableRegion> {
        self.regions.iter().find(|region| region.id == id)
    }

    fn connection_by_id(&self, id: ConnectionId) -> Option<&TraversalConnection> {
        self.connections
            .iter()
            .find(|connection| connection.id == id)
    }

    /// The lowest-id connection that permits going from `a` to `b`.
    fn connection(&self, a: RegionId, b: RegionId) -> Option<&TraversalConnection> {
        self.preferred(a, b, None)
    }

    /// The connection from `a` to `b` of kind `kind` if one exists, else any; lowest id first.
    fn preferred(
        &self,
        a: RegionId,
        b: RegionId,
        kind: Option<ConnectionKind>,
    ) -> Option<&TraversalConnection> {
        self.connections
            .iter()
            .filter(|connection| connection.leads(a, b))
            .min_by_key(|connection| (Some(connection.kind) != kind, connection.id))
    }

    /// Whether `membership` names a region or connection of this topology.
    pub fn knows(&self, membership: RegionMembership) -> bool {
        match membership {
            RegionMembership::Region { id } => self.region(id).is_some(),
            RegionMembership::Transit { connection } => self.connection_by_id(connection).is_some(),
        }
    }

    /// The region whose floor supports a body of `radius` at (`x`, `z`) with its feet at
    /// `feet_y`, within `tolerance` world units; the lowest id wins a tie. `None` on undeclared
    /// walkable geometry.
    pub fn supporting(
        &self,
        x: f32,
        z: f32,
        feet_y: f32,
        tolerance: f32,
        radius: f32,
    ) -> Option<RegionId> {
        self.regions
            .iter()
            .filter(|region| {
                (feet_y - region.foot_y as f32).abs() <= tolerance && region.contains(x, z, radius)
            })
            .map(|region| region.id)
            .min()
    }

    /// Membership after a body ends supported: the supporting region, which also ends a
    /// transit. Undeclared support keeps the membership, since geometry must not create
    /// connectivity the topology does not declare.
    pub fn settle(
        &self,
        membership: RegionMembership,
        x: f32,
        z: f32,
        feet_y: f32,
        tolerance: f32,
        radius: f32,
    ) -> RegionMembership {
        self.supporting(x, z, feet_y, tolerance, radius)
            .map_or(membership, |id| RegionMembership::Region { id })
    }

    /// Membership after a tick that ends airborne. A body in a region enters transit along
    /// the declared connection to a higher region it has risen over (within `tolerance` of
    /// that floor or above), or to the highest lower region below it once its centre has left
    /// its own region's area. Otherwise, and in transit, the membership stays. Areas count
    /// without a body's margin here: by the time a body rounding an edge falls free, its
    /// centre is well past the area, so the transit does not hinge on sub-unit differences.
    pub fn airborne(
        &self,
        membership: RegionMembership,
        x: f32,
        z: f32,
        feet_y: f32,
        tolerance: f32,
    ) -> RegionMembership {
        let RegionMembership::Region { id } = membership else {
            return membership;
        };
        let Some(current) = self.region(id) else {
            return membership;
        };
        let rising = self
            .regions
            .iter()
            .filter(|region| {
                region.foot_y > current.foot_y
                    && region.contains(x, z, 0.0)
                    && feet_y >= region.foot_y as f32 - tolerance
            })
            .min_by_key(|region| region.id);
        let falling = || {
            if current.contains(x, z, 0.0) {
                return None;
            }
            self.regions
                .iter()
                .filter(|region| region.foot_y < current.foot_y && region.contains(x, z, 0.0))
                .max_by_key(|region| (region.foot_y, std::cmp::Reverse(region.id)))
        };
        rising
            .map(|target| (target, ConnectionKind::Jump))
            .or_else(|| falling().map(|target| (target, ConnectionKind::Drop)))
            .and_then(|(target, kind)| self.preferred(id, target.id, Some(kind)))
            .map_or(membership, |connection| RegionMembership::Transit {
                connection: connection.id,
            })
    }

    /// The regions an actor counts as present in: its region, or both ends of the connection
    /// it is traversing, so a body in transit stays reachable from either side.
    fn presence(&self, membership: RegionMembership) -> [Option<RegionId>; 2] {
        match membership {
            RegionMembership::Region { id } => [Some(id), None],
            RegionMembership::Transit { connection } => self
                .connection_by_id(connection)
                .map_or([None, None], |connection| {
                    [Some(connection.from), Some(connection.to)]
                }),
        }
    }

    /// Whether two actors share a region: the default eligibility of ordinary combat,
    /// targeting and interaction. Abilities that opt into cross-region play skip this.
    pub fn local(&self, a: RegionMembership, b: RegionMembership) -> bool {
        let (a, b) = (self.presence(a), self.presence(b));
        a.iter()
            .flatten()
            .any(|region| b.iter().flatten().any(|other| other == region))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(id: RegionId, foot_y: i32) -> WalkableRegion {
        WalkableRegion {
            id,
            foot_y,
            area: None,
        }
    }

    fn raised(id: RegionId, foot_y: i32, min_x: i32, max_x: i32) -> WalkableRegion {
        WalkableRegion {
            id,
            foot_y,
            area: Some(RegionArea {
                min_x,
                max_x,
                min_z: -100,
                max_z: 100,
            }),
        }
    }

    fn connection(
        id: ConnectionId,
        kind: ConnectionKind,
        from: RegionId,
        to: RegionId,
    ) -> TraversalConnection {
        TraversalConnection { id, kind, from, to }
    }

    fn at(id: RegionId) -> RegionMembership {
        RegionMembership::Region { id }
    }

    /// A floor, a platform (60 up, x 0..200) and a bridge (140 up, x 200..500) above the
    /// floor, as the traversal scenario lays them.
    fn traversal(connections: Vec<TraversalConnection>) -> Topology {
        Topology::new(
            vec![
                region(1, 0),
                raised(2, 60, 0, 200),
                raised(3, 140, 200, 500),
            ],
            connections,
        )
        .unwrap()
    }

    fn declared() -> Vec<TraversalConnection> {
        vec![
            connection(1, ConnectionKind::Jump, 1, 2),
            connection(2, ConnectionKind::Drop, 2, 1),
            connection(3, ConnectionKind::Jump, 2, 3),
            connection(4, ConnectionKind::Drop, 3, 1),
        ]
    }

    #[test]
    fn invalid_topologies_are_rejected() {
        assert!(Topology::new(vec![], vec![]).is_err());
        assert!(Topology::new(vec![region(1, 0), region(1, 100)], vec![]).is_err());
        assert!(Topology::new(vec![region(1, 0), raised(2, 60, 10, 0)], vec![]).is_err());
        let two = || vec![region(1, 0), region(2, 100)];
        for connections in [
            vec![connection(1, ConnectionKind::Jump, 1, 1)],
            vec![connection(1, ConnectionKind::Jump, 1, 3)],
            vec![
                connection(1, ConnectionKind::Drop, 2, 1),
                connection(1, ConnectionKind::Walk, 1, 2),
            ],
        ] {
            assert!(Topology::new(two(), connections).is_err());
        }
        assert!(Topology::new(two(), vec![connection(7, ConnectionKind::Drop, 2, 1)]).is_ok());
    }

    #[test]
    fn an_undeclared_crossing_the_geometry_permits_is_rejected() {
        assert!(
            traversal(declared())
                .validate_crossings(90.0, 300.0)
                .is_ok()
        );
        // Without the jump between platform and bridge (80 apart, within a 90 rise).
        let mut missing = declared();
        missing.retain(|connection| connection.id != 3);
        let error = traversal(missing)
            .validate_crossings(90.0, 300.0)
            .unwrap_err();
        assert!(
            error.message().contains("region 2 can reach region 3"),
            "{error:?}"
        );
        // Without the drop from the bridge to the floor below it.
        let mut missing = declared();
        missing.retain(|connection| connection.id != 4);
        assert!(traversal(missing).validate_crossings(90.0, 300.0).is_err());
        // The bridge is out of reach from the floor, so no connection is needed there.
        assert!(traversal(declared()).connection(1, 3).is_none());
    }

    #[test]
    fn the_generated_dungeon_is_one_floor_region_and_everything_on_it_is_local() {
        let topology = Topology::single_floor(0);
        assert_eq!(topology.base(), DUNGEON_FLOOR_REGION);
        assert!(topology.connections().is_empty());
        assert!(topology.validate_crossings(90.0, 300.0).is_ok());
        assert!(topology.local(at(1), at(1)));
        assert_eq!(topology.supporting(0.0, 0.0, 3.0, 6.0, 0.0), Some(1));
        assert_eq!(topology.supporting(0.0, 0.0, 40.0, 6.0, 0.0), None);
        assert_eq!(topology.settle(at(1), 0.0, 0.0, 40.0, 6.0, 0.0), at(1));
        assert_eq!(topology.airborne(at(1), 0.0, 0.0, 90.0, 6.0), at(1));
    }

    #[test]
    fn overlapping_regions_are_told_apart_by_height() {
        let topology = traversal(declared());
        // Under the bridge, on the floor; on top of it, the bridge.
        assert_eq!(topology.supporting(300.0, 0.0, 0.0, 6.0, 0.0), Some(1));
        assert_eq!(topology.supporting(300.0, 0.0, 140.0, 6.0, 0.0), Some(3));
        assert!(!topology.local(at(1), at(3)));
    }

    #[test]
    fn rising_over_a_higher_region_or_leaving_one_enters_transit() {
        let topology = traversal(declared());
        // A jump on the floor, beside the platform or below its top: still the floor.
        assert_eq!(topology.airborne(at(1), -50.0, 0.0, 80.0, 6.0), at(1));
        assert_eq!(topology.airborne(at(1), 50.0, 0.0, 40.0, 6.0), at(1));
        // Above the platform's top over it: in transit along the jump.
        let up = RegionMembership::Transit { connection: 1 };
        assert_eq!(topology.airborne(at(1), 50.0, 0.0, 70.0, 6.0), up);
        assert_eq!(topology.airborne(up, -50.0, 0.0, 10.0, 6.0), up);
        assert_eq!(topology.settle(up, 50.0, 0.0, 60.0, 6.0, 0.0), at(2));
        // A jump on the platform stays; leaving its area drops towards the floor.
        assert_eq!(topology.airborne(at(2), 100.0, 0.0, 120.0, 6.0), at(2));
        assert_eq!(
            topology.airborne(at(2), -40.0, 0.0, 50.0, 6.0),
            RegionMembership::Transit { connection: 2 }
        );
        // From the platform up onto the bridge, and off its side down to the floor.
        assert_eq!(
            topology.airborne(at(2), 210.0, 0.0, 141.0, 6.0),
            RegionMembership::Transit { connection: 3 }
        );
        let off = topology.airborne(at(3), 300.0, 150.0, 130.0, 6.0);
        assert_eq!(off, RegionMembership::Transit { connection: 4 });
        assert!(topology.local(off, at(1)) && topology.local(off, at(3)));
        assert!(!topology.local(off, at(2)));
        assert_eq!(topology.settle(off, 300.0, 150.0, 0.0, 6.0, 0.0), at(1));
    }
}
