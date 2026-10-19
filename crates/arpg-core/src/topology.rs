//! Walkable topology (#207, ADR 0002): stable walkable regions, typed traversal connections
//! between them, and each actor's region membership or transit.
//!
//! The physics backend owns physical height and support; this module owns the semantic
//! connectivity gameplay rules read. Ordinary combat, targeting and interaction are
//! region-local by default ([`Topology::local`]). Until elevation exists, a level is one region
//! per flat floor (Atlas 5.1.2): the generated dungeon is a single region, and its doors are
//! gated passages inside it, not region boundaries.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::GameError;

pub type RegionId = u32;
pub type ConnectionId = u32;

/// A connected walkable surface. It may later hold slopes, stairs and uneven ground; until
/// elevation exists every region is a flat floor at `foot_y`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WalkableRegion {
    pub id: RegionId,
    /// Height (world units) at which a body supported on this floor has its feet.
    pub foot_y: i32,
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
    /// Validated topology: at least one region, unique region and connection ids, and every
    /// connection between two distinct existing regions.
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

    fn connection(&self, id: ConnectionId) -> Option<&TraversalConnection> {
        self.connections
            .iter()
            .find(|connection| connection.id == id)
    }

    /// Whether `membership` names a region or connection of this topology.
    pub fn knows(&self, membership: RegionMembership) -> bool {
        match membership {
            RegionMembership::Region { id } => self.regions.iter().any(|region| region.id == id),
            RegionMembership::Transit { connection } => self.connection(connection).is_some(),
        }
    }

    /// The region whose floor supports a body with its feet at `feet_y`, within `tolerance`
    /// world units; the lowest id wins a tie. `None` on undeclared walkable geometry.
    pub fn supporting(&self, feet_y: f32, tolerance: f32) -> Option<RegionId> {
        self.regions
            .iter()
            .filter(|region| (feet_y - region.foot_y as f32).abs() <= tolerance)
            .map(|region| region.id)
            .min()
    }

    /// Membership after a body ends supported with its feet at `feet_y`: the supporting region,
    /// which also ends a transit. Undeclared support keeps the membership, since geometry must
    /// not create connectivity the topology does not declare (ADR 0002).
    pub fn settle(
        &self,
        membership: RegionMembership,
        feet_y: f32,
        tolerance: f32,
    ) -> RegionMembership {
        self.supporting(feet_y, tolerance)
            .map_or(membership, |id| RegionMembership::Region { id })
    }

    /// The regions an actor counts as present in: its region, or both ends of the connection
    /// it is traversing, so a body in transit stays reachable from either side.
    fn presence(&self, membership: RegionMembership) -> [Option<RegionId>; 2] {
        match membership {
            RegionMembership::Region { id } => [Some(id), None],
            RegionMembership::Transit { connection } => self
                .connection(connection)
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
        WalkableRegion { id, foot_y }
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

    #[test]
    fn invalid_topologies_are_rejected() {
        assert!(Topology::new(vec![], vec![]).is_err());
        assert!(Topology::new(vec![region(1, 0), region(1, 100)], vec![]).is_err());
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
    fn the_generated_dungeon_is_one_floor_region_and_everything_on_it_is_local() {
        let topology = Topology::single_floor(0);
        assert_eq!(topology.base(), DUNGEON_FLOOR_REGION);
        assert!(topology.connections().is_empty());
        assert!(topology.local(at(1), at(1)));
        assert_eq!(topology.supporting(3.0, 6.0), Some(1));
        // Airborne or on undeclared geometry: membership stays.
        assert_eq!(topology.supporting(40.0, 6.0), None);
        assert_eq!(topology.settle(at(1), 40.0, 6.0), at(1));
    }

    #[test]
    fn transit_is_local_to_both_ends_and_landing_ends_it() {
        let topology = Topology::new(
            vec![region(1, 0), region(2, 200), region(3, 400)],
            vec![connection(5, ConnectionKind::Drop, 2, 1)],
        )
        .unwrap();
        let transit = RegionMembership::Transit { connection: 5 };
        assert!(topology.knows(transit));
        assert!(!topology.knows(RegionMembership::Transit { connection: 6 }));
        assert!(!topology.local(at(1), at(2)));
        assert!(topology.local(transit, at(1)));
        assert!(topology.local(at(2), transit));
        assert!(!topology.local(transit, at(3)));
        assert_eq!(topology.settle(transit, 1.0, 6.0), at(1));
    }
}
