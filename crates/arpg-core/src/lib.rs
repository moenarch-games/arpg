#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::sync::{Arc, LazyLock};

use physics::{BodyKey, Physics, box_hit, vector};
use rapier3d::math::Vector;
use serde::{Deserialize, Serialize};

use content::{
    BowData, Content, EncounterActivation, EncounterCompletion, EncounterDefinition, GuardData,
    JumpTuning, MonsterDefinition, StrikeDefinition, content,
};
pub use content::{
    CONTENT_FORMAT_VERSION, ContentBundle, ContentError, DefinitionId, MAX_ENCOUNTER_SPAWNS,
    base_bundle, content_revision,
};
use navigation::{Cell, FieldRoute, FieldWork, NAV_CELL_SIZE, Rect, RoomGrid, TargetField};
pub use topology::{
    ConnectionId, ConnectionKind, DUNGEON_FLOOR_REGION, RegionArea, RegionId, RegionMembership,
    Topology, TraversalConnection, WalkableRegion,
};
pub use workbench::{
    MAX_WORKBENCH_MONSTERS, MAX_WORKBENCH_SPAWNS, TuningParameter, TuningValue, WorkbenchOperation,
};
use workbench::{Tuning, Workbench};

mod content;
mod navigation;
mod operation_work;
mod physics;
#[cfg(feature = "physics-ab")]
pub mod physics_ab;
#[cfg(any(test, feature = "physics-ab"))]
mod seeded_workloads;
mod topology;
mod workbench;
use operation_work::{Counter, count};
#[cfg(feature = "operation-work")]
pub use operation_work::{OperationWork, WorkFixture, contract::check_work_contract};

/// A position or extent in whole world units: level layout and snapshots. Authoritative
/// positions and velocities are `f32` ([`Vector`]) and gameplay rules read them directly;
/// only snapshots round them, through [`Vec3i::rounded`] (Atlas 2.3.2, #182).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct Vec3i {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl Vec3i {
    pub const ZERO: Self = Self::new(0, 0, 0);

    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// The nearest whole world-unit point to a physical position, for snapshots only.
    pub(crate) fn rounded(position: Vector) -> Self {
        // Positions lie within the dungeon, far inside `i32`.
        Self::new(
            position.x.round() as i32,
            position.y.round() as i32,
            position.z.round() as i32,
        )
    }
}

impl From<Vec3i> for Vector {
    fn from(value: Vec3i) -> Self {
        vector(value)
    }
}

pub type DoorId = u64;
pub type GroundLootId = u64;
pub type PlayerId = u32;
pub type RoomId = u32;
pub type RunSeed = u32;
pub const TICK_HZ: u16 = 60;
pub const MAX_PLAYERS: usize = 4;
pub const WORLD_UNITS_PER_METER: i32 = 100;
/// Largest magnitude of either component of an aim direction. Aim is a direction only: its
/// length carries no meaning, so `[1, 0]` and `[1000, 0]` aim the same way.
pub const AIM_COMPONENT_LIMIT: i16 = 1_000;
/// 13: Rapier physics (#175): `f32` positions and velocities; schema 12 saves are refused.
/// 14: gameplay rules on `f32` positions (#182): projectiles, ground loot and monster posts
/// are `f32` game state; schema 13 saves are refused.
/// 15: jumping and support (#59): a player's vertical velocity, grounded state, coyote and
/// jump-buffer ticks continue exactly after a load; schema 14 saves are refused.
/// 16: walkable topology (#207): a player's region membership or transit; schema 15 saves
/// are refused.
/// 17: fall recovery (#211): a player's last supported position; schema 16 saves are refused.
pub const SAVE_STATE_SCHEMA_VERSION: u16 = 17;
// 2: directional multi-target strike volumes and obstruction by fixed geometry.
// 3: directional shield guard, block and guard break.
// 4: post-block counterattack opportunity.
// 5: authored light/heavy combo transitions.
// 6: bow loadout, draw/release and authoritative arrows.
// 7: reward chests, line-of-sight interaction and reasoned interaction results.
// 8: enemy engagement: aggro, leash and return, target hysteresis, separation (#109).
// 9: aim intent separate from movement and optional target lock (#103).
// 10: ranged spacing and monster projectiles on the shared arrow path (#108).
// 11: shield monsters guard on the shared guard path (#92).
// 12: jumping, gravity and grounded-only guard and counter (#59).
pub const SAVE_STATE_RULES_VERSION: u16 = 12;
// Velocities are world units per simulation tick. At 60 Hz and 100 world units per
// meter, 7 units/tick is 4.2 m/s.
const PLAYER_SPEED: i32 = 7;
const PLAYER_DIAGONAL_SPEED: i32 = 5;
// Ground control converges on player intent over a few 60 Hz ticks, starting from the
// velocity physics resolved in the previous tick rather than replacing it. Airborne control
// is content (`jump`).
const PLAYER_ACCELERATION_PER_TICK: i32 = 3;
const PLAYER_BRAKING_PER_TICK: i32 = 5;
const PLAYER_REVERSAL_PER_TICK: i32 = 5;
const PLAYER_BODY_BASE: u64 = 1_000;
const STATIC_BODY_BASE: u64 = 10_000;
const DOOR_BODY_BASE: u64 = 20_000;
const GROUND_LOOT_ID_BASE: u64 = 30_000;
const PLAYER_HALF_EXTENTS: Vec3i = Vec3i::new(30, 50, 30);
const PLAYER_Y: i32 = 50;
/// Height of a grounded body's lowest point: the flat floor's walking height (#59).
const FLOOR_FOOT_Y: i32 = PLAYER_Y - PLAYER_HALF_EXTENTS.y;
/// How far (world units) a saved grounded player may stand from the plane: Rapier settles a
/// landing body within its support reach, about five units, and to `f32` resolution after.
const GROUNDED_HEIGHT_TOLERANCE: f32 = 6.0;
#[cfg(test)]
const COYOTE_TICKS: u8 = 6;
#[cfg(test)]
const ATTACK_RANGE: i64 = 220;
#[cfg(test)]
const SECONDARY_ATTACK_RANGE: i64 = 150;
#[cfg(test)]
const SECONDARY_ATTACK_DAMAGE_NUMERATOR: u16 = 3;
#[cfg(test)]
const SECONDARY_ATTACK_DAMAGE_DENOMINATOR: u16 = 2;
#[cfg(test)]
const PRIMARY_WINDUP_TICKS: u8 = 5;
#[cfg(test)]
const PRIMARY_ACTIVE_TICKS: u8 = 1;
#[cfg(test)]
const PRIMARY_RECOVERY_TICKS: u8 = 8;
#[cfg(test)]
const SECONDARY_WINDUP_TICKS: u8 = 10;
#[cfg(test)]
const SECONDARY_ACTIVE_TICKS: u8 = 1;
#[cfg(test)]
const SECONDARY_RECOVERY_TICKS: u8 = 14;
#[cfg(test)]
const INTERACT_WINDUP_TICKS: u8 = 2;
#[cfg(test)]
const INTERACT_ACTIVE_TICKS: u8 = 1;
#[cfg(test)]
const INTERACT_RECOVERY_TICKS: u8 = 3;
const INTERACT_RANGE: i64 = 160;
#[cfg(test)]
const GROUND_LOOT_GOLD_AMOUNT: u32 = 10;
/// One reward chest per combat room, opened once after the room's encounter is cleared.
const CHEST_ID_BASE: u64 = 50_000;
#[cfg(test)]
const CHEST_GOLD_AMOUNT: u32 = 25;
#[cfg(test)]
const MONSTER_ATTACK_RANGE: i64 = 180;
#[cfg(test)]
const MONSTER_ATTACK_DAMAGE: u16 = 10;
#[cfg(test)]
const MONSTER_ATTACK_WINDUP_TICKS: u8 = 18;
#[cfg(test)]
const MONSTER_ATTACK_ACTIVE_TICKS: u8 = 1;
#[cfg(test)]
const MONSTER_ATTACK_RECOVERY_TICKS: u8 = 30;
const PLAYER_HURT_TICKS: u8 = 6;
/// Ticks between pressing guard and the shield protecting.
#[cfg(test)]
const GUARD_RAISE_TICKS: u8 = 4;
#[cfg(test)]
const MAX_GUARD_POINTS: u16 = 100;
/// Guard regenerates only while lowered and not broken.
#[cfg(test)]
const GUARD_REGEN_PER_TICK: u16 = 1;
#[cfg(test)]
const GUARD_BLOCK_REACTION_TICKS: u8 = 8;
#[cfg(test)]
const GUARD_BREAK_TICKS: u8 = 45;
#[cfg(test)]
const MONSTER_CLAW_GUARD_COST: u16 = 30;
/// Ticks a successful block keeps the counter opportunity open (initial playtest proposal:
/// about half a second at 60 Hz).
#[cfg(test)]
pub const COUNTER_WINDOW_TICKS: u64 = 30;
#[cfg(test)]
const COUNTER_WINDUP_TICKS: u8 = 3;
#[cfg(test)]
const COUNTER_ACTIVE_TICKS: u8 = 1;
#[cfg(test)]
const COUNTER_RECOVERY_TICKS: u8 = 10;
#[cfg(test)]
const COUNTER_RANGE: i64 = 200;
#[cfg(test)]
const COUNTER_DAMAGE_NUMERATOR: u16 = 2;
#[cfg(test)]
const COUNTER_DAMAGE_DENOMINATOR: u16 = 1;
#[cfg(test)]
const COUNTER_STAGGER_TICKS: u8 = 12;
#[cfg(test)]
const LIGHT_FOLLOW_UP_WINDUP_TICKS: u8 = 4;
#[cfg(test)]
const LIGHT_FOLLOW_UP_ACTIVE_TICKS: u8 = 1;
#[cfg(test)]
const LIGHT_FOLLOW_UP_RECOVERY_TICKS: u8 = 8;
#[cfg(test)]
const LIGHT_FINISHER_WINDUP_TICKS: u8 = 6;
#[cfg(test)]
const LIGHT_FINISHER_ACTIVE_TICKS: u8 = 1;
#[cfg(test)]
const LIGHT_FINISHER_RECOVERY_TICKS: u8 = 16;
#[cfg(test)]
const LIGHT_FINISHER_RANGE: i64 = 240;
#[cfg(test)]
const LIGHT_FINISHER_DAMAGE_NUMERATOR: u16 = 3;
#[cfg(test)]
const LIGHT_FINISHER_DAMAGE_DENOMINATOR: u16 = 2;
#[cfg(test)]
const LIGHT_FINISHER_STAGGER_TICKS: u8 = 10;
#[cfg(test)]
const HEAVY_FINISHER_WINDUP_TICKS: u8 = 8;
#[cfg(test)]
const HEAVY_FINISHER_ACTIVE_TICKS: u8 = 1;
#[cfg(test)]
const HEAVY_FINISHER_RECOVERY_TICKS: u8 = 18;
#[cfg(test)]
const HEAVY_FINISHER_RANGE: i64 = 180;
#[cfg(test)]
const HEAVY_FINISHER_DAMAGE_NUMERATOR: u16 = 2;
#[cfg(test)]
const HEAVY_FINISHER_DAMAGE_DENOMINATOR: u16 = 1;
#[cfg(test)]
const HEAVY_FINISHER_STAGGER_TICKS: u8 = 14;
/// Draw ticks below which a release does not shoot.
#[cfg(test)]
const BOW_MIN_DRAW_TICKS: u8 = 8;
/// Draw ticks at which an arrow reaches full speed and damage.
#[cfg(test)]
const BOW_FULL_DRAW_TICKS: u8 = 30;
#[cfg(test)]
const SHOOT_WINDUP_TICKS: u8 = 1;
#[cfg(test)]
const SHOOT_ACTIVE_TICKS: u8 = 1;
#[cfg(test)]
const SHOOT_RECOVERY_TICKS: u8 = 10;
#[cfg(test)]
const ARROW_MIN_SPEED: i32 = 30;
#[cfg(test)]
const ARROW_FULL_SPEED: i32 = 60;
const ARROW_DIAGONAL_NUMERATOR: i32 = 707;
const ARROW_DIAGONAL_DENOMINATOR: i32 = 1_000;
#[cfg(test)]
const ARROW_MIN_DAMAGE: u16 = 15;
#[cfg(test)]
const ARROW_FULL_DAMAGE: u16 = 35;
#[cfg(test)]
const ARROW_LIFETIME_TICKS: u8 = 40;
#[cfg(test)]
const ARROW_STAGGER_TICKS: u8 = 4;
/// Live arrows are bounded; launching beyond the bound retires the oldest arrow.
#[cfg(test)]
const MAX_LIVE_ARROWS: usize = 32;
const ARROW_ID_BASE: u64 = 1;
/// Monster bodies exist while their monster is alive in an Active room (#65). They sit
/// above every player body (`PLAYER_BODY_BASE + u32`), so no player ID can alias one.
const MONSTER_BODY_BASE: u64 = 1 << 33;
const _: () = assert!(MONSTER_BODY_BASE > PLAYER_BODY_BASE + u32::MAX as u64);
const MONSTER_BODY_HALF_EXTENTS: Vec3i = Vec3i::new(30, 50, 30);
/// Query-only hurt boxes for arrows and shots: gameplay volumes, never physics bodies.
const MONSTER_HURTBOX_HALF_EXTENTS: Vec3i = Vec3i::new(40, 50, 40);
const PLAYER_HURTBOX_HALF_EXTENTS: Vec3i = Vec3i::new(40, 50, 40);
#[cfg(test)]
const PRIMARY_STAGGER_TICKS: u8 = 4;
#[cfg(test)]
const SECONDARY_STAGGER_TICKS: u8 = 8;
#[cfg(test)]
const BASE_ATTACK_DAMAGE: u16 = 25;
#[cfg(test)]
const ATTACK_DAMAGE_PER_LEVEL: u16 = 5;
#[cfg(test)]
const BASE_MAX_HEALTH: u16 = 100;
#[cfg(test)]
const MAX_HEALTH_PER_LEVEL: u16 = 10;
#[cfg(test)]
const EXPERIENCE_PER_LEVEL: u32 = 100;
#[cfg(test)]
const MONSTER_EXPERIENCE_REWARD: u32 = 50;
const DEFAULT_DUNGEON_SEED: RunSeed = 0xA420_0916;
const ARENA_HALF_WIDTH: i32 = 3_000;
const ARENA_HALF_DEPTH: i32 = 1_800;
const WALL_HALF_THICKNESS: i32 = 25;
const WALL_HALF_HEIGHT: i32 = 100;
const DOOR_HALF_WIDTH: i32 = 90;
const PARTITION_MARGIN: i32 = 25;
const DOOR_EDGE_MARGIN: i32 = 250;
const ROOM_SPAWN_MARGIN: i32 = 220;
const PLAYER_SPAWN_OFFSET: i32 = 60;
const ROOM_ACTIVATION_MARGIN: i32 = 30;
/// A returning monster within this distance of its post is home (two navigation cells).
const POST_ARRIVAL_DISTANCE: i64 = 2 * NAV_CELL_SIZE as i64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GameError {
    message: String,
}

impl GameError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for GameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for GameError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlayerCommand<C> {
    pub player_id: PlayerId,
    pub sequence: u32,
    pub command: C,
}

impl<C> PlayerCommand<C> {
    pub fn new(player_id: PlayerId, sequence: u32, command: C) -> Result<Self, GameError> {
        if sequence == 0 {
            return Err(GameError::new("command sequence must be non-zero"));
        }
        Ok(Self {
            player_id,
            sequence,
            command,
        })
    }
}

/// The ordering lane a command arrived on.
///
/// Each lane has its own per-player sequence watermark, so a command is stale only
/// against earlier commands of its own lane. `Datagram` is the default lane: local play,
/// peer play and every caller of [`AuthoritativeGame::apply_command`] use it, and it keeps
/// latest-wins semantics for held state. `Reliable` is the ordered, acknowledged lane that
/// dedicated play uses for one-shot commands. Gameplay rules do not depend on the lane.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum CommandLane {
    Datagram,
    Reliable,
}

/// Transport-neutral authority boundary for one ARPG simulation instance.
///
/// Local play, a browser peer host, and a dedicated `game-server` host all
/// drive the same implementation. Rendering and transport stay outside core.
pub trait AuthoritativeGame: Send + 'static {
    type Command: Send + 'static;
    type Snapshot: Send + 'static;

    fn tick_hz(&self) -> u16;
    fn max_players(&self) -> usize;
    fn current_tick(&self) -> u64;
    fn add_player(&mut self, player_id: PlayerId) -> Result<(), GameError>;
    fn remove_player(&mut self, player_id: PlayerId) -> bool;
    fn apply_command(&mut self, command: PlayerCommand<Self::Command>) -> Result<(), GameError>;
    /// Applies a command delivered on `lane`. The default ignores the lane.
    fn apply_lane_command(
        &mut self,
        lane: CommandLane,
        command: PlayerCommand<Self::Command>,
    ) -> Result<(), GameError> {
        let _ = lane;
        self.apply_command(command)
    }
    fn advance_tick(&mut self) -> Result<(), GameError>;
    fn snapshot(&self) -> Result<Self::Snapshot, GameError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ArpgCommand {
    SetMovement {
        x: i8,
        z: i8,
    },
    /// Light attack. `aim`, when present, is the aim intent the attack commits: it is
    /// validated and applied exactly like `SetAim` before the attack, in the same command,
    /// so the direction never depends on a separately sent (and possibly lost) aim.
    PrimaryAttack {
        #[serde(default)]
        aim: Option<[i16; 2]>,
    },
    /// Heavy attack; `aim` as for `PrimaryAttack`.
    SecondaryAttack {
        #[serde(default)]
        aim: Option<[i16; 2]>,
    },
    Interact,
    /// Held shield input: `raised: true` while the guard button is held.
    SetGuard {
        raised: bool,
    },
    /// Switches the loadout fixture; only while no action is in progress.
    EquipWeapon {
        weapon: Weapon,
    },
    /// Starts drawing the bow (held input).
    DrawBow,
    /// Releases the drawn bow: shoots once if drawn at least the minimum. `aim` as for
    /// `PrimaryAttack`; it is applied even when the release shoots nothing.
    ReleaseBow {
        #[serde(default)]
        aim: Option<[i16; 2]>,
    },
    /// Lowers a drawn bow without shooting (focus loss, menus, explicit cancel).
    CancelBow,
    /// Semantic aim intent (mouse, stick or touch), independent of movement. `Some` points
    /// melee strikes and bow shots along that direction and turns the facing to it; `None`
    /// restores the default of committed facing along movement. Each component must lie
    /// within ±`AIM_COMPONENT_LIMIT` and the direction must be non-zero.
    SetAim {
        direction: Option<[i16; 2]>,
    },
    /// Locks the nearest eligible monster, or moves an existing lock to the next one in
    /// nearest-first order (lower id breaks ties), wrapping around.
    CycleTarget,
    /// Drops the target lock.
    ClearTarget,
    /// A jump press (#59): a one-shot intent. The jump takes off as soon as it is possible
    /// within the content's buffer window: on the ground, or within the coyote window after
    /// leaving it without jumping, and not while an action is committed. Repeated presses
    /// only renew the window; a press never stacks a second jump.
    Jump,
}

impl ArpgCommand {
    /// The aim an aimed one-shot (`PrimaryAttack`, `SecondaryAttack`, `ReleaseBow`) carries
    /// for itself; `None` for every other command and for an unaimed one-shot.
    pub fn embedded_aim(&self) -> Option<[i16; 2]> {
        match *self {
            Self::PrimaryAttack { aim }
            | Self::SecondaryAttack { aim }
            | Self::ReleaseBow { aim } => aim,
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionKind {
    PrimaryAttack,
    SecondaryAttack,
    Interact,
    /// Fast punishing strike available only through a post-block counter opportunity.
    Counter,
    /// Second light strike of the sword combo.
    LightFollowUp,
    /// Third light strike: wider, harder finisher.
    LightFinisher,
    /// Heavy finisher branch after a connecting second light strike.
    HeavyFinisher,
    /// Bow release: launches one arrow when its active phase opens.
    Shoot,
}

/// The core-owned loadout fixture a player fights with. Ammunition is unlimited: this is a
/// labelled training fixture until #69 introduces real equipment and items.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Weapon {
    #[default]
    SwordAndShield,
    Bow,
}

/// A named, deterministic workbench scenario. Every scenario is the normal generated dungeon
/// for its seed plus an exact placement in one combat room: the first one (room 2), or the
/// first room the content gives the scenario's enemy. The player stands at the room centre
/// facing +x and the room's generated monster at an authored offset, with an optional
/// authored pillar. Scenarios never add rules; they only arrange the real runtime.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScenarioId {
    /// The ordinary generated dungeon.
    #[default]
    Dungeon,
    /// A target just outside the monster's own reach but inside the light swing.
    Dummy,
    /// An enemy inside its attack reach, for shield, block and counter work.
    Enemy,
    /// The dummy behind a pillar, for obstructed strikes.
    Obstructed,
    /// A distant target for bow shots.
    Archery,
    /// The distant target behind a pillar, for arrows stopped by walls.
    ArcheryObstructed,
    /// A ranged enemy in its preferred band, shooting and backing off (#108).
    Ranged,
    /// A heavy enemy inside its reach, winding up its long telegraphed slam (#108).
    Heavy,
    /// A ranged enemy inside the light swing's reach and its own retreat range: it backs
    /// away at once, so a strike started now misses once its wind-up ends (#90, #94).
    Retreating,
    /// A platform to jump onto and drop off, its own walkable region (#208), with the
    /// room's enemy waiting on the floor.
    Traversal,
}

impl ScenarioId {
    pub const ALL: [Self; 10] = [
        Self::Dungeon,
        Self::Dummy,
        Self::Enemy,
        Self::Obstructed,
        Self::Archery,
        Self::ArcheryObstructed,
        Self::Ranged,
        Self::Heavy,
        Self::Retreating,
        Self::Traversal,
    ];

    /// The URL/query name, identical to the serialised form.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Dungeon => "dungeon",
            Self::Dummy => "dummy",
            Self::Enemy => "enemy",
            Self::Obstructed => "obstructed",
            Self::Archery => "archery",
            Self::ArcheryObstructed => "archeryObstructed",
            Self::Ranged => "ranged",
            Self::Heavy => "heavy",
            Self::Retreating => "retreating",
            Self::Traversal => "traversal",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|scenario| scenario.name() == name)
    }

    /// Offset of the room's first generated monster from the room centre.
    const fn target_offset(self) -> Option<i32> {
        match self {
            Self::Dungeon => None,
            Self::Dummy | Self::Obstructed => Some(200),
            Self::Enemy => Some(150),
            Self::Archery | Self::ArcheryObstructed | Self::Ranged => Some(500),
            Self::Heavy => Some(150),
            Self::Retreating => Some(RETREATING_TARGET_OFFSET),
            Self::Traversal => Some(-500),
        }
    }

    /// The monster definition whose first room the scenario is arranged in; the others
    /// use the first combat room.
    const fn enemy(self) -> Option<&'static str> {
        match self {
            Self::Ranged | Self::Retreating => Some("monster.archer"),
            Self::Heavy => Some("monster.bruiser"),
            _ => None,
        }
    }

    /// The room the scenario is arranged in: a function of seed and content.
    fn room(self, content: &Content, monsters: &[MonsterState]) -> Result<RoomId, GameError> {
        let Some(enemy) = self.enemy() else {
            return Ok(SCENARIO_ROOM);
        };
        monsters
            .iter()
            .find(|monster| *content.id(monster.definition().id) == *enemy)
            .map(|monster| monster.room_id)
            .ok_or_else(|| GameError::new(format!("no combat room holds a {enemy}")))
    }

    /// Offset of an authored pillar between player and target.
    const fn pillar_offset(self) -> Option<i32> {
        match self {
            Self::Obstructed => Some(110),
            Self::ArcheryObstructed => Some(250),
            _ => None,
        }
    }
}

/// One recorded command of a reproduction: applied before the tick numbered `tick` runs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReproductionCommand {
    pub tick: u64,
    pub player_id: PlayerId,
    pub sequence: u32,
    pub command: ArpgCommand,
}

/// One recorded workbench operation of a reproduction (#126): applied before the tick
/// numbered `tick` runs, after the first `command_index` recorded commands and before the
/// next one.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReproductionOperation {
    pub tick: u64,
    pub command_index: usize,
    pub operation: WorkbenchOperation,
}

/// Portable reproduction input: scenario, seed, players, the exact accepted command
/// sequence and the workbench operations interleaved with it. `replay_reproduction` runs
/// it headlessly on the normal runtime.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Reproduction {
    pub scenario: ScenarioId,
    pub seed: RunSeed,
    pub players: Vec<PlayerId>,
    pub ticks: u64,
    pub commands: Vec<ReproductionCommand>,
    /// Omitted when empty, so reproductions without workbench operations keep their form.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<ReproductionOperation>,
}

/// Maximum ticks a reproduction may request (ten minutes at 60 Hz).
pub const MAX_REPRODUCTION_TICKS: u64 = 36_000;

/// Maximum commands a reproduction may carry: four per tick of the longest reproduction,
/// enough for continuous movement and aim changes.
pub const MAX_REPRODUCTION_COMMANDS: usize = 4 * 36_000;

/// Maximum workbench operations a reproduction may carry.
pub const MAX_REPRODUCTION_OPERATIONS: usize = 10_000;

/// Replays `reproduction` headlessly and returns the snapshot after every tick.
pub fn replay_reproduction(reproduction: &Reproduction) -> Result<Vec<ArpgSnapshot>, GameError> {
    if reproduction.ticks > MAX_REPRODUCTION_TICKS {
        return Err(GameError::new(
            "reproduction is longer than the supported bound",
        ));
    }
    if reproduction.commands.len() > MAX_REPRODUCTION_COMMANDS {
        return Err(GameError::new(
            "reproduction has more commands than the supported bound",
        ));
    }
    if reproduction.operations.len() > MAX_REPRODUCTION_OPERATIONS {
        return Err(GameError::new(
            "reproduction has more workbench operations than the supported bound",
        ));
    }
    if reproduction
        .commands
        .windows(2)
        .any(|pair| pair[0].tick > pair[1].tick)
    {
        return Err(GameError::new(
            "reproduction commands must be in tick order",
        ));
    }
    let mut game = ArpgGame::new_scenario(reproduction.scenario, reproduction.seed)?;
    for &player in &reproduction.players {
        game.add_player(player)?;
    }
    let mut commands = reproduction.commands.iter().peekable();
    let mut operations = reproduction.operations.iter().peekable();
    let mut applied_commands = 0;
    let mut snapshots = Vec::with_capacity(usize::try_from(reproduction.ticks).unwrap_or(0));
    for tick in 0..reproduction.ticks {
        loop {
            if let Some(recorded) = operations.next_if(|recorded| {
                recorded.tick == tick && recorded.command_index == applied_commands
            }) {
                game.apply_workbench(&recorded.operation)?;
            } else if let Some(recorded) = commands.next_if(|recorded| recorded.tick == tick) {
                game.apply_command(PlayerCommand::new(
                    recorded.player_id,
                    recorded.sequence,
                    recorded.command,
                )?)?;
                applied_commands += 1;
            } else {
                break;
            }
        }
        game.advance_tick()?;
        snapshots.push(game.snapshot()?);
    }
    if commands.next().is_some() {
        return Err(GameError::new(
            "reproduction has commands after its last tick",
        ));
    }
    if operations.next().is_some() {
        return Err(GameError::new(
            "reproduction has workbench operations out of order or after its last tick",
        ));
    }
    Ok(snapshots)
}

/// Records a local session as the portable [`Reproduction`] that [`replay_reproduction`]
/// consumes. The caller reports each operation the authority *accepted*, in order; a
/// rejected command never reaches the recording. Recording stops (and export fails with
/// the reason) when the session leaves what a reproduction can express: a player joining
/// after the start or leaving, or a session longer than the supported bounds.
#[derive(Clone, Debug)]
pub struct ReproductionRecorder {
    reproduction: Reproduction,
    stopped: Option<&'static str>,
}

impl ReproductionRecorder {
    /// Starts recording `game`, which must be fresh: no tick advanced and no player yet.
    pub fn start(game: &ArpgGame) -> Result<Self, GameError> {
        // Workbench edits made before recording would be missing from the reproduction.
        if game.tick != 0
            || !game.players.is_empty()
            || game.workbench.as_ref().is_some_and(Workbench::edited)
        {
            return Err(GameError::new(
                "a reproduction must be recorded from a fresh game",
            ));
        }
        Ok(Self {
            reproduction: Reproduction {
                scenario: game.scenario,
                seed: game.run_seed,
                players: Vec::new(),
                ticks: 0,
                commands: Vec::new(),
                operations: Vec::new(),
            },
            stopped: None,
        })
    }

    fn stop(&mut self, reason: &'static str) {
        if self.stopped.is_none() {
            self.stopped = Some(reason);
        }
    }

    /// An accepted `add_player`.
    pub fn record_player_added(&mut self, player_id: PlayerId) {
        // Replay adds every player before anything else, so a join after a workbench
        // operation (even within tick 0) would replay in another order.
        if self.reproduction.ticks > 0 || !self.reproduction.operations.is_empty() {
            self.stop("a player joined after the session started");
        } else {
            self.reproduction.players.push(player_id);
        }
    }

    /// A successful `remove_player`.
    pub fn record_player_removed(&mut self) {
        self.stop("a player left the session");
    }

    /// An accepted command, applied before the next tick runs.
    pub fn record_command(&mut self, command: &PlayerCommand<ArpgCommand>) {
        if self.stopped.is_some() {
            return;
        }
        if self.reproduction.commands.len() >= MAX_REPRODUCTION_COMMANDS {
            self.stop("the session sent more commands than a reproduction can hold");
            return;
        }
        self.reproduction.commands.push(ReproductionCommand {
            tick: self.reproduction.ticks,
            player_id: command.player_id,
            sequence: command.sequence,
            command: command.command,
        });
    }

    /// An accepted workbench operation, applied before the next tick runs.
    pub fn record_workbench(&mut self, operation: &WorkbenchOperation) {
        if self.stopped.is_some() {
            return;
        }
        if self.reproduction.operations.len() >= MAX_REPRODUCTION_OPERATIONS {
            self.stop("the session applied more workbench operations than a reproduction can hold");
            return;
        }
        self.reproduction.operations.push(ReproductionOperation {
            tick: self.reproduction.ticks,
            command_index: self.reproduction.commands.len(),
            operation: operation.clone(),
        });
    }

    /// A successful `advance_tick`.
    pub fn record_tick(&mut self) {
        if self.stopped.is_some() {
            return;
        }
        if self.reproduction.ticks >= MAX_REPRODUCTION_TICKS {
            self.stop("the session ran longer than a reproduction can hold");
            return;
        }
        self.reproduction.ticks += 1;
    }

    /// The recording up to the last completed tick, or why the session can no longer be
    /// reproduced. Commands accepted after the last completed tick (for example while the
    /// arena is paused) have not affected any snapshot yet and are left out, so every
    /// export replays.
    pub fn reproduction(&self) -> Result<Reproduction, GameError> {
        if let Some(reason) = self.stopped {
            return Err(GameError::new(format!(
                "this session cannot be exported as a reproduction: {reason}"
            )));
        }
        let mut reproduction = self.reproduction.clone();
        let ticks = reproduction.ticks;
        reproduction.commands.retain(|command| command.tick < ticks);
        reproduction
            .operations
            .retain(|operation| operation.tick < ticks);
        Ok(reproduction)
    }
}

const SCENARIO_ROOM: RoomId = 2;
/// Inside the light swing's reach, so only the enemy's retreat makes the swing miss.
const RETREATING_TARGET_OFFSET: i32 = 210;
const SCENARIO_PILLAR_HALF_EXTENTS: Vec3i = Vec3i::new(20, WALL_HALF_HEIGHT, 60);
/// The traversal scenario's platform (#208): its top, 60 units up, is within a jump's
/// 90-unit apex; its near edge is 160 units east of the player's spawn.
const TRAVERSAL_PLATFORM_OFFSET: i32 = 260;
const TRAVERSAL_PLATFORM_HALF_EXTENTS: Vec3i = Vec3i::new(100, 30, 100);
/// The platform's walkable region and its connections to the floor: a jump both ways and a
/// one-way drop.
const TRAVERSAL_PLATFORM_REGION: RegionId = 2;
/// The traversal scenario's low ceiling (#209), 250 units north of the spawn: its underside,
/// 140 units up, clears a standing body (100 tall) but not a jump's 90-unit apex.
const TRAVERSAL_CEILING_OFFSET: i32 = 250;
const TRAVERSAL_CEILING_BOTTOM: i32 = 140;
const TRAVERSAL_CEILING_HALF_EXTENTS: Vec3i = Vec3i::new(100, 20, 100);
/// The traversal scenario's bridge (#210), adjoining the platform's far edge: its top, 140
/// units up, is a jump from the platform (80 up) but out of reach from the floor, and its
/// underside, 120 units up, lets a body walk the floor path beneath it.
const TRAVERSAL_BRIDGE_OFFSET: i32 = 510;
const TRAVERSAL_BRIDGE_HALF_EXTENTS: Vec3i = Vec3i::new(150, 10, 60);
const TRAVERSAL_BRIDGE_TOP: i32 = 140;
const TRAVERSAL_BRIDGE_REGION: RegionId = 3;

/// One resolved strike as published in a snapshot for diagnostics and feedback.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrikeEventSnapshot {
    /// Position among every event (strikes and interactions) resolved in the same tick.
    pub order: u32,
    pub source: StrikeSource,
    pub strike_tick: u64,
    pub definition: String,
    pub target: StrikeTarget,
    pub result: StrikeResult,
}

/// One projectile in flight as published: a player's arrow, or a ranged monster's shot
/// (#108), with its position and velocity rounded to whole world units. Both fly the same
/// path; a player's arrow hits monsters, a monster's shot hits players.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArrowSnapshot {
    pub id: u64,
    pub source: StrikeSource,
    pub launched_at_tick: u64,
    pub position: [i32; 3],
    /// World units travelled per tick; committed at release and never steered.
    pub velocity: [i32; 3],
    pub damage: u16,
    pub ticks_remaining: u8,
}

/// Which attack input continues a combo.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ComboInput {
    Light,
    Heavy,
}

impl ActionKind {
    fn windup_ticks(self, content: &Content) -> u8 {
        content.action(self).windup_ticks
    }

    fn active_ticks(self, content: &Content) -> u8 {
        content.action(self).active_ticks
    }

    fn recovery_ticks(self, content: &Content) -> u8 {
        content.action(self).recovery_ticks
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionPhase {
    Windup,
    Active,
    Recovery,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerActionSnapshot {
    pub kind: ActionKind,
    pub phase: ActionPhase,
    pub ticks_remaining: u8,
    pub facing: [i8; 2],
    /// Whether this action's own strike hit a target (combo hit confirmation).
    pub connected: bool,
    /// The single buffered combo input waiting for its transition interval.
    pub buffered: Option<ComboInput>,
    /// Bow draw ticks committed at release (zero for other actions).
    pub charge: u8,
    /// Exact direction committed from aim intent or a target lock; `None` means the action
    /// follows its committed `facing`.
    pub aim: Option<[i16; 2]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MonsterReactionKind {
    Stagger,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonsterReactionSnapshot {
    pub kind: MonsterReactionKind,
    pub ticks_remaining: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlayerReactionKind {
    Hurt,
    Blocked,
    GuardBroken,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GuardPhase {
    /// Shield is coming up and does not protect yet.
    Raising,
    /// Shield protects the front sector.
    Raised,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuardStance {
    pub phase: GuardPhase,
    /// Remaining raise ticks; zero once raised.
    pub ticks_remaining: u8,
}

/// Authoritative shield guard state of one player or shield monster.
///
/// Players and shield monsters (#92) share these rules; only who holds the guard differs:
/// a player's guard input, or a monster's engagement policy. For a monster, "hurt" is its
/// stagger.
///
/// Rules: guard rises while the guard input is held and the player is alive, not acting,
/// not hurt and not guard-broken. Starting an action, a hurt reaction or death lowers it;
/// the held input then raises it again from the start once the player is free. Movement
/// and turning stay allowed while guarding. A raised shield protects 60 degrees either
/// side of the current facing against blockable strikes; each block costs guard points,
/// and a strike whose cost reaches the remaining points breaks the guard instead.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuardState {
    pub held: bool,
    pub stance: Option<GuardStance>,
    pub points: u16,
    pub broken_ticks_remaining: u8,
    pub block_reaction_ticks_remaining: u8,
}

/// The single pending counter opportunity granted by an actual successful block.
///
/// Eligibility is the half-open command interval `[usable_from_tick, expires_at_tick)`,
/// compared with the game tick at which a command is applied. The block resolves during
/// tick `blocked_at_tick`; the first command applied afterwards may already use it. A
/// fresh primary attack within the interval starts the counter and consumes it, even if
/// the counter later misses. A newer successful block replaces it; an unblocked hit, guard
/// break or death removes it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CounterOpportunity {
    pub blocked_monster_id: u32,
    pub blocked_at_tick: u64,
    pub usable_from_tick: u64,
    pub expires_at_tick: u64,
}

impl CounterOpportunity {
    fn grant(window_ticks: u64, blocked_monster_id: u32, blocked_at_tick: u64) -> Self {
        let usable_from_tick = blocked_at_tick + 1;
        Self {
            blocked_monster_id,
            blocked_at_tick,
            usable_from_tick,
            expires_at_tick: usable_from_tick + window_ticks,
        }
    }

    fn usable_at(self, tick: u64) -> bool {
        (self.usable_from_tick..self.expires_at_tick).contains(&tick)
    }
}

impl GuardState {
    fn ready(max_points: u16) -> Self {
        Self {
            held: false,
            stance: None,
            points: max_points,
            broken_ticks_remaining: 0,
            block_reaction_ticks_remaining: 0,
        }
    }

    fn is_raised(self) -> bool {
        matches!(
            self.stance,
            Some(GuardStance {
                phase: GuardPhase::Raised,
                ..
            })
        )
    }

    /// One tick of the shared guard timeline. `free` means neither acting nor hurt (or
    /// staggered), with a shield in hand.
    fn advance(&mut self, tuning: &GuardData, alive: bool, free: bool) {
        self.block_reaction_ticks_remaining = self.block_reaction_ticks_remaining.saturating_sub(1);
        if !alive {
            self.held = false;
            self.stance = None;
            self.block_reaction_ticks_remaining = 0;
            return;
        }
        if self.broken_ticks_remaining > 0 {
            self.broken_ticks_remaining -= 1;
            self.stance = None;
            return;
        }
        self.stance = if self.held && free {
            Some(match self.stance {
                None => GuardStance {
                    phase: GuardPhase::Raising,
                    ticks_remaining: tuning.raise_ticks,
                },
                Some(GuardStance {
                    phase: GuardPhase::Raising,
                    ticks_remaining,
                }) if ticks_remaining > 1 => GuardStance {
                    phase: GuardPhase::Raising,
                    ticks_remaining: ticks_remaining - 1,
                },
                Some(_) => GuardStance {
                    phase: GuardPhase::Raised,
                    ticks_remaining: 0,
                },
            })
        } else {
            None
        };
        if self.stance.is_none() {
            self.points = self
                .points
                .saturating_add(tuning.regen_per_tick)
                .min(tuning.max_points);
        }
    }

    /// Whether a saved guard is reachable under `tuning`; `can_guard` means alive, not
    /// acting and not hurt (or staggered).
    fn save_valid(self, tuning: &GuardData, can_guard: bool) -> bool {
        let stance_valid = match self.stance {
            None => true,
            Some(stance) => {
                let timing = match stance.phase {
                    GuardPhase::Raising => {
                        (1..=tuning.raise_ticks).contains(&stance.ticks_remaining)
                    }
                    GuardPhase::Raised => stance.ticks_remaining == 0,
                };
                timing && self.held && can_guard && self.broken_ticks_remaining == 0
            }
        };
        stance_valid
            && self.points <= tuning.max_points
            && self.broken_ticks_remaining <= tuning.break_ticks
            && self.block_reaction_ticks_remaining <= tuning.block_reaction_ticks
    }

    /// Keeps live guard state within new tuning: a running timer continues as if it had
    /// started under the new value (the elapsed time counts) and one that ended stays ended.
    pub(crate) fn follow_tuning(&mut self, old: &GuardData, tuning: &GuardData) {
        let follow = |remaining: u8, old_total: u8, new_total: u8| {
            if remaining == 0 {
                return 0;
            }
            new_total.saturating_sub(old_total.saturating_sub(remaining))
        };
        self.points = self.points.min(tuning.max_points);
        self.broken_ticks_remaining = follow(
            self.broken_ticks_remaining,
            old.break_ticks,
            tuning.break_ticks,
        );
        self.block_reaction_ticks_remaining = follow(
            self.block_reaction_ticks_remaining,
            old.block_reaction_ticks,
            tuning.block_reaction_ticks,
        );
        if let Some(stance) = self.stance.as_mut()
            && stance.phase == GuardPhase::Raising
        {
            let left = follow(stance.ticks_remaining, old.raise_ticks, tuning.raise_ticks);
            // A raise that already took the new duration completes now; a completed raise
            // stays raised.
            *stance = if left == 0 {
                GuardStance {
                    phase: GuardPhase::Raised,
                    ticks_remaining: 0,
                }
            } else {
                GuardStance {
                    phase: GuardPhase::Raising,
                    ticks_remaining: left,
                }
            };
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerReactionSnapshot {
    pub kind: PlayerReactionKind,
    pub ticks_remaining: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonsterActionSnapshot {
    pub phase: ActionPhase,
    pub ticks_remaining: u8,
    pub target_player_id: PlayerId,
    pub range: i32,
    /// How the attack lands, so presentation telegraphs a ring or a line of fire.
    pub delivery: AttackDelivery,
}

/// How a monster attack reaches its target (#108).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AttackDelivery {
    /// Resolves the strike around the monster when the active phase opens.
    Strike,
    /// Launches a projectile at the target when the active phase opens.
    Projectile,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArpgSnapshot {
    pub schema_version: u16,
    pub run_seed: RunSeed,
    pub tick: u64,
    pub world_units_per_meter: i32,
    pub rooms: Vec<RoomSnapshot>,
    pub doors: Vec<DoorSnapshot>,
    pub players: Vec<PlayerSnapshot>,
    pub monsters: Vec<MonsterSnapshot>,
    pub ground_loot: Vec<GroundLootSnapshot>,
    pub static_colliders: Vec<StaticColliderSnapshot>,
    pub arrows: Vec<ArrowSnapshot>,
    pub scenario: ScenarioId,
    /// Revision of the content bundle this authority runs.
    pub content_revision: String,
    /// Strikes resolved during the tick that produced this snapshot, in resolution order.
    /// Transient presentation evidence: not part of saves, so a freshly restored game
    /// publishes none until its next tick.
    pub strike_events: Vec<StrikeEventSnapshot>,
    pub chests: Vec<ChestSnapshot>,
    /// Interaction attempts resolved during the tick that produced this snapshot.
    pub interaction_events: Vec<InteractionEventSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomSnapshot {
    pub id: RoomId,
    pub min_x: i32,
    pub max_x: i32,
    pub min_z: i32,
    pub max_z: i32,
    pub kind: RoomKind,
    pub encounter_state: RoomEncounterState,
    pub neighbors: Vec<RoomId>,
    /// The encounter definition a combat room runs (`None` for the start room), published
    /// like `MonsterSnapshot::definition` so presentation can key on it.
    pub encounter: Option<String>,
}

impl RoomSnapshot {
    /// Whether a physical position lies inside the room on the plane.
    fn contains_xz(&self, position: Vector) -> bool {
        self.contains_xz_with_margin(position, 0)
    }

    /// Whether a physical position lies inside the room shrunk by `margin` on every side.
    fn contains_xz_with_margin(&self, position: Vector, margin: i32) -> bool {
        let (x, z) = (f64::from(position.x), f64::from(position.z));
        x >= f64::from(self.min_x + margin)
            && x <= f64::from(self.max_x - margin)
            && z >= f64::from(self.min_z + margin)
            && z <= f64::from(self.max_z - margin)
    }

    fn center(&self) -> (i32, i32) {
        ((self.min_x + self.max_x) / 2, (self.min_z + self.max_z) / 2)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomKind {
    Start,
    Combat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoomEncounterState {
    Dormant,
    Active,
    Cleared,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoorSnapshot {
    pub id: DoorId,
    pub room_a: RoomId,
    pub room_b: RoomId,
    pub position: [i32; 3],
    pub half_extents: [i32; 3],
    pub locked: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerSnapshot {
    pub id: PlayerId,
    pub position: [i32; 3],
    pub health: u16,
    pub max_health: u16,
    pub level: u16,
    pub experience: u32,
    pub experience_into_level: u32,
    pub experience_for_next_level: u32,
    pub attack_damage: u16,
    pub gold: u32,
    pub alive: bool,
    pub facing: [i8; 2],
    pub action: Option<PlayerActionSnapshot>,
    pub reaction: Option<PlayerReactionSnapshot>,
    pub guard: Option<GuardStance>,
    pub guard_points: u16,
    pub max_guard_points: u16,
    pub counter: Option<CounterOpportunity>,
    pub weapon: Weapon,
    /// Bow draw ticks while the bow is being drawn.
    pub draw_ticks: Option<u8>,
    /// What Interact would act on now, or why it would do nothing.
    pub interaction: InteractionPrompt,
    /// Current aim intent; `None` while facing follows movement.
    pub aim: Option<[i16; 2]>,
    /// Monster held by the optional target lock.
    pub locked_monster_id: Option<u32>,
    /// The body's physical height and support (#59).
    pub motion: PlayerMotionSnapshot,
    /// Walkable region membership or transit (#207).
    pub region: RegionMembership,
}

/// A player body's height and support (#59), for presentation and animation: they place
/// and pose the character from it and never decide height or support themselves.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerMotionSnapshot {
    /// Supported by the ground; false from takeoff to landing.
    pub grounded: bool,
    /// Vertical velocity (world units per tick, rounded): positive while rising.
    pub vertical_velocity: i32,
    /// Height of the body's lowest point, its feet.
    pub feet_y: i32,
    /// Height of the ground directly below the body, where its feet would stand; `None`
    /// when no ground lies below.
    pub ground_y: Option<i32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonsterSnapshot {
    pub id: u32,
    /// Content definition id (for example `monster.brute`): presentation keys appearance
    /// and audio by it; gameplay never reads it back.
    pub definition: String,
    pub room_id: RoomId,
    pub position: [i32; 3],
    pub health: u16,
    pub max_health: u16,
    pub alive: bool,
    pub action: Option<MonsterActionSnapshot>,
    pub reaction: Option<MonsterReactionSnapshot>,
    /// What the monster did in the tick that produced this snapshot (#109).
    pub behavior: MonsterBehavior,
    /// The player it is engaged with, while alive, engaged and in a running encounter.
    pub target_player_id: Option<PlayerId>,
    /// Shield guard of a shield monster (#92), as on `PlayerSnapshot`; a raised guard
    /// faces `target_player_id`. Monsters without a shield publish `None` and zero points.
    pub guard: Option<GuardStance>,
    pub guard_points: u16,
    pub max_guard_points: u16,
}

/// A monster's engagement with the players of its room (#109). Authoritative and saved;
/// it changes only at the end of a tick in which the monster is free (alive, in a running
/// encounter, neither staggered nor acting), or when a player's strike provokes it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Engagement {
    /// At rest: engages the nearest living player of its room within aggro range.
    #[default]
    Idle,
    /// Pursues and attacks `target_player_id`, which it keeps unless a competitor is nearer
    /// by more than the switch margin (or attackable while the target is not).
    Engaged { target_player_id: PlayerId },
    /// Lost its target (death, departure); holds for `ticks_remaining` free ticks, then
    /// engages whoever is in aggro range or returns to its post.
    Searching { ticks_remaining: u8 },
    /// Broke off at the leash (or found nobody); walks back to its post and ignores
    /// players until it arrives.
    Returning,
}

/// Published per-tick monster behaviour, so presentation never infers it from motion.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MonsterBehavior {
    Dead,
    /// Its room's encounter is not running.
    Dormant,
    Staggered,
    /// In an attack's wind-up, active or recovery phase (see `action`).
    Attacking,
    /// Engaged and moved toward its target this tick.
    Pursuing,
    /// Engaged and backed away from its target this tick to keep its spacing (#108).
    Retreating,
    /// Engaged but stood still this tick: about to strike, provoked this tick, or without
    /// a route to a place it could strike from.
    Holding,
    Searching,
    Returning,
    Idle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LootKind {
    Gold,
}

/// A reward chest. Core owns availability: it can be opened once its room is cleared.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChestSnapshot {
    pub id: u64,
    pub room_id: RoomId,
    pub position: [i32; 3],
    pub opened: bool,
    /// Whether the chest can be opened now (its room is cleared and it is still closed).
    pub available: bool,
}

/// What an interaction would or did act on.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "camelCase")]
pub enum InteractionTarget {
    Loot(GroundLootId),
    Chest(u64),
}

/// Why an interaction did nothing. Ordinary gameplay refusals, not transport errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InteractionRefusal {
    /// Nothing interactable within reach.
    NothingInRange,
    /// The nearest interactable is a chest whose room is not cleared yet.
    ChestLocked,
    /// Everything within reach is behind fixed geometry.
    Obstructed,
    /// The player is mid-action (or defeated) and cannot start an interaction now.
    Busy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum InteractionResult {
    PickedUp {
        target: InteractionTarget,
        gold: u32,
    },
    Opened {
        target: InteractionTarget,
        gold: u32,
    },
    Refused {
        reason: InteractionRefusal,
    },
}

/// What a player's Interact would do now: the authoritative source for prompts.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum InteractionPrompt {
    Available { target: InteractionTarget },
    Unavailable { reason: InteractionRefusal },
}

/// One resolved interaction attempt, published with the tick that resolved it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionEventSnapshot {
    /// Position among every event (strikes and interactions) resolved in the same tick.
    pub order: u32,
    pub player_id: PlayerId,
    pub result: InteractionResult,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroundLootSnapshot {
    pub id: GroundLootId,
    pub position: [i32; 3],
    pub kind: LootKind,
    pub amount: u32,
}

/// A saved projectile: its authoritative `f32` position and velocity (#182).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArrowSaveState {
    pub id: u64,
    pub source: StrikeSource,
    pub launched_at_tick: u64,
    pub position: [f32; 3],
    /// World units travelled per tick; committed at release and never steered.
    pub velocity: [f32; 3],
    pub damage: u16,
    pub ticks_remaining: u8,
}

/// Saved ground loot: it lies where its monster fell, in `f32` world units (#182).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GroundLootSaveState {
    pub id: GroundLootId,
    pub position: [f32; 3],
    pub kind: LootKind,
    pub amount: u32,
}

/// Saved game state. Physical positions and velocities are the authoritative `f32` values;
/// Rapier's own state is never saved: a load rebuilds the physical world from the seed and
/// this state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArpgSaveState {
    pub schema_version: u16,
    pub rules_version: u16,
    pub run_seed: RunSeed,
    pub tick: u64,
    pub players: Vec<PlayerSaveState>,
    pub rooms: Vec<RoomSaveState>,
    pub monsters: Vec<MonsterSaveState>,
    pub ground_loot: Vec<GroundLootSaveState>,
    pub next_ground_loot_id: GroundLootId,
    pub arrows: Vec<ArrowSaveState>,
    pub next_arrow_id: u64,
    pub scenario: ScenarioId,
    /// Ids of chests already opened.
    pub opened_chests: Vec<u64>,
    /// Revision of the content bundle the authority ran; a different bundle is rejected.
    pub content_revision: String,
}

impl ArpgSaveState {
    /// Parses a save document's JSON, refusing a save of another schema or rules version
    /// by its version before any field it lacks or adds: older saves are refused cleanly.
    pub fn from_json(json: &str) -> Result<Self, GameError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Versions {
            schema_version: u16,
            rules_version: u16,
        }
        let versions = serde_json::from_str::<Versions>(json)
            .map_err(|error| GameError::new(format!("invalid ARPG save: {error}")))?;
        ArpgGame::check_save_versions(versions.schema_version, versions.rules_version)?;
        serde_json::from_str(json)
            .map_err(|error| GameError::new(format!("invalid ARPG save: {error}")))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlayerSaveState {
    pub id: PlayerId,
    /// Physical position in world units.
    pub position: [f32; 3],
    /// Physical velocity (world units per tick) carried into the next tick by controlled
    /// acceleration: the part of last tick's intended motion that physics allowed. Its
    /// vertical part is the jump's or fall's speed while airborne, and zero on the ground.
    pub velocity: [f32; 3],
    /// Supported by the ground at the end of the saved tick.
    pub grounded: bool,
    /// Ticks left in which a jump may still take off after leaving the ground unjumped.
    pub coyote_ticks_remaining: u8,
    /// Ticks left in which a buffered jump press may still take off.
    pub jump_buffer_ticks_remaining: u8,
    /// Walkable region membership or transit (#207).
    pub region: RegionMembership,
    /// Where the player last ended a tick supported, in world units (#211).
    pub last_supported: [f32; 3],
    pub movement: [i8; 2],
    pub facing: [i8; 2],
    pub action: Option<PlayerActionSnapshot>,
    pub hurt_ticks_remaining: u8,
    pub guard: GuardState,
    pub counter: Option<CounterOpportunity>,
    pub weapon: Weapon,
    pub draw_ticks: Option<u8>,
    pub aim: Option<[i16; 2]>,
    pub locked_monster_id: Option<u32>,
    pub health: u16,
    pub experience: u32,
    pub gold: u32,
    /// Datagram-lane watermark.
    pub last_sequence: u32,
    /// Reliable-lane watermark. Saves written before the reliable lane omit it and load
    /// with 0; the save schema version is unchanged.
    #[serde(default)]
    pub last_reliable_sequence: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoomSaveState {
    pub id: RoomId,
    pub encounter_state: RoomEncounterState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MonsterActionSaveState {
    pub phase: ActionPhase,
    pub ticks_remaining: u8,
    pub target_player_id: PlayerId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MonsterSaveState {
    pub id: u32,
    pub room_id: RoomId,
    /// Physical position in world units.
    pub position: [f32; 3],
    pub health: u16,
    pub action: Option<MonsterActionSaveState>,
    pub stagger_ticks_remaining: u8,
    /// Where the monster stood when its encounter first evaluated it, in world units;
    /// leash and return measure from here.
    pub post: Option<[f32; 3]>,
    pub engagement: Engagement,
    /// Whether steering moved the body in the tick the save was taken after: the one
    /// input of the published behaviour that the rest of the state does not determine.
    pub steered: bool,
    /// Whether that movement backed away from the target to keep spacing (#108).
    pub retreating: bool,
    /// Shield guard (#92): present exactly for a shield monster.
    pub guard: Option<GuardState>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StaticColliderSnapshot {
    pub id: u64,
    pub position: [i32; 3],
    pub half_extents: [i32; 3],
    pub kind: StaticColliderKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StaticColliderKind {
    Wall,
    Pillar,
    Door,
    /// A walkable box top (#208): a floor of its own region, standing on the dungeon floor.
    Platform,
    /// A low ceiling (#209): a box overhead that a jump bumps into and a body walks under.
    Ceiling,
}

/// Speed changes per tick by which controlled horizontal velocity converges on the movement
/// intent: towards a held direction, to rest, and when reversing.
#[derive(Clone, Copy, Debug)]
struct ControlRates {
    acceleration: i32,
    braking: i32,
    reversal: i32,
}

impl ControlRates {
    const GROUND: Self = Self {
        acceleration: PLAYER_ACCELERATION_PER_TICK,
        braking: PLAYER_BRAKING_PER_TICK,
        reversal: PLAYER_REVERSAL_PER_TICK,
    };
}

#[derive(Clone, Copy, Debug)]
struct ActionState {
    kind: ActionKind,
    phase: ActionPhase,
    ticks_remaining: u8,
    facing_x: i8,
    facing_z: i8,
    connected: bool,
    buffered: Option<ComboInput>,
    charge: u8,
    aim: Option<[i16; 2]>,
}

impl ActionState {
    fn snapshot(self) -> PlayerActionSnapshot {
        PlayerActionSnapshot {
            kind: self.kind,
            phase: self.phase,
            ticks_remaining: self.ticks_remaining,
            facing: [self.facing_x, self.facing_z],
            connected: self.connected,
            buffered: self.buffered,
            charge: self.charge,
            aim: self.aim,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct MonsterActionState {
    phase: ActionPhase,
    ticks_remaining: u8,
    target_player_id: PlayerId,
}

impl MonsterActionState {
    fn snapshot(self, definition: &MonsterDefinition) -> MonsterActionSnapshot {
        MonsterActionSnapshot {
            phase: self.phase,
            ticks_remaining: self.ticks_remaining,
            target_player_id: self.target_player_id,
            range: i32::try_from(definition.strike.reach)
                .expect("monster attack range must fit i32"),
            delivery: if definition.projectile.is_some() {
                AttackDelivery::Projectile
            } else {
                AttackDelivery::Strike
            },
        }
    }
}

/// Who performed a strike.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "camelCase")]
pub enum StrikeSource {
    Player(PlayerId),
    Monster(u32),
}

/// What a strike contacted. Ordering is the stable identity tie-break for equal distances.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "camelCase")]
pub enum StrikeTarget {
    Player(PlayerId),
    Monster(u32),
}

/// Stable identity of one strike: its source and the tick on which its active window opened.
/// A source performs at most one action at a time, so this pair is unique and targets are
/// deduplicated per strike rather than per rendered frame or active tick.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct StrikeId {
    pub source: StrikeSource,
    pub tick: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum StrikeResult {
    /// The strike connected and dealt `damage`; `defeated` reports a killing blow.
    Hit { damage: u16, defeated: bool },
    /// A raised shield intercepted the strike: no health damage, `guard_damage` guard spent.
    Blocked { guard_damage: u16 },
    /// The strike reached a raised shield but its guard cost exhausted the guard: no health
    /// damage, the guard drops and cannot be raised for a recovery period.
    GuardBroken,
    /// The target was inside the strike volume but fixed geometry (a wall, pillar or closed
    /// door) lies between attacker and target.
    Obstructed,
}

/// The strike definition arrows publish: arrows are bow tuning, not a content strike.
static BOW_ARROW_DEFINITION: LazyLock<DefinitionId> = LazyLock::new(|| "bow.arrow".into());

/// One reasoned strike outcome. Player and monster melee share this result path; later
/// projectile, block and guard-break outcomes extend `StrikeResult` rather than adding a
/// parallel path per weapon.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StrikeOutcome {
    /// Position among every event (strikes and interactions) resolved in the same tick.
    pub order: u32,
    pub strike: StrikeId,
    /// The strike definition id, owned: it stays valid after the game and its content are
    /// dropped.
    pub definition: DefinitionId,
    pub target: StrikeTarget,
    pub result: StrikeResult,
}

#[derive(Clone, Copy, Debug)]
struct PlayerState {
    /// Authoritative physical position in world units, mirrored by its Rapier body.
    position: Vector,
    /// The motion physics allowed last tick, in world units per tick; vertically the
    /// airborne speed (zero on the ground).
    velocity: Vector,
    /// Supported by the ground after last tick's motion (Rapier's controller and support
    /// query). Guard and counter need it; jumps take off from it.
    grounded: bool,
    /// Coyote window: ticks in which a jump may still take off after the body left the
    /// ground without jumping.
    coyote_ticks_remaining: u8,
    /// Jump buffer: ticks in which a jump press keeps waiting for a takeoff.
    jump_buffer_ticks_remaining: u8,
    /// The walkable region it belongs to, or the connection it traverses (#207).
    region: RegionMembership,
    /// Where it last ended a tick supported: a fall out of the world returns it here (#211).
    last_supported: Vector,
    movement_x: i8,
    movement_z: i8,
    facing_x: i8,
    facing_z: i8,
    action: Option<ActionState>,
    hurt_ticks_remaining: u8,
    guard: GuardState,
    counter: Option<CounterOpportunity>,
    weapon: Weapon,
    draw_ticks: Option<u8>,
    aim: Option<[i16; 2]>,
    locked_monster_id: Option<u32>,
    health: u16,
    experience: u32,
    gold: u32,
}

#[derive(Clone, Copy, Debug)]
struct MonsterState {
    id: u32,
    /// Its definition in the content the game runs (plain data; its id is resolved through
    /// that content).
    definition: MonsterDefinition,
    room_id: RoomId,
    /// Authoritative physical position in world units, mirrored by its Rapier body while
    /// it has one.
    position: Vector,
    health: u16,
    action: Option<MonsterActionState>,
    stagger_ticks_remaining: u8,
    /// Where it stood when its encounter first evaluated it (leash and return).
    post: Option<Vector>,
    engagement: Engagement,
    /// Whether steering gave the body a velocity in the last tick.
    steered: bool,
    /// Whether that velocity backed away from the target (#108).
    retreating: bool,
    /// Transient within a tick, never saved: steering wanted to back away this tick and
    /// found no way to, so the attack may start from inside the retreat range.
    cornered: bool,
    /// Shield guard of a shield monster (#92); `held` is its engagement policy's choice.
    guard: Option<GuardState>,
}

impl MonsterState {
    fn new(
        id: u32,
        definition: MonsterDefinition,
        room_id: RoomId,
        position: Vec3i,
        guard_max_points: u16,
    ) -> Self {
        Self {
            id,
            definition,
            room_id,
            position: vector(position),
            health: definition.health,
            action: None,
            stagger_ticks_remaining: 0,
            post: None,
            engagement: Engagement::Idle,
            steered: false,
            retreating: false,
            cornered: false,
            guard: definition
                .shield
                .then(|| GuardState::ready(guard_max_points)),
        }
    }

    fn definition(&self) -> MonsterDefinition {
        self.definition
    }

    /// The published behaviour: a function of authoritative state and whether the body
    /// was steered this tick.
    fn derived_behavior(&self, active: bool, steered: bool) -> MonsterBehavior {
        if self.health == 0 {
            MonsterBehavior::Dead
        } else if !active {
            MonsterBehavior::Dormant
        } else if self.stagger_ticks_remaining > 0 {
            MonsterBehavior::Staggered
        } else if self.action.is_some() {
            MonsterBehavior::Attacking
        } else {
            match self.engagement {
                Engagement::Idle => MonsterBehavior::Idle,
                Engagement::Engaged { .. } if steered && self.retreating => {
                    MonsterBehavior::Retreating
                }
                Engagement::Engaged { .. } if steered => MonsterBehavior::Pursuing,
                Engagement::Engaged { .. } => MonsterBehavior::Holding,
                Engagement::Searching { .. } => MonsterBehavior::Searching,
                Engagement::Returning => MonsterBehavior::Returning,
            }
        }
    }

    /// A player's hit turns a resting or searching monster on its attacker. An engaged
    /// monster keeps its target (hysteresis); a returning one ignores it until home.
    fn provoke(&mut self, player_id: PlayerId) {
        if matches!(
            self.engagement,
            Engagement::Idle | Engagement::Searching { .. }
        ) {
            self.engagement = Engagement::Engaged {
                target_player_id: player_id,
            };
        }
    }

    fn engaged_target(&self) -> Option<PlayerId> {
        match self.engagement {
            Engagement::Engaged { target_player_id } => Some(target_player_id),
            _ => None,
        }
    }
}

/// How a free monster's body moves this tick (#65, #109).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Steering {
    Hold,
    /// Toward a place within strike reach of the engaged target.
    Pursue {
        target: Vector,
    },
    /// Away from an engaged target that came nearer than the retreat range (#108).
    Retreat {
        target: Vector,
    },
    /// Back to the post.
    Return {
        post: Vector,
    },
}

/// One authoritative projectile in flight (see [`ArrowSnapshot`]).
#[derive(Clone, Copy, Debug, PartialEq)]
struct ArrowState {
    id: u64,
    source: StrikeSource,
    launched_at_tick: u64,
    position: Vector,
    /// World units travelled per tick; committed at release and never steered.
    velocity: Vector,
    damage: u16,
    ticks_remaining: u8,
}

impl ArrowState {
    fn snapshot(&self) -> ArrowSnapshot {
        ArrowSnapshot {
            id: self.id,
            source: self.source,
            launched_at_tick: self.launched_at_tick,
            position: vec_to_array(Vec3i::rounded(self.position)),
            velocity: vec_to_array(Vec3i::rounded(self.velocity)),
            damage: self.damage,
            ticks_remaining: self.ticks_remaining,
        }
    }

    fn save(&self) -> ArrowSaveState {
        ArrowSaveState {
            id: self.id,
            source: self.source,
            launched_at_tick: self.launched_at_tick,
            position: self.position.to_array(),
            velocity: self.velocity.to_array(),
            damage: self.damage,
            ticks_remaining: self.ticks_remaining,
        }
    }

    fn restore(save: &ArrowSaveState) -> Self {
        Self {
            id: save.id,
            source: save.source,
            launched_at_tick: save.launched_at_tick,
            position: Vector::from_array(save.position),
            velocity: Vector::from_array(save.velocity),
            damage: save.damage,
            ticks_remaining: save.ticks_remaining,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct GroundLootState {
    id: GroundLootId,
    /// Where its monster fell, in world units.
    position: Vector,
    kind: LootKind,
    amount: u32,
}

#[derive(Clone, Copy, Debug)]
struct DungeonRng {
    state: u64,
}

impl DungeonRng {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
        }
    }

    fn next_u32(&mut self) -> u32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        (self.state >> 32) as u32
    }

    fn range_i32(&mut self, min: i32, max: i32) -> i32 {
        debug_assert!(min <= max);
        let span = u32::try_from(max - min + 1).expect("dungeon range must fit u32");
        min + i32::try_from(self.next_u32() % span).expect("range sample must fit i32")
    }
}

#[derive(Debug)]
struct GeneratedDungeon {
    rooms: Vec<RoomSnapshot>,
    doors: Vec<DoorSnapshot>,
    static_colliders: Vec<StaticColliderSnapshot>,
    player_spawns: [Vec3i; MAX_PLAYERS],
    monsters: Vec<MonsterState>,
}

/// Monster-pursuit navigation work since construction or load (#110). Work evidence only:
/// none of it is gameplay state, saved, or able to change an outcome.
///
/// A *repath* is a retained-field search or an exact plan.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NavigationWork {
    /// Monster-ticks spent pursuing.
    pub pursuit_ticks: u64,
    /// A* node expansions of retained-field searches and exact plans together.
    pub expansions: u64,
    /// Searches of a retained target field: a new target cell, a topology change, or a body
    /// that left every route the field already knows.
    pub field_searches: u64,
    /// Exact per-tick plans: the final approach from a goal cell, a target the field cannot
    /// reach or serve, and every plan of the per-tick reference.
    pub exact_plans: u64,
    /// Exact plans that back a spacing monster away from its target (#108). Not repaths:
    /// they plan away from a target, which the retained fields never serve.
    pub retreat_plans: u64,
    /// Physics ray queries made by the planners.
    pub physics_queries: u64,
    /// Room grids rasterized from the fixed bodies.
    pub grid_builds: u64,
    /// Target fields created (goal cells of one target cell computed).
    pub field_builds: u64,
}

impl NavigationWork {
    pub fn repaths(&self) -> u64 {
        self.field_searches + self.exact_plans
    }

    /// Work done since `earlier`, a reading of the same game.
    #[cfg(test)]
    fn since(self, earlier: Self) -> Self {
        Self {
            pursuit_ticks: self.pursuit_ticks - earlier.pursuit_ticks,
            expansions: self.expansions - earlier.expansions,
            field_searches: self.field_searches - earlier.field_searches,
            exact_plans: self.exact_plans - earlier.exact_plans,
            retreat_plans: self.retreat_plans - earlier.retreat_plans,
            physics_queries: self.physics_queries - earlier.physics_queries,
            grid_builds: self.grid_builds - earlier.grid_builds,
            field_builds: self.field_builds - earlier.field_builds,
        }
    }

    fn add(&mut self, other: Self) {
        self.pursuit_ticks += other.pursuit_ticks;
        self.expansions += other.expansions;
        self.field_searches += other.field_searches;
        self.exact_plans += other.exact_plans;
        self.retreat_plans += other.retreat_plans;
        self.physics_queries += other.physics_queries;
        self.grid_builds += other.grid_builds;
        self.field_builds += other.field_builds;
    }
}

/// Derived navigation data retained across ticks (#110). It is a cache in the strict
/// sense: rebuilt on demand from the world's fixed bodies and the targets' cells, never
/// saved, and dropping it at any tick changes work counts only (see `TargetField`).
#[derive(Default)]
struct NavigationCache {
    /// Footprints of the fixed bodies the grids and fields were built from; any change to
    /// them (a door locking or unlocking, a fixed body added or removed) drops everything.
    obstacles: Vec<Rect>,
    grids: BTreeMap<RoomId, RoomGrid>,
    /// Fields by room, target cell and goal reach, kept while a pursuer still chases that
    /// cell. Reach is part of the key: monsters of different definitions chasing the same
    /// cell stop at different distances.
    fields: BTreeMap<(RoomId, Cell, i64), TargetField>,
}

impl fmt::Debug for NavigationCache {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NavigationCache")
            .field("obstacles", &self.obstacles.len())
            .field("grids", &self.grids.keys().collect::<Vec<_>>())
            .field("fields", &self.fields.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// How pursuit plans; tests compare the retained planner with its references.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NavigationMode {
    Retained,
    /// The retained planner with its cache dropped after every tick: the same outcomes
    /// must follow.
    RetainedWithoutCache,
    /// The #65 per-tick planner: a fresh grid and an exact plan every tick.
    PerTickReference,
}

#[derive(Debug)]
pub struct ArpgGame {
    /// The validated content this game runs: the built-in bundle unless it was created or
    /// restored with another one. The game owns it: custom content is freed with the last
    /// game running it.
    content: Arc<Content>,
    run_seed: RunSeed,
    tick: u64,
    physics: Physics,
    physics_steps: u64,
    navigation_work: NavigationWork,
    navigation: NavigationCache,
    #[cfg(test)]
    navigation_mode: NavigationMode,
    /// Strike-geometry tests pack monsters closer than bodies allow; they keep monsters
    /// out of physics.
    #[cfg(test)]
    monsters_without_bodies: bool,
    players: BTreeMap<PlayerId, PlayerState>,
    /// Datagram-lane watermarks.
    last_sequences: BTreeMap<PlayerId, u32>,
    /// Reliable-lane watermarks, independent of the datagram lane.
    last_reliable_sequences: BTreeMap<PlayerId, u32>,
    rooms: Vec<RoomSnapshot>,
    doors: Vec<DoorSnapshot>,
    player_spawns: [Vec3i; MAX_PLAYERS],
    monsters: Vec<MonsterState>,
    ground_loot: Vec<GroundLootState>,
    next_ground_loot_id: GroundLootId,
    static_colliders: Vec<StaticColliderSnapshot>,
    strike_outcomes: Vec<StrikeOutcome>,
    arrows: Vec<ArrowState>,
    next_arrow_id: u64,
    scenario: ScenarioId,
    chests: Vec<ChestSnapshot>,
    interaction_events: Vec<InteractionEventSnapshot>,
    /// Counter, guard and bow values this game runs: the content's, unless a workbench
    /// session tuned them.
    tuning: Tuning,
    /// Present only in games created as workbench scenarios.
    workbench: Option<Workbench>,
    /// Walkable regions and typed connections (#207): one floor region until elevation
    /// exists (Atlas 5.1.2).
    topology: Topology,
}

impl Default for ArpgGame {
    fn default() -> Self {
        Self::new().expect("built-in ARPG fixture must be valid")
    }
}

impl ArpgGame {
    pub fn new() -> Result<Self, GameError> {
        Self::new_with_seed(DEFAULT_DUNGEON_SEED)
    }

    pub fn new_with_seed(run_seed: RunSeed) -> Result<Self, GameError> {
        Self::generated(Arc::clone(content()), run_seed)
    }

    /// The ordinary generated dungeon for `run_seed` running `bundle` instead of the
    /// built-in content. The bundle is validated first and refused with every violation;
    /// snapshots and saves carry its revision. The game owns that content, which is freed
    /// with the last game running it.
    pub fn new_with_content(bundle: ContentBundle, run_seed: RunSeed) -> Result<Self, GameError> {
        Self::generated(Self::load_content(bundle)?, run_seed)
    }

    /// The validated content of `bundle` for a game to own, shared with any running game of
    /// the same bundle.
    fn load_content(bundle: ContentBundle) -> Result<Arc<Content>, GameError> {
        content::load(bundle).map_err(Self::load_error)
    }

    fn load_error(error: content::LoadError) -> GameError {
        match error {
            content::LoadError::Invalid(errors) => GameError::new(format!(
                "content bundle is invalid: {}",
                errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            )),
            content::LoadError::Refused(reason) => {
                GameError::new(format!("content bundle refused: {reason}"))
            }
        }
    }

    fn generated(content: Arc<Content>, run_seed: RunSeed) -> Result<Self, GameError> {
        let mut physics = Physics::new();
        physics.add_floor(FLOOR_FOOT_Y as f32)?;
        let GeneratedDungeon {
            rooms,
            doors,
            static_colliders,
            player_spawns,
            monsters,
        } = generate_dungeon(&content, run_seed);
        let chests = rooms
            .iter()
            .filter(|room| room.kind == RoomKind::Combat)
            .map(|room| ChestSnapshot {
                id: CHEST_ID_BASE + u64::from(room.id),
                room_id: room.id,
                position: [
                    (room.min_x + room.max_x) / 2,
                    PLAYER_Y,
                    room.max_z - ROOM_SPAWN_MARGIN,
                ],
                opened: false,
                available: false,
            })
            .collect::<Vec<_>>();
        for collider in &static_colliders {
            physics.add_fixed(
                collider.id,
                array_to_vec(collider.position),
                array_to_vec(collider.half_extents),
            )?;
        }

        let tuning = Tuning::from_content(&content);
        Ok(Self {
            content,
            run_seed,
            tick: 0,
            physics,
            physics_steps: 0,
            navigation_work: NavigationWork::default(),
            navigation: NavigationCache::default(),
            #[cfg(test)]
            navigation_mode: NavigationMode::Retained,
            #[cfg(test)]
            monsters_without_bodies: false,
            players: BTreeMap::new(),
            last_sequences: BTreeMap::new(),
            last_reliable_sequences: BTreeMap::new(),
            rooms,
            doors,
            player_spawns,
            monsters,
            ground_loot: Vec::new(),
            next_ground_loot_id: GROUND_LOOT_ID_BASE,
            static_colliders,
            strike_outcomes: Vec::new(),
            arrows: Vec::new(),
            next_arrow_id: ARROW_ID_BASE,
            scenario: ScenarioId::Dungeon,
            chests,
            interaction_events: Vec::new(),
            tuning,
            workbench: None,
            topology: Topology::single_floor(FLOOR_FOOT_Y),
        })
    }

    /// The generated dungeon for `run_seed` arranged as `scenario`.
    /// Only games created here accept workbench operations
    /// ([`ArpgGame::apply_workbench`]).
    pub fn new_scenario(scenario: ScenarioId, run_seed: RunSeed) -> Result<Self, GameError> {
        let mut game = Self::arranged_scenario(scenario, run_seed)?;
        game.workbench = Some(Workbench::new(&game)?);
        Ok(game)
    }

    fn arranged_scenario(scenario: ScenarioId, run_seed: RunSeed) -> Result<Self, GameError> {
        let mut game = Self::with_scenario_layout(Arc::clone(content()), scenario, run_seed)?;
        let Some(target_offset) = scenario.target_offset() else {
            return Ok(game);
        };
        let room_id = scenario.room(&game.content, &game.monsters)?;
        let (center_x, center_z) = game
            .rooms
            .iter()
            .find(|room| room.id == room_id)
            .map(RoomSnapshot::center)
            .ok_or_else(|| GameError::new("scenario room is missing"))?;
        let mut placed = 0;
        for monster in game
            .monsters
            .iter_mut()
            .filter(|monster| monster.room_id == room_id)
        {
            // The first monster is the authored target; any others wait in a far corner.
            monster.position = if placed == 0 {
                Vec3i::new(center_x + target_offset, PLAYER_Y, center_z)
            } else {
                Vec3i::new(center_x - 600, PLAYER_Y, center_z + 300 - 120 * placed)
            }
            .into();
            placed += 1;
        }
        if placed == 0 {
            return Err(GameError::new("scenario room has no monster"));
        }
        Ok(game)
    }

    /// The generated dungeon plus the scenario's fixed geometry (no placements). Saves
    /// restore through this so authored pillars come back while positions come from the save.
    fn with_scenario_layout(
        content: Arc<Content>,
        scenario: ScenarioId,
        run_seed: RunSeed,
    ) -> Result<Self, GameError> {
        let mut game = Self::generated(content, run_seed)?;
        game.scenario = scenario;
        if scenario == ScenarioId::Dungeon {
            return Ok(game);
        }
        let room_id = scenario.room(&game.content, &game.monsters)?;
        let (center_x, center_z) = game
            .rooms
            .iter()
            .find(|room| room.id == room_id)
            .map(RoomSnapshot::center)
            .ok_or_else(|| GameError::new("scenario room is missing"))?;
        // Spawn points are layout: players joining a restored scenario arrive where they
        // would have in the uninterrupted game.
        game.player_spawns = [(0, 0), (0, 90), (0, -90), (-90, 0)]
            .map(|(dx, dz)| Vec3i::new(center_x + dx, PLAYER_Y, center_z + dz));
        if let Some(offset) = scenario.pillar_offset() {
            let index =
                u64::try_from(game.static_colliders.len()).expect("collider count fits u64");
            let pillar = StaticColliderSnapshot {
                id: STATIC_BODY_BASE + index,
                position: [center_x + offset, PLAYER_Y, center_z],
                half_extents: vec_to_array(SCENARIO_PILLAR_HALF_EXTENTS),
                kind: StaticColliderKind::Pillar,
            };
            game.physics.add_fixed(
                pillar.id,
                array_to_vec(pillar.position),
                SCENARIO_PILLAR_HALF_EXTENTS,
            )?;
            game.static_colliders.push(pillar);
        }
        if scenario == ScenarioId::Traversal {
            let index =
                u64::try_from(game.static_colliders.len()).expect("collider count fits u64");
            let half = TRAVERSAL_PLATFORM_HALF_EXTENTS;
            let platform = StaticColliderSnapshot {
                id: STATIC_BODY_BASE + index,
                position: [
                    center_x + TRAVERSAL_PLATFORM_OFFSET,
                    FLOOR_FOOT_Y + half.y,
                    center_z,
                ],
                half_extents: vec_to_array(half),
                kind: StaticColliderKind::Platform,
            };
            game.physics
                .add_fixed(platform.id, array_to_vec(platform.position), half)?;
            game.static_colliders.push(platform);
            let ceiling_half = TRAVERSAL_CEILING_HALF_EXTENTS;
            let ceiling = StaticColliderSnapshot {
                id: STATIC_BODY_BASE + index + 1,
                position: [
                    center_x,
                    TRAVERSAL_CEILING_BOTTOM + ceiling_half.y,
                    center_z - TRAVERSAL_CEILING_OFFSET,
                ],
                half_extents: vec_to_array(ceiling_half),
                kind: StaticColliderKind::Ceiling,
            };
            game.physics
                .add_fixed(ceiling.id, array_to_vec(ceiling.position), ceiling_half)?;
            game.static_colliders.push(ceiling);
            let bridge_half = TRAVERSAL_BRIDGE_HALF_EXTENTS;
            let bridge = StaticColliderSnapshot {
                id: STATIC_BODY_BASE + index + 2,
                position: [
                    center_x + TRAVERSAL_BRIDGE_OFFSET,
                    TRAVERSAL_BRIDGE_TOP - bridge_half.y,
                    center_z,
                ],
                half_extents: vec_to_array(bridge_half),
                kind: StaticColliderKind::Platform,
            };
            game.physics
                .add_fixed(bridge.id, array_to_vec(bridge.position), bridge_half)?;
            // A box top's area: its walkable surface.
            let area = |collider: &StaticColliderSnapshot| {
                let [x, _, z] = collider.position;
                let [hx, _, hz] = collider.half_extents;
                Some(RegionArea {
                    min_x: x - hx,
                    max_x: x + hx,
                    min_z: z - hz,
                    max_z: z + hz,
                })
            };
            let platform_area = area(&game.static_colliders[game.static_colliders.len() - 2]);
            let bridge_area = area(&bridge);
            game.static_colliders.push(bridge);
            game.topology = Topology::new(
                vec![
                    WalkableRegion {
                        id: DUNGEON_FLOOR_REGION,
                        foot_y: FLOOR_FOOT_Y,
                        area: None,
                    },
                    WalkableRegion {
                        id: TRAVERSAL_PLATFORM_REGION,
                        foot_y: FLOOR_FOOT_Y + 2 * half.y,
                        area: platform_area,
                    },
                    WalkableRegion {
                        id: TRAVERSAL_BRIDGE_REGION,
                        foot_y: TRAVERSAL_BRIDGE_TOP,
                        area: bridge_area,
                    },
                ],
                vec![
                    TraversalConnection {
                        id: 1,
                        kind: ConnectionKind::Jump,
                        from: DUNGEON_FLOOR_REGION,
                        to: TRAVERSAL_PLATFORM_REGION,
                    },
                    TraversalConnection {
                        id: 2,
                        kind: ConnectionKind::Drop,
                        from: TRAVERSAL_PLATFORM_REGION,
                        to: DUNGEON_FLOOR_REGION,
                    },
                    TraversalConnection {
                        id: 3,
                        kind: ConnectionKind::Jump,
                        from: TRAVERSAL_PLATFORM_REGION,
                        to: TRAVERSAL_BRIDGE_REGION,
                    },
                    TraversalConnection {
                        id: 4,
                        kind: ConnectionKind::Drop,
                        from: TRAVERSAL_BRIDGE_REGION,
                        to: DUNGEON_FLOOR_REGION,
                    },
                ],
            )?;
        }
        game.validate_topology()?;
        Ok(game)
    }

    /// Rejects a level whose geometry lets a body cross between regions no connection
    /// joins (ADR 0002): within a jump's rise and its horizontal reach (the full run speed
    /// for the whole airtime, plus a body's width).
    fn validate_topology(&self) -> Result<(), GameError> {
        let jump = &self.content.jump.data;
        let reach = PLAYER_SPEED * i32::from(jump.airtime_ticks) + 2 * PLAYER_HALF_EXTENTS.x;
        self.topology
            .validate_crossings(f32::from(jump.apex_height), reach as f32)
    }

    pub fn scenario(&self) -> ScenarioId {
        self.scenario
    }

    pub fn run_seed(&self) -> RunSeed {
        self.run_seed
    }

    pub fn save_state(&self) -> Result<ArpgSaveState, GameError> {
        if self.workbench.as_ref().is_some_and(Workbench::edited) {
            return Err(GameError::new(
                "a session changed by workbench operations cannot be saved; export a reproduction instead",
            ));
        }
        let players = self
            .players
            .iter()
            .map(|(&id, state)| {
                Ok(PlayerSaveState {
                    id,
                    position: state.position.to_array(),
                    velocity: state.velocity.to_array(),
                    grounded: state.grounded,
                    coyote_ticks_remaining: state.coyote_ticks_remaining,
                    jump_buffer_ticks_remaining: state.jump_buffer_ticks_remaining,
                    region: state.region,
                    last_supported: state.last_supported.to_array(),
                    movement: [state.movement_x, state.movement_z],
                    facing: [state.facing_x, state.facing_z],
                    action: state.action.map(ActionState::snapshot),
                    hurt_ticks_remaining: state.hurt_ticks_remaining,
                    guard: state.guard,
                    counter: state.counter,
                    weapon: state.weapon,
                    draw_ticks: state.draw_ticks,
                    aim: state.aim,
                    locked_monster_id: state.locked_monster_id,
                    health: state.health,
                    experience: state.experience,
                    gold: state.gold,
                    last_sequence: self.last_sequences.get(&id).copied().unwrap_or_default(),
                    last_reliable_sequence: self
                        .last_reliable_sequences
                        .get(&id)
                        .copied()
                        .unwrap_or_default(),
                })
            })
            .collect::<Result<Vec<_>, GameError>>()?;

        Ok(ArpgSaveState {
            schema_version: SAVE_STATE_SCHEMA_VERSION,
            rules_version: SAVE_STATE_RULES_VERSION,
            run_seed: self.run_seed,
            tick: self.tick,
            players,
            rooms: self
                .rooms
                .iter()
                .map(|room| RoomSaveState {
                    id: room.id,
                    encounter_state: room.encounter_state,
                })
                .collect(),
            monsters: self
                .monsters
                .iter()
                .map(|monster| MonsterSaveState {
                    id: monster.id,
                    room_id: monster.room_id,
                    position: monster.position.to_array(),
                    health: monster.health,
                    action: monster.action.map(|action| MonsterActionSaveState {
                        phase: action.phase,
                        ticks_remaining: action.ticks_remaining,
                        target_player_id: action.target_player_id,
                    }),
                    stagger_ticks_remaining: monster.stagger_ticks_remaining,
                    post: monster.post.map(|post| post.to_array()),
                    engagement: monster.engagement,
                    steered: monster.steered,
                    retreating: monster.retreating,
                    guard: monster.guard,
                })
                .collect(),
            ground_loot: self
                .ground_loot
                .iter()
                .map(|loot| GroundLootSaveState {
                    id: loot.id,
                    position: loot.position.to_array(),
                    kind: loot.kind,
                    amount: loot.amount,
                })
                .collect(),
            next_ground_loot_id: self.next_ground_loot_id,
            arrows: self.arrows.iter().map(ArrowState::save).collect(),
            next_arrow_id: self.next_arrow_id,
            scenario: self.scenario,
            opened_chests: self
                .chests
                .iter()
                .filter(|chest| chest.opened)
                .map(|chest| chest.id)
                .collect(),
            content_revision: self.content.revision.clone(),
        })
    }

    /// Restores a save made with the built-in content.
    pub fn from_save_state(save: ArpgSaveState) -> Result<Self, GameError> {
        Self::restore(save, Arc::clone(content()))
    }

    /// Restores a save made with `bundle`. The bundle is validated first, and like
    /// [`ArpgGame::from_save_state`] a save from another content revision is refused.
    pub fn from_save_state_with_content(
        save: ArpgSaveState,
        bundle: ContentBundle,
    ) -> Result<Self, GameError> {
        // Refuse a save from another bundle or format before anything is resolved.
        Self::check_save_versions(save.schema_version, save.rules_version)?;
        let revision = bundle.revision();
        if save.content_revision != revision {
            return Err(GameError::new(format!(
                "save was made with content revision {}; this bundle is {revision}",
                save.content_revision
            )));
        }
        Self::restore(save, Self::load_content(bundle)?)
    }

    fn check_save_versions(schema_version: u16, rules_version: u16) -> Result<(), GameError> {
        if schema_version != SAVE_STATE_SCHEMA_VERSION {
            return Err(GameError::new(format!(
                "unsupported ARPG save schema version {schema_version}; expected {SAVE_STATE_SCHEMA_VERSION}"
            )));
        }
        if rules_version != SAVE_STATE_RULES_VERSION {
            return Err(GameError::new(format!(
                "unsupported ARPG save rules version {rules_version}; expected {SAVE_STATE_RULES_VERSION}"
            )));
        }
        Ok(())
    }

    fn restore(save: ArpgSaveState, content: Arc<Content>) -> Result<Self, GameError> {
        Self::check_save_versions(save.schema_version, save.rules_version)?;
        if save.content_revision != content.revision {
            return Err(GameError::new(format!(
                "save was made with content revision {}; this authority runs {}",
                save.content_revision, content.revision
            )));
        }
        if save.players.len() > MAX_PLAYERS {
            return Err(GameError::new("save contains too many players"));
        }

        let mut game =
            Self::with_scenario_layout(Arc::clone(&content), save.scenario, save.run_seed)?;
        let content: &Content = &content;
        game.tick = save.tick;

        if save.rooms.len() != game.rooms.len() {
            return Err(GameError::new(
                "save room set does not match generated dungeon",
            ));
        }
        let mut room_states = BTreeMap::new();
        for room in save.rooms {
            if room_states.insert(room.id, room.encounter_state).is_some() {
                return Err(GameError::new("save contains duplicate room ids"));
            }
        }
        for room in &mut game.rooms {
            let state = room_states
                .remove(&room.id)
                .ok_or_else(|| GameError::new("save is missing a generated room"))?;
            if room.kind == RoomKind::Start && state != RoomEncounterState::Cleared {
                return Err(GameError::new("start room must remain cleared"));
            }
            room.encounter_state = state;
        }
        if !room_states.is_empty() {
            return Err(GameError::new("save contains unknown room ids"));
        }

        let mut player_ids = BTreeSet::new();
        for player in &save.players {
            if player.id == 0 || !player_ids.insert(player.id) {
                return Err(GameError::new(
                    "save contains invalid or duplicate player ids",
                ));
            }
            Self::validate_axis(player.movement, "movement")?;
            Self::validate_facing(player.facing)?;
            if let Some(aim) = player.aim {
                Self::validate_aim(aim)?;
                // Without a lock the facing follows the aim exactly (a lock's direction is
                // refreshed from positions once the save is loaded).
                let (x, z) = Self::facing_for(aim);
                if player.locked_monster_id.is_none() && player.facing != [x, z] {
                    return Err(GameError::new("saved player facing does not match its aim"));
                }
            }
            if player.health
                > Self::max_health_for_level(
                    content,
                    Self::level_for_experience(content, player.experience),
                )
            {
                return Err(GameError::new(
                    "saved player health exceeds authoritative maximum",
                ));
            }
            if player.hurt_ticks_remaining > PLAYER_HURT_TICKS {
                return Err(GameError::new("saved player hurt reaction is invalid"));
            }
            if let Some(action) = player.action {
                Self::validate_action_snapshot(content, action)?;
            }
            Self::validate_guard(content, player)?;
            Self::validate_loadout(content, player)?;
            if let Some(counter) = player.counter
                && (counter
                    != CounterOpportunity::grant(
                        content.counter_window_ticks,
                        counter.blocked_monster_id,
                        counter.blocked_at_tick,
                    )
                    || counter.expires_at_tick <= save.tick
                    || counter.blocked_at_tick >= save.tick
                    || player.health == 0)
            {
                return Err(GameError::new("saved counter opportunity is invalid"));
            }
            if player.last_sequence == u32::MAX || player.last_reliable_sequence == u32::MAX {
                return Err(GameError::new(
                    "saved command sequence leaves no valid next command",
                ));
            }
            if !game.character_fits(player.position, PLAYER_HALF_EXTENTS) {
                return Err(GameError::new(
                    "saved player position is outside the dungeon",
                ));
            }
            let [velocity_x, velocity_y, velocity_z] = player.velocity;
            let speed = PLAYER_SPEED as f32;
            let jump = &content.jump;
            // The fastest fall is one from a jump's apex above the highest floor to the lowest:
            // the speed of that drop plus a takeoff and a gravity step of discrete integration.
            let rising = jump.takeoff_speed + jump.gravity;
            let falling = rising + (2.0 * jump.gravity * game.highest_reach(content)).sqrt();
            if !(-falling..=rising).contains(&velocity_y)
                || !(-speed..=speed).contains(&velocity_x)
                || !(-speed..=speed).contains(&velocity_z)
            {
                return Err(GameError::new("saved player velocity is out of range"));
            }
            // Heights are checked against the ground physics finds below the body (#208):
            // the floor or the top of a platform.
            let feet = player.position[1] - PLAYER_HALF_EXTENTS.y as f32;
            let lowest = game.topology.lowest_foot() as f32;
            let above_ground = game
                .physics
                .ground_height(Vector::from_array(player.position))
                .map(|ground| feet - ground);
            let support_valid = if player.grounded {
                velocity_y == 0.0
                    && player.coyote_ticks_remaining == 0
                    && above_ground.is_some_and(|height| height.abs() <= GROUNDED_HEIGHT_TOLERANCE)
            } else {
                player.coyote_ticks_remaining <= jump.data.coyote_ticks
                    && above_ground.is_some_and(|height| height >= -GROUNDED_HEIGHT_TOLERANCE)
                    && feet - lowest <= game.highest_reach(content) + GROUNDED_HEIGHT_TOLERANCE
            };
            if !support_valid
                || player.jump_buffer_ticks_remaining > jump.data.buffer_ticks
                || (player.health == 0 && player.jump_buffer_ticks_remaining > 0)
            {
                return Err(GameError::new("saved player jump state is invalid"));
            }
            let region_valid = game.topology.knows(player.region)
                && (!player.grounded
                    || game
                        .topology
                        .supporting(
                            player.position[0],
                            player.position[2],
                            feet,
                            GROUNDED_HEIGHT_TOLERANCE,
                            PLAYER_HALF_EXTENTS.x as f32,
                        )
                        .map(|id| RegionMembership::Region { id })
                        == Some(player.region));
            if !region_valid {
                return Err(GameError::new("saved player region is invalid"));
            }
            // In the world and clear of fixed geometry. A supported body may round an edge,
            // where no ground lies straight below, so support itself is not checked.
            if game.out_of_world(Vector::from_array(player.last_supported))
                || !game.character_fits(player.last_supported, PLAYER_HALF_EXTENTS)
            {
                return Err(GameError::new(
                    "saved player last supported position is invalid",
                ));
            }
        }

        if save.monsters.len() != game.monsters.len() {
            return Err(GameError::new(
                "save monster set does not match generated dungeon",
            ));
        }
        let mut monster_states = BTreeMap::new();
        for monster in save.monsters {
            // A monster's definition follows from the generated dungeon, not the save.
            let definition = game
                .monsters
                .iter()
                .find(|generated| generated.id == monster.id)
                .ok_or_else(|| GameError::new("save contains unknown monster ids"))?
                .definition();
            if monster.health > definition.health {
                return Err(GameError::new(
                    "saved monster health exceeds authoritative maximum",
                ));
            }
            if monster.stagger_ticks_remaining > content.max_stagger_ticks() {
                return Err(GameError::new("saved monster stagger reaction is invalid"));
            }
            let engagement_valid = match monster.engagement {
                Engagement::Idle | Engagement::Returning => true,
                Engagement::Engaged { target_player_id } => target_player_id != 0,
                Engagement::Searching { ticks_remaining } => {
                    (1..=definition.reacquire_ticks).contains(&ticks_remaining)
                }
            };
            if !engagement_valid {
                return Err(GameError::new("saved monster engagement is invalid"));
            }
            if monster.retreating
                && (!monster.steered
                    || definition.retreat_range.is_none()
                    || !matches!(monster.engagement, Engagement::Engaged { .. }))
            {
                return Err(GameError::new("saved monster retreat is invalid"));
            }
            let guard_valid = match monster.guard {
                None => !definition.shield,
                Some(guard) => {
                    let can_guard = monster.health > 0
                        && monster.action.is_none()
                        && monster.stagger_ticks_remaining == 0;
                    definition.shield
                        && guard.save_valid(&content.guard, can_guard)
                        // Only the engagement policy holds a monster's guard, and it lets go
                        // on committing to an attack or on death. A stagger keeps it held
                        // (without a stance) so the guard rises again once free.
                        // An encounter that is not active releases it as well.
                        && (!guard.held
                            || (monster.health > 0
                                && monster.action.is_none()
                                && matches!(monster.engagement, Engagement::Engaged { .. })
                                && game.rooms.iter().any(|room| {
                                    room.id == monster.room_id
                                        && room.encounter_state == RoomEncounterState::Active
                                })))
                }
            };
            if !guard_valid {
                return Err(GameError::new("saved monster guard is invalid"));
            }
            if let Some(action) = monster.action {
                let maximum_ticks = match action.phase {
                    ActionPhase::Windup => definition.windup_ticks,
                    ActionPhase::Active => definition.active_ticks,
                    ActionPhase::Recovery => definition.recovery_ticks,
                };
                if action.ticks_remaining == 0
                    || action.ticks_remaining > maximum_ticks
                    || action.target_player_id == 0
                {
                    return Err(GameError::new("saved monster action is invalid"));
                }
            }
            if monster_states.insert(monster.id, monster).is_some() {
                return Err(GameError::new("save contains duplicate monster ids"));
            }
        }
        for monster in &mut game.monsters {
            let saved = monster_states
                .remove(&monster.id)
                .ok_or_else(|| GameError::new("save is missing a generated monster"))?;
            if saved.room_id != monster.room_id {
                return Err(GameError::new(
                    "saved monster room does not match generated dungeon",
                ));
            }
            let position = Vector::from_array(saved.position);
            let room_contains_monster = game.rooms.iter().any(|room| {
                room.id == saved.room_id
                    && position.is_finite()
                    && position.y == PLAYER_Y as f32
                    && room.contains_xz(position)
            });
            let post_in_room = saved.post.map(Vector::from_array).is_none_or(|post| {
                game.rooms.iter().any(|room| {
                    room.id == saved.room_id
                        && post.is_finite()
                        && post.y == PLAYER_Y as f32
                        && room.contains_xz(post)
                })
            });
            if !room_contains_monster || !post_in_room {
                return Err(GameError::new("saved monster position is outside its room"));
            }
            monster.position = position;
            monster.health = saved.health;
            monster.action = saved.action.map(|action| MonsterActionState {
                phase: action.phase,
                ticks_remaining: action.ticks_remaining,
                target_player_id: action.target_player_id,
            });
            monster.stagger_ticks_remaining = saved.stagger_ticks_remaining;
            monster.post = saved.post.map(Vector::from_array);
            monster.engagement = saved.engagement;
            monster.steered = saved.steered;
            monster.retreating = saved.retreating;
            monster.guard = saved.guard;
        }
        if !monster_states.is_empty() {
            return Err(GameError::new("save contains unknown monster ids"));
        }

        let mut loot_ids = BTreeSet::new();
        let mut ground_loot = Vec::with_capacity(save.ground_loot.len());
        for loot in save.ground_loot {
            let position = Vector::from_array(loot.position);
            if loot.id < GROUND_LOOT_ID_BASE
                || position.y != PLAYER_Y as f32
                || !game.dungeon_contains(position, Vec3i::ZERO)
                || loot.amount != content.loot.monster_gold
                || !loot_ids.insert(loot.id)
                || loot.id >= save.next_ground_loot_id
            {
                return Err(GameError::new("save contains invalid ground loot"));
            }
            ground_loot.push(GroundLootState {
                id: loot.id,
                position,
                kind: loot.kind,
                amount: loot.amount,
            });
        }
        if save.next_ground_loot_id < GROUND_LOOT_ID_BASE {
            return Err(GameError::new(
                "save contains an invalid next ground loot id",
            ));
        }
        game.ground_loot = ground_loot;
        game.next_ground_loot_id = save.next_ground_loot_id;

        if save.next_arrow_id < ARROW_ID_BASE
            || save.arrows.len() > usize::from(content.bow.max_live_arrows)
        {
            return Err(GameError::new("save contains invalid arrow bookkeeping"));
        }
        // Arrows are kept oldest first: the live cap retires the first entry and impacts
        // resolve in this order, so restored ids must strictly increase.
        let mut previous_arrow_id = None;
        for arrow in &save.arrows {
            let in_order = previous_arrow_id.is_none_or(|previous| arrow.id > previous);
            previous_arrow_id = Some(arrow.id);
            // (lifetime, ticks flown before the save tick, launch possible). A player's
            // arrow launched during tick L flies from that tick on; a monster's shot
            // launched during tick L flies from tick L + 1 on.
            let flight = match arrow.source {
                StrikeSource::Player(owner_id) => Some((
                    content.bow.arrow_lifetime_ticks,
                    0,
                    owner_id != 0 && Self::arrow_launch_is_possible(&content.bow, arrow),
                )),
                StrikeSource::Monster(monster_id) => game
                    .monsters
                    .iter()
                    .find(|monster| monster.id == monster_id)
                    .map(MonsterState::definition)
                    .and_then(|definition| {
                        let projectile = definition.projectile?;
                        Some((
                            projectile.lifetime_ticks,
                            1,
                            arrow.damage == definition.damage
                                && projectile_velocity_is_possible(
                                    Vector::from_array(arrow.velocity),
                                    projectile.speed,
                                ),
                        ))
                    }),
            };
            let Some((lifetime, unflown, possible)) = flight else {
                return Err(GameError::new("save contains an invalid arrow"));
            };
            if !in_order
                || arrow.id < ARROW_ID_BASE
                || arrow.id >= save.next_arrow_id
                || arrow.launched_at_tick >= save.tick
                || !(1..=lifetime).contains(&arrow.ticks_remaining)
                || save.tick - arrow.launched_at_tick
                    != u64::from(lifetime - arrow.ticks_remaining) + unflown
                || !possible
                || !game.projectile_height_possible(content, arrow.position[1])
                || !game.dungeon_contains(Vector::from_array(arrow.position), Vec3i::ZERO)
            {
                return Err(GameError::new("save contains an invalid arrow"));
            }
        }
        game.arrows = save.arrows.iter().map(ArrowState::restore).collect();
        game.next_arrow_id = save.next_arrow_id;

        let mut opened = BTreeSet::new();
        for id in save.opened_chests {
            // Opening requires a cleared room, so an opened chest elsewhere is impossible.
            let cleared = game
                .chests
                .iter()
                .find(|chest| chest.id == id)
                .is_some_and(|chest| game.room_cleared(chest.room_id));
            let chest = game
                .chests
                .iter_mut()
                .find(|chest| chest.id == id)
                .filter(|_| cleared && opened.insert(id))
                .ok_or_else(|| GameError::new("save contains an invalid opened chest"))?;
            chest.opened = true;
        }

        for player in save.players {
            game.add_player(player.id)?;
            let position = Vector::from_array(player.position);
            game.physics
                .place_character(Self::player_body_id(player.id), position)?;
            let state = game
                .players
                .get_mut(&player.id)
                .expect("saved player was just added");
            *state = PlayerState {
                position,
                velocity: Vector::from_array(player.velocity),
                grounded: player.grounded,
                coyote_ticks_remaining: player.coyote_ticks_remaining,
                jump_buffer_ticks_remaining: player.jump_buffer_ticks_remaining,
                region: player.region,
                last_supported: Vector::from_array(player.last_supported),
                movement_x: player.movement[0],
                movement_z: player.movement[1],
                facing_x: player.facing[0],
                facing_z: player.facing[1],
                action: player.action.map(|action| ActionState {
                    kind: action.kind,
                    phase: action.phase,
                    ticks_remaining: action.ticks_remaining,
                    facing_x: action.facing[0],
                    facing_z: action.facing[1],
                    connected: action.connected,
                    buffered: action.buffered,
                    charge: action.charge,
                    aim: action.aim,
                }),
                hurt_ticks_remaining: player.hurt_ticks_remaining,
                guard: player.guard,
                counter: player.counter,
                weapon: player.weapon,
                draw_ticks: player.draw_ticks,
                aim: player.aim,
                locked_monster_id: player.locked_monster_id,
                health: player.health,
                experience: player.experience,
                gold: player.gold,
            };
            game.last_sequences.insert(player.id, player.last_sequence);
            game.last_reliable_sequences
                .insert(player.id, player.last_reliable_sequence);
        }

        for room in &game.rooms {
            let living_monster = game
                .monsters
                .iter()
                .any(|monster| monster.room_id == room.id && monster.health > 0);
            if room.encounter_state == RoomEncounterState::Cleared && living_monster {
                return Err(GameError::new(
                    "saved cleared room still contains a living monster",
                ));
            }
        }

        game.sync_door_locks()?;
        let locked_players = game
            .players
            .iter()
            .filter(|(_, state)| state.locked_monster_id.is_some())
            .map(|(&id, _)| id)
            .collect::<Vec<_>>();
        for player_id in locked_players {
            if !game.lock_holds(player_id)? {
                return Err(GameError::new("saved target lock is invalid"));
            }
            // Facing is refreshed toward the lock after physics every tick, so a saved locked
            // player faces its target; anything else would let the first restored tick
            // resolve guards against a contradictory facing.
            if let Some(target) = game.locked_target_position(player_id)?
                && let Some(direction) =
                    Self::direction_between(game.player_position(player_id)?, target)
            {
                let state = &game.players[&player_id];
                if (state.facing_x, state.facing_z) != Self::facing_for(direction) {
                    return Err(GameError::new(
                        "saved player facing does not match its target lock",
                    ));
                }
            }
        }
        Ok(game)
    }

    /// Whether a saved character with `half_extents` lies inside the generated dungeon
    /// without penetrating its floor, walls or pillars. Touching is allowed; its height is
    /// checked against its support state by the caller.
    fn character_fits(&self, position: [f32; 3], half_extents: Vec3i) -> bool {
        let position = Vector::from_array(position);
        let extent = |select: fn(&RoomSnapshot) -> i32, pick: fn(i32, i32) -> i32| {
            self.rooms
                .iter()
                .map(select)
                .reduce(pick)
                .map(|value| value as f32)
        };
        position.is_finite()
            && extent(|room| room.min_x, i32::min).is_some_and(|min| position.x >= min)
            && extent(|room| room.max_x, i32::max).is_some_and(|max| position.x <= max)
            && extent(|room| room.min_z, i32::min).is_some_and(|min| position.z >= min)
            && extent(|room| room.max_z, i32::max).is_some_and(|max| position.z <= max)
            && !self.physics.penetrates_fixed(position, half_extents)
    }

    /// Whether a saved entity with `half_extents` lies inside the generated dungeon without
    /// overlapping its walls, pillars or platforms.
    /// Touching fixed geometry is allowed; penetrating it is not.
    fn dungeon_contains(&self, position: Vector, half_extents: Vec3i) -> bool {
        let extent = |select: fn(&RoomSnapshot) -> i32, pick: fn(i32, i32) -> i32| {
            self.rooms.iter().map(select).reduce(pick).map(f64::from)
        };
        let [x, _, z] = position.to_array().map(f64::from);
        let inside_extents = position.is_finite()
            && extent(|room| room.min_x, i32::min).is_some_and(|min| x >= min)
            && extent(|room| room.max_x, i32::max).is_some_and(|max| x <= max)
            && extent(|room| room.min_z, i32::min).is_some_and(|min| z >= min)
            && extent(|room| room.max_z, i32::max).is_some_and(|max| z <= max);
        let position = position.to_array().map(f64::from);
        let half_extents = vec_to_array(half_extents);
        inside_extents
            && !self.static_colliders.iter().any(|collider| {
                collider.kind != StaticColliderKind::Door
                    && (0..3).all(|axis| {
                        let reach =
                            f64::from(collider.half_extents[axis]) + f64::from(half_extents[axis]);
                        (position[axis] - f64::from(collider.position[axis])).abs() < reach
                    })
            })
    }

    /// Whether a projectile can fly at `height`: shots leave from the shooter's body, which
    /// stands on a walkable floor or is up to a jump's apex above the highest one.
    fn projectile_height_possible(&self, content: &Content, height: f32) -> bool {
        let above = height - PLAYER_Y as f32;
        (0.0..=self.highest_reach(content) + GROUNDED_HEIGHT_TOLERANCE).contains(&above)
    }

    /// How far (world units) a body's feet can rise above the lowest floor: a jump's apex
    /// above the highest walkable floor (#208).
    fn highest_reach(&self, content: &Content) -> f32 {
        (self.topology.highest_foot() - self.topology.lowest_foot()) as f32
            + f32::from(content.jump.data.apex_height)
    }

    fn validate_axis(axis: [i8; 2], label: &str) -> Result<(), GameError> {
        if axis
            .into_iter()
            .any(|component| !(-1..=1).contains(&component))
        {
            return Err(GameError::new(format!(
                "saved {label} axis is outside -1..=1"
            )));
        }
        Ok(())
    }

    /// Aim intent is any non-zero direction whose components lie within the aim limit.
    fn validate_aim(direction: [i16; 2]) -> Result<(), GameError> {
        let limit = -AIM_COMPONENT_LIMIT..=AIM_COMPONENT_LIMIT;
        if direction == [0, 0] || !direction.iter().all(|component| limit.contains(component)) {
            return Err(GameError::new(format!(
                "aim direction must be non-zero with components within ±{AIM_COMPONENT_LIMIT}"
            )));
        }
        Ok(())
    }

    /// The eight-way facing closest to `direction`. A component counts when it is at least
    /// tan 22.5° (≈ 0.41421) of the other, so sector boundaries lie halfway between the
    /// eight directions.
    fn facing_for(direction: [i16; 2]) -> (i8, i8) {
        let (x, z) = (i32::from(direction[0]), i32::from(direction[1]));
        let axis = |own: i32, other: i32| -> i8 {
            if own.abs() * 100_000 < other.abs() * 41_421 {
                return 0;
            }
            match own.cmp(&0) {
                std::cmp::Ordering::Greater => 1,
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
            }
        };
        (axis(x, z), axis(z, x))
    }

    /// The aim direction from `origin` towards `target` on the plane: the larger component
    /// is the full aim limit and the other is rounded to the nearest step of it. `None` when
    /// the positions coincide on the plane.
    fn direction_between(origin: Vector, target: Vector) -> Option<[i16; 2]> {
        let (dx, dz) = xz_delta(origin, target);
        let largest = dx.abs().max(dz.abs());
        if largest == 0.0 || !largest.is_finite() {
            return None;
        }
        let limit = f64::from(AIM_COMPONENT_LIMIT);
        // Within ±limit by construction; `as` saturates in any case.
        let scale = |component: f64| (component * limit / largest).round() as i16;
        Some([scale(dx), scale(dz)])
    }

    /// Arrow velocity along an exact aim direction: `speed` world units per tick, each
    /// component rounded toward zero.
    fn aimed_velocity(direction: [i16; 2], speed: i32) -> (i32, i32) {
        let (x, z) = (i64::from(direction[0]), i64::from(direction[1]));
        // Thousandths keep the length exact enough for short directions such as [1, 1]. The
        // root is rounded up so no component, and so no launch, exceeds the draw speed.
        let squared = (x * x + z * z) * 1_000_000;
        let floor = squared.isqrt();
        let scaled_length = if floor * floor == squared {
            floor
        } else {
            floor + 1
        };
        let component = |value: i64| {
            i32::try_from(value * i64::from(speed) * 1_000 / scaled_length)
                .expect("aimed arrow velocity fits i32")
        };
        (component(x), component(z))
    }

    fn validate_facing(facing: [i8; 2]) -> Result<(), GameError> {
        Self::validate_axis(facing, "facing")?;
        if facing == [0, 0] {
            return Err(GameError::new("saved facing cannot be zero"));
        }
        Ok(())
    }

    /// Whether some accepted draw and direction produce this arrow's velocity and damage.
    /// Facing and aimed launches round each component toward zero, so a launched arrow's
    /// planar speed lies within two units below its draw speed and never above it.
    /// A launch commits whole-unit velocity components, so any other saved value is refused.
    fn arrow_launch_is_possible(bow: &BowData, arrow: &ArrowSaveState) -> bool {
        let whole = arrow
            .velocity
            .iter()
            .all(|component| component.is_finite() && component.fract() == 0.0);
        // Launch speeds are at most `u16::MAX`, far inside `i64` and exact in `f32`.
        whole
            && (bow.min_draw_ticks..=bow.full_draw_ticks).any(|charge| {
                let (speed, damage) = Self::arrow_launch(bow, charge);
                let [vx, vy, vz] = arrow.velocity.map(|component| component as i64);
                let speed = i64::from(speed);
                let length_sq = vx * vx + vz * vz;
                damage == arrow.damage
                    && vy == 0
                    && (speed - 2) * (speed - 2) <= length_sq
                    && length_sq <= speed * speed
            })
    }

    /// Speed and damage of an arrow launched with `charge` draw ticks: the minimum accepted
    /// draw gives the minimum arrow and a full draw the full arrow, linearly in between.
    fn arrow_launch(bow: &BowData, charge: u8) -> (i32, u16) {
        let charge = charge.clamp(bow.min_draw_ticks, bow.full_draw_ticks);
        let progress = i32::from(charge - bow.min_draw_ticks);
        let span = i32::from(bow.full_draw_ticks - bow.min_draw_ticks);
        let (min_speed, full_speed) = (
            i32::from(bow.arrow_min_speed),
            i32::from(bow.arrow_full_speed),
        );
        let speed = min_speed + (full_speed - min_speed) * progress / span;
        let damage = bow.arrow_min_damage
            + u16::try_from(
                i32::from(bow.arrow_full_damage - bow.arrow_min_damage) * progress / span,
            )
            .expect("arrow damage fits u16");
        (speed, damage)
    }

    fn lane_sequences(&self, lane: CommandLane) -> &BTreeMap<PlayerId, u32> {
        match lane {
            CommandLane::Datagram => &self.last_sequences,
            CommandLane::Reliable => &self.last_reliable_sequences,
        }
    }

    fn lane_sequences_mut(&mut self, lane: CommandLane) -> &mut BTreeMap<PlayerId, u32> {
        match lane {
            CommandLane::Datagram => &mut self.last_sequences,
            CommandLane::Reliable => &mut self.last_reliable_sequences,
        }
    }

    fn validate_loadout(content: &Content, player: &PlayerSaveState) -> Result<(), GameError> {
        let bow = player.weapon == Weapon::Bow;
        let draw_valid = player.draw_ticks.is_none_or(|drawn| {
            bow && drawn <= content.bow.full_draw_ticks
                && player.health > 0
                && player.action.is_none()
                && player.hurt_ticks_remaining == 0
        });
        let action_valid = player.action.is_none_or(|action| match action.kind {
            ActionKind::Shoot => {
                // Any hit clears a shot, so a shot can only belong to a live, unhurt archer.
                bow && player.health > 0
                    && player.hurt_ticks_remaining == 0
                    && (content.bow.min_draw_ticks..=content.bow.full_draw_ticks)
                        .contains(&action.charge)
            }
            ActionKind::Interact => action.charge == 0,
            _ => !bow && action.charge == 0,
        });
        let guard_valid = !bow || player.guard.stance.is_none();
        let counter_valid = !bow || player.counter.is_none();
        if !(draw_valid && action_valid && guard_valid && counter_valid) {
            return Err(GameError::new("saved player loadout state is invalid"));
        }
        Ok(())
    }

    fn validate_guard(content: &Content, player: &PlayerSaveState) -> Result<(), GameError> {
        let can_guard = player.health > 0
            && player.action.is_none()
            && player.hurt_ticks_remaining == 0
            && player.grounded;
        if !player.guard.save_valid(&content.guard, can_guard) {
            return Err(GameError::new("saved player guard is invalid"));
        }
        Ok(())
    }

    fn validate_action_snapshot(
        content: &Content,
        action: PlayerActionSnapshot,
    ) -> Result<(), GameError> {
        Self::validate_facing(action.facing)?;
        if let Some(aim) = action.aim {
            Self::validate_aim(aim)?;
            let (x, z) = Self::facing_for(aim);
            if action.facing != [x, z] {
                return Err(GameError::new(
                    "saved player action facing does not match its aim",
                ));
            }
        }
        let maximum_ticks = match action.phase {
            ActionPhase::Windup => action.kind.windup_ticks(content),
            ActionPhase::Active => action.kind.active_ticks(content),
            ActionPhase::Recovery => action.kind.recovery_ticks(content),
        };
        if action.ticks_remaining == 0 || action.ticks_remaining > maximum_ticks {
            return Err(GameError::new("saved player action timing is invalid"));
        }
        let strikes = !matches!(action.kind, ActionKind::Interact);
        if action.connected && (!strikes || action.phase == ActionPhase::Windup) {
            return Err(GameError::new(
                "saved player action hit confirmation is invalid",
            ));
        }
        if let Some(input) = action.buffered {
            let buffered_legally = content.combo(action.kind, input).is_some_and(|transition| {
                action.phase == ActionPhase::Active
                    || (action.phase == ActionPhase::Recovery
                        && action.kind.recovery_ticks(content) - action.ticks_remaining
                            < transition.opens_at)
            });
            if !buffered_legally {
                return Err(GameError::new("saved player combo buffer is invalid"));
            }
        }
        Ok(())
    }

    /// Strike outcomes resolved during the most recent tick, in resolution order.
    pub fn strike_outcomes(&self) -> &[StrikeOutcome] {
        &self.strike_outcomes
    }

    /// Number of physics passes (one Rapier resolution of every character's motion) run by
    /// this game since construction.
    ///
    /// Performance evidence uses this operation count to detect regressions
    /// where one gameplay tick begins stepping physics more than once.
    pub fn physics_steps(&self) -> u64 {
        self.physics_steps
    }

    /// A* node expansions spent on monster pursuit so far.
    pub fn navigation_expansions(&self) -> u64 {
        self.navigation_work.expansions
    }

    /// Monster-pursuit navigation work so far: searches, repaths and physics queries.
    pub fn navigation_work(&self) -> NavigationWork {
        self.navigation_work
    }

    fn monster_body_id(monster_id: u32) -> BodyKey {
        MONSTER_BODY_BASE + u64::from(monster_id)
    }

    fn player_body_id(player_id: PlayerId) -> BodyKey {
        PLAYER_BODY_BASE + u64::from(player_id)
    }

    fn level_for_experience(content: &Content, experience: u32) -> u16 {
        let level = 1u32.saturating_add(experience / content.progression.experience_per_level);
        u16::try_from(level.min(u32::from(u16::MAX))).expect("clamped level must fit u16")
    }

    fn max_health_for_level(content: &Content, level: u16) -> u16 {
        content.progression.base_max_health.saturating_add(
            level
                .saturating_sub(1)
                .saturating_mul(content.progression.max_health_per_level),
        )
    }

    fn attack_damage_for_level(content: &Content, level: u16) -> u16 {
        content.progression.base_attack_damage.saturating_add(
            level
                .saturating_sub(1)
                .saturating_mul(content.progression.attack_damage_per_level),
        )
    }

    fn movement_target_velocity(state: PlayerState) -> Vector {
        let x = i32::from(state.movement_x.clamp(-1, 1));
        let z = i32::from(state.movement_z.clamp(-1, 1));
        if x != 0 && z != 0 {
            Vec3i::new(x * PLAYER_DIAGONAL_SPEED, 0, z * PLAYER_DIAGONAL_SPEED).into()
        } else {
            Vec3i::new(x * PLAYER_SPEED, 0, z * PLAYER_SPEED).into()
        }
    }

    fn player_reaction(state: &PlayerState) -> Option<PlayerReactionSnapshot> {
        [
            (PlayerReactionKind::Hurt, state.hurt_ticks_remaining),
            (
                PlayerReactionKind::GuardBroken,
                state.guard.broken_ticks_remaining,
            ),
            (
                PlayerReactionKind::Blocked,
                state.guard.block_reaction_ticks_remaining,
            ),
        ]
        .into_iter()
        .find(|(_, ticks)| *ticks > 0)
        .map(|(kind, ticks_remaining)| PlayerReactionSnapshot {
            kind,
            ticks_remaining,
        })
    }

    fn advance_draw(bow: &BowData, state: &mut PlayerState) {
        if state.health == 0 || state.hurt_ticks_remaining > 0 || state.action.is_some() {
            state.draw_ticks = None;
        } else if let Some(drawn) = state.draw_ticks.as_mut() {
            *drawn = (*drawn + 1).min(bow.full_draw_ticks);
        }
    }

    fn advance_guard(tuning: &GuardData, state: &mut PlayerState) {
        if state.health == 0 {
            state.counter = None;
        }
        // Guarding needs the ground (#59): airborne, the held guard waits for the landing.
        let free = state.action.is_none()
            && state.hurt_ticks_remaining == 0
            && state.weapon == Weapon::SwordAndShield
            && state.grounded;
        state.guard.advance(tuning, state.health > 0, free);
    }

    /// A shield monster's guard tick (#92): the shared timeline, where a monster is free
    /// when its encounter runs and it is neither staggered nor attacking.
    fn advance_monster_guard(tuning: &GuardData, monster: &mut MonsterState, active: bool) {
        let free = active && monster.stagger_ticks_remaining == 0 && monster.action.is_none();
        let alive = monster.health > 0;
        if let Some(guard) = monster.guard.as_mut() {
            guard.advance(tuning, alive, free);
        }
    }

    /// Where a shield monster's guard faces: toward its engaged target (#92). `None` when
    /// it is engaged with nobody it can face, so no strike meets its shield.
    fn monster_guard_facing(&self, monster: &MonsterState) -> Option<(f64, f64)> {
        let target = monster.engaged_target()?;
        Some(xz_delta(
            monster.position,
            self.players.get(&target)?.position,
        ))
    }

    /// The horizontal velocity a player intends this tick: last tick's resolved velocity
    /// converging on the movement target by the bounded acceleration, braking and reversal
    /// steps of `control` (ground or air).
    fn controlled_movement_velocity(
        current: Vector,
        state: PlayerState,
        control: ControlRates,
    ) -> Vector {
        if state.health == 0 || state.action.is_some() {
            return Vector::ZERO;
        }
        let target = Self::movement_target_velocity(state);
        let changing_x = current.x != target.x;
        let changing_z = current.z != target.z;
        let changing_two_axes = changing_x && changing_z;
        Vector::new(
            Self::approach_controlled_axis(current.x, target.x, changing_two_axes, control),
            0.0,
            Self::approach_controlled_axis(current.z, target.z, changing_two_axes, control),
        )
    }

    /// The control rates for a player on the ground or in the air.
    fn control_rates(&self, grounded: bool) -> ControlRates {
        if grounded {
            ControlRates::GROUND
        } else {
            let jump = &self.content.jump.data;
            ControlRates {
                acceleration: i32::from(jump.air_acceleration_per_tick),
                braking: i32::from(jump.air_braking_per_tick),
                reversal: i32::from(jump.air_reversal_per_tick),
            }
        }
    }

    fn approach_controlled_axis(
        current: f32,
        target: f32,
        changing_two_axes: bool,
        control: ControlRates,
    ) -> f32 {
        let maximum_delta = if target == 0.0 {
            control.braking
        } else if current != 0.0 && current.signum() != target.signum() {
            control.reversal
        } else {
            control.acceleration
        };
        let maximum_delta =
            Self::normalized_axis_control_delta(maximum_delta, changing_two_axes) as f32;
        if current < target {
            (current + maximum_delta).min(target)
        } else if current > target {
            (current - maximum_delta).max(target)
        } else {
            current
        }
    }

    fn normalized_axis_control_delta(maximum_delta: i32, changing_two_axes: bool) -> i32 {
        if !changing_two_axes {
            return maximum_delta;
        }
        let maximum_squared = maximum_delta * maximum_delta;
        let mut component = maximum_delta;
        while component > 0 && 2 * component * component > maximum_squared {
            component -= 1;
        }
        component
    }

    fn room_at_position(&self, position: Vector) -> Option<RoomId> {
        self.rooms
            .iter()
            .find(|room| room.contains_xz_with_margin(position, ROOM_ACTIVATION_MARGIN))
            .map(|room| room.id)
    }

    fn award_experience(&mut self, player_id: PlayerId, amount: u32) -> Result<(), GameError> {
        let state = self
            .players
            .get_mut(&player_id)
            .ok_or_else(|| GameError::new("experience references an unknown player"))?;
        let previous_level = Self::level_for_experience(&self.content, state.experience);
        let previous_max_health = Self::max_health_for_level(&self.content, previous_level);
        state.experience = state.experience.saturating_add(amount);
        let level = Self::level_for_experience(&self.content, state.experience);
        let max_health = Self::max_health_for_level(&self.content, level);
        if max_health > previous_max_health {
            state.health = state
                .health
                .saturating_add(max_health - previous_max_health)
                .min(max_health);
        }
        Ok(())
    }

    fn start_action(&mut self, player_id: PlayerId, kind: ActionKind) -> Result<(), GameError> {
        let aim = self.intent_aim(player_id)?;
        let state = self
            .players
            .get_mut(&player_id)
            .ok_or_else(|| GameError::new("action references an unknown player"))?;
        if state.health == 0 || state.action.is_some() {
            return Ok(());
        }
        // Committing to an action lowers the shield (a held guard rises again afterwards)
        // and lowers a drawn bow without shooting.
        state.guard.stance = None;
        state.draw_ticks = None;
        state.action = Some(Self::new_action(&self.content, kind, state, aim));
        Ok(())
    }

    /// Commits a new action. With `aim` (from aim intent or a held target lock) the action
    /// strikes or shoots along that exact direction and faces its nearest eight-way facing;
    /// without it the action keeps the player's current facing.
    fn new_action(
        content: &Content,
        kind: ActionKind,
        state: &PlayerState,
        aim: Option<[i16; 2]>,
    ) -> ActionState {
        let (facing_x, facing_z) = aim.map_or((state.facing_x, state.facing_z), Self::facing_for);
        ActionState {
            kind,
            phase: ActionPhase::Windup,
            ticks_remaining: kind.windup_ticks(content),
            facing_x,
            facing_z,
            connected: false,
            buffered: None,
            charge: 0,
            aim,
        }
    }

    /// Applies an attack input to an action in progress under the combo transition table.
    fn combo_input(
        content: &Content,
        state: &mut PlayerState,
        input: ComboInput,
        aim: Option<[i16; 2]>,
    ) {
        let Some(mut action) = state.action else {
            return;
        };
        let Some(transition) = content.combo(action.kind, input) else {
            return;
        };
        match action.phase {
            ActionPhase::Windup => {}
            ActionPhase::Active => {
                action.buffered.get_or_insert(input);
                state.action = Some(action);
            }
            ActionPhase::Recovery => {
                let elapsed = action.kind.recovery_ticks(content) - action.ticks_remaining;
                if elapsed < transition.opens_at {
                    action.buffered.get_or_insert(input);
                    state.action = Some(action);
                } else if elapsed < transition.closes_at
                    && (!transition.requires_hit || action.connected)
                {
                    state.action = Some(Self::new_action(content, transition.to, state, aim));
                }
            }
        }
    }

    /// Commits a buffered combo input on the tick its transition interval opens.
    fn commit_buffered_combo(content: &Content, state: &mut PlayerState, aim: Option<[i16; 2]>) {
        let Some(action) = state.action else {
            return;
        };
        let Some(input) = action.buffered else {
            return;
        };
        if action.phase != ActionPhase::Recovery {
            return;
        }
        let Some(transition) = content.combo(action.kind, input) else {
            return;
        };
        let elapsed = action.kind.recovery_ticks(content) - action.ticks_remaining;
        if elapsed < transition.opens_at {
            return;
        }
        state.action = if transition.requires_hit && !action.connected {
            // The branch was not earned: drop the intent and keep recovering.
            Some(ActionState {
                buffered: None,
                ..action
            })
        } else {
            Some(Self::new_action(content, transition.to, state, aim))
        };
    }

    /// Whether `(dx, dz)` lies within 60° of the direction `(facing_x, facing_z)`, which may
    /// be an eight-way facing or an exact aim direction.
    fn target_is_in_front(facing: (f64, f64), (dx, dz): (f64, f64)) -> bool {
        let (facing_x, facing_z) = facing;
        let distance_sq = dx * dx + dz * dz;
        if distance_sq == 0.0 {
            return true;
        }
        let facing_len_sq = facing_x * facing_x + facing_z * facing_z;
        if facing_len_sq == 0.0 {
            return true;
        }
        let dot = dx * facing_x + dz * facing_z;
        dot > 0.0 && 4.0 * dot * dot >= distance_sq * facing_len_sq
    }

    fn spawn_ground_loot(&mut self, position: Vector) -> Result<(), GameError> {
        let id = self.next_ground_loot_id;
        self.next_ground_loot_id = self
            .next_ground_loot_id
            .checked_add(1)
            .ok_or_else(|| GameError::new("ground loot id overflow"))?;
        self.ground_loot.push(GroundLootState {
            id,
            position,
            kind: LootKind::Gold,
            amount: self.content.loot.monster_gold,
        });
        Ok(())
    }

    /// Next ordinal in this tick's shared strike/interaction event order.
    fn next_event_order(&self) -> u32 {
        u32::try_from(self.strike_outcomes.len() + self.interaction_events.len())
            .expect("per-tick event count fits u32")
    }

    fn push_strike_outcome(&mut self, outcome: StrikeOutcome) {
        let order = self.next_event_order();
        self.strike_outcomes
            .push(StrikeOutcome { order, ..outcome });
    }

    /// The encounter a combat room runs: generation assigns it from the combat room's
    /// position among the generated rooms, never from the published id.
    fn room_encounter(&self, room_id: RoomId) -> Option<&EncounterDefinition> {
        self.rooms
            .iter()
            .filter(|room| room.kind == RoomKind::Combat)
            .position(|room| room.id == room_id)
            .map(|index| self.content.room_encounter(index))
    }

    /// Gold the reward chest of `room_id` pays: its encounter's reward.
    fn room_reward_gold(&self, room_id: RoomId) -> u32 {
        self.room_encounter(room_id)
            .map(|encounter| {
                let reward = self.content.reward(encounter.reward);
                match reward.kind {
                    content::RewardKind::Chest => reward.gold,
                }
            })
            .expect("only combat rooms have reward chests")
    }

    fn room_cleared(&self, room_id: RoomId) -> bool {
        self.rooms
            .iter()
            .any(|room| room.id == room_id && room.encounter_state == RoomEncounterState::Cleared)
    }

    /// Every interactable within reach of `position`, nearest first. Equal distances prefer
    /// loot over chests and then the lower id. `usable` is false for a locked chest.
    fn interaction_candidates(
        &self,
        membership: RegionMembership,
        position: Vector,
    ) -> Vec<(f64, InteractionTarget, Vector, bool)> {
        // Loot and chests lie on the base floor; interaction is region-local (#207).
        if !self.reaches_base(membership) {
            return Vec::new();
        }
        let distance_sq = |at: Vector| xz_distance_sq(position, at);
        let loot = self.ground_loot.iter().map(|loot| {
            (
                distance_sq(loot.position),
                InteractionTarget::Loot(loot.id),
                loot.position,
                true,
            )
        });
        let chests = self
            .chests
            .iter()
            .filter(|chest| !chest.opened)
            .map(|chest| {
                let at = vector(array_to_vec(chest.position));
                (
                    distance_sq(at),
                    InteractionTarget::Chest(chest.id),
                    at,
                    self.room_cleared(chest.room_id),
                )
            });
        let mut candidates = loot
            .chain(chests)
            .filter(|(distance, ..)| within_range(*distance, INTERACT_RANGE))
            .collect::<Vec<_>>();
        candidates.sort_unstable_by(|(left, left_target, ..), (right, right_target, ..)| {
            left.total_cmp(right).then(left_target.cmp(right_target))
        });
        candidates
    }

    /// The interaction `Interact` would perform for this player right now, if any. The
    /// browser shows prompts from this, never from its own radius.
    fn interaction_choice(
        &self,
        membership: RegionMembership,
        position: Vector,
    ) -> Result<Result<InteractionTarget, InteractionRefusal>, GameError> {
        let mut refusal = InteractionRefusal::NothingInRange;
        for (_, target, at, usable) in self.interaction_candidates(membership, position) {
            if self.strike_obstructed(position, at)? {
                if refusal == InteractionRefusal::NothingInRange {
                    refusal = InteractionRefusal::Obstructed;
                }
                continue;
            }
            if !usable {
                refusal = InteractionRefusal::ChestLocked;
                continue;
            }
            return Ok(Ok(target));
        }
        Ok(Err(refusal))
    }

    fn resolve_interaction(&mut self, player_id: PlayerId) -> Result<(), GameError> {
        let player_position = self.player_position(player_id)?;
        let membership = self.membership(StrikeTarget::Player(player_id));
        let result = match self.interaction_choice(membership, player_position)? {
            Err(reason) => InteractionResult::Refused { reason },
            Ok(target @ InteractionTarget::Loot(id)) => {
                let index = self
                    .ground_loot
                    .iter()
                    .position(|loot| loot.id == id)
                    .expect("candidate loot exists");
                let loot = self.ground_loot.remove(index);
                match loot.kind {
                    LootKind::Gold => InteractionResult::PickedUp {
                        target,
                        gold: loot.amount,
                    },
                }
            }
            Ok(target @ InteractionTarget::Chest(id)) => {
                let chest = self
                    .chests
                    .iter_mut()
                    .find(|chest| chest.id == id)
                    .expect("candidate chest exists");
                chest.opened = true;
                let room_id = chest.room_id;
                InteractionResult::Opened {
                    target,
                    gold: self.room_reward_gold(room_id),
                }
            }
        };
        let mut result = result;
        if let InteractionResult::PickedUp { gold, .. } | InteractionResult::Opened { gold, .. } =
            &mut result
        {
            let player = self
                .players
                .get_mut(&player_id)
                .ok_or_else(|| GameError::new("interaction references an unknown player"))?;
            let before = player.gold;
            player.gold = player.gold.saturating_add(*gold);
            // Publish what was actually credited (gold saturates at its maximum).
            *gold = player.gold - before;
        }
        let order = self.next_event_order();
        self.interaction_events.push(InteractionEventSnapshot {
            order,
            player_id,
            result,
        });
        Ok(())
    }

    /// Orders the eligible candidates inside a strike volume nearest-first (stable target
    /// identity breaks ties) and resolves obstruction for each until `max_targets` connect.
    /// Ordinary strikes are region-local (#207): a candidate in another region is not
    /// eligible, whatever its horizontal distance.
    fn strike_contacts(
        &self,
        definition: StrikeDefinition,
        source: StrikeTarget,
        origin: Vector,
        facing: (f64, f64),
        candidates: impl IntoIterator<Item = (StrikeTarget, Vector)>,
    ) -> Result<Vec<(StrikeTarget, bool)>, GameError> {
        let source = self.membership(source);
        let mut inside = candidates
            .into_iter()
            .filter(|&(target, _)| self.topology.local(source, self.membership(target)))
            .filter_map(|(target, position)| {
                let distance_sq = xz_distance_sq(origin, position);
                let in_arc = !definition.frontal
                    || Self::target_is_in_front(facing, xz_delta(origin, position));
                (within_range(distance_sq, definition.reach) && in_arc).then_some((
                    distance_sq,
                    target,
                    position,
                ))
            })
            .collect::<Vec<_>>();
        inside.sort_unstable_by(|(left, left_target, _), (right, right_target, _)| {
            left.total_cmp(right).then(left_target.cmp(right_target))
        });

        let mut contacts = Vec::new();
        let mut connected = 0;
        for (_, target, position) in inside {
            if connected == definition.max_targets {
                break;
            }
            let obstructed = self.strike_obstructed(origin, position)?;
            if !obstructed {
                connected += 1;
            }
            contacts.push((target, obstructed));
        }
        Ok(contacts)
    }

    /// Whether fixed geometry (walls, pillars, closed doors) lies on the segment from the
    /// attacker to the target on the plane, using Rapier's ray query over the current world.
    /// Characters never block a strike.
    fn strike_obstructed(&self, origin: Vector, target: Vector) -> Result<bool, GameError> {
        let to = Vector::new(target.x, origin.y, target.z);
        Ok(self.physics.segment_obstructed(origin, to))
    }

    /// Where an actor belongs (#207). Monsters move on the base floor; a missing player
    /// counts as there too, since rules only ask about actors they found.
    fn membership(&self, actor: StrikeTarget) -> RegionMembership {
        let base = RegionMembership::Region {
            id: self.topology.base(),
        };
        match actor {
            StrikeTarget::Player(id) => self.players.get(&id).map_or(base, |state| state.region),
            StrikeTarget::Monster(_) => base,
        }
    }

    /// Whether ordinary rules let a player with `membership` reach what stands on the base
    /// floor (monsters, loot, chests): region-local by default (#207).
    fn reaches_base(&self, membership: RegionMembership) -> bool {
        self.topology.local(
            membership,
            RegionMembership::Region {
                id: self.topology.base(),
            },
        )
    }

    /// The player's authoritative physical position, which gameplay rules read directly.
    fn player_position(&self, player_id: PlayerId) -> Result<Vector, GameError> {
        self.players
            .get(&player_id)
            .map(|state| state.position)
            .ok_or_else(|| GameError::new("unknown player"))
    }

    /// Whether a target lock may hold `monster` from `origin` for a player with
    /// `membership`: the monster is alive, in the player's region, its encounter is active and
    /// it is within `range` on the plane.
    fn monster_lockable(
        &self,
        monster: &MonsterState,
        membership: RegionMembership,
        origin: Vector,
        range: i64,
    ) -> bool {
        monster.health > 0
            && self.reaches_base(membership)
            && self.rooms.iter().any(|room| {
                room.id == monster.room_id && room.encounter_state == RoomEncounterState::Active
            })
            && within_range(xz_distance_sq(origin, monster.position), range)
    }

    /// Where a player's lock currently points, revalidated now: `None` when there is no
    /// lock, the player is defeated, or the target died, left its active encounter or moved
    /// beyond the break range. Line of sight is not required to keep a lock (stickiness).
    fn locked_target_position(&self, player_id: PlayerId) -> Result<Option<Vector>, GameError> {
        let Some(state) = self.players.get(&player_id) else {
            return Ok(None);
        };
        let Some(monster_id) = state.locked_monster_id.filter(|_| state.health > 0) else {
            return Ok(None);
        };
        let origin = self.player_position(player_id)?;
        Ok(self
            .monsters
            .iter()
            .find(|monster| monster.id == monster_id)
            .filter(|monster| {
                self.monster_lockable(
                    monster,
                    state.region,
                    origin,
                    self.content.targeting.break_range,
                )
            })
            .map(|monster| monster.position))
    }

    fn lock_holds(&self, player_id: PlayerId) -> Result<bool, GameError> {
        Ok(self.locked_target_position(player_id)?.is_some())
    }

    /// The exact direction a newly committed action takes: towards a valid target lock,
    /// else the player's aim intent. `None` keeps the default committed facing. A lock lost
    /// here falls back silently, so the action still happens.
    fn intent_aim(&self, player_id: PlayerId) -> Result<Option<[i16; 2]>, GameError> {
        let locked = match self.locked_target_position(player_id)? {
            Some(target) => Self::direction_between(self.player_position(player_id)?, target),
            None => None,
        };
        Ok(locked.or_else(|| self.players.get(&player_id).and_then(|state| state.aim)))
    }

    /// Applies `SetAim`: validates `direction`, makes it the held aim intent and refreshes
    /// the facing. Nothing changes when the direction is rejected.
    fn set_aim(
        &mut self,
        player_id: PlayerId,
        direction: Option<[i16; 2]>,
    ) -> Result<(), GameError> {
        if let Some(direction) = direction {
            Self::validate_aim(direction)?;
        }
        self.players
            .get_mut(&player_id)
            .ok_or_else(|| GameError::new("aim references an unknown player"))?
            .aim = direction;
        self.refresh_facing(player_id)
    }

    /// Points the facing at the lock, else the aim intent, else the movement direction
    /// (unchanged while standing still without aim).
    fn refresh_facing(&mut self, player_id: PlayerId) -> Result<(), GameError> {
        let locked = match self.locked_target_position(player_id)? {
            Some(target) => Self::direction_between(self.player_position(player_id)?, target),
            None => None,
        };
        let state = self
            .players
            .get_mut(&player_id)
            .ok_or_else(|| GameError::new("facing references an unknown player"))?;
        if let Some(direction) = locked.or(state.aim) {
            (state.facing_x, state.facing_z) = Self::facing_for(direction);
        } else if state.movement_x != 0 || state.movement_z != 0 {
            state.facing_x = state.movement_x;
            state.facing_z = state.movement_z;
        }
        Ok(())
    }

    /// Locks the nearest eligible monster or advances an existing lock to the next one.
    /// Eligible monsters are lockable within the lock range and in line of sight (no fixed
    /// geometry between), ordered nearest first with the lower id breaking ties. A lock held
    /// outside that set (sticky beyond the lock range) moves to the nearest; with no eligible
    /// monster the current lock is kept.
    fn cycle_target(&mut self, player_id: PlayerId) -> Result<(), GameError> {
        if self
            .players
            .get(&player_id)
            .is_none_or(|state| state.health == 0)
        {
            return Ok(());
        }
        let origin = self.player_position(player_id)?;
        let range = self.content.targeting.lock_range;
        let mut candidates = Vec::new();
        for monster in &self.monsters {
            if self.monster_lockable(
                monster,
                self.membership(StrikeTarget::Player(player_id)),
                origin,
                range,
            ) && !self.strike_obstructed(origin, monster.position)?
            {
                candidates.push((xz_distance_sq(origin, monster.position), monster.id));
            }
        }
        candidates.sort_unstable_by(|(left, left_id), (right, right_id)| {
            left.total_cmp(right).then(left_id.cmp(right_id))
        });
        let current = self.players[&player_id].locked_monster_id;
        let next = match current.and_then(|id| candidates.iter().position(|&(_, c)| c == id)) {
            Some(index) => candidates.get((index + 1) % candidates.len()),
            None => candidates.first(),
        };
        if let Some(&(_, monster_id)) = next {
            self.players
                .get_mut(&player_id)
                .expect("player existence checked")
                .locked_monster_id = Some(monster_id);
        }
        Ok(())
    }

    /// Drops locks that no longer hold and turns locked players towards their targets.
    fn face_held_locks(&mut self) -> Result<(), GameError> {
        let locked = self
            .players
            .iter()
            .filter(|(_, state)| state.locked_monster_id.is_some())
            .map(|(&id, _)| id)
            .collect::<Vec<_>>();
        for player_id in locked {
            if self.lock_holds(player_id)? {
                self.refresh_facing(player_id)?;
            }
        }
        Ok(())
    }

    fn revalidate_target_locks(&mut self) -> Result<(), GameError> {
        let locked = self
            .players
            .iter()
            .filter(|(_, state)| state.locked_monster_id.is_some())
            .map(|(&id, _)| id)
            .collect::<Vec<_>>();
        for player_id in locked {
            if !self.lock_holds(player_id)? {
                self.players
                    .get_mut(&player_id)
                    .expect("player id came from player map")
                    .locked_monster_id = None;
            }
            self.refresh_facing(player_id)?;
        }
        Ok(())
    }

    fn resolve_attack(
        &mut self,
        player_id: PlayerId,
        kind: ActionKind,
        facing_x: i8,
        facing_z: i8,
        aim: Option<[i16; 2]>,
    ) -> Result<(), GameError> {
        let player_position = self.player_position(player_id)?;
        let player_level = Self::level_for_experience(
            &self.content,
            self.players
                .get(&player_id)
                .ok_or_else(|| GameError::new("attack references an unknown player"))?
                .experience,
        );
        let action = self.content.action(kind);
        let Some(definition) = action.strike else {
            return Ok(());
        };
        let (damage_numerator, damage_denominator, stagger_ticks) = (
            action.damage_numerator,
            action.damage_denominator,
            action.stagger_ticks,
        );
        let attack_damage = Self::attack_damage_for_level(&self.content, player_level)
            .saturating_mul(damage_numerator)
            / damage_denominator.max(1);
        let active_rooms = self
            .rooms
            .iter()
            .filter(|room| room.encounter_state == RoomEncounterState::Active)
            .map(|room| room.id)
            .collect::<BTreeSet<_>>();
        let candidates = self
            .monsters
            .iter()
            .filter(|monster| monster.health > 0 && active_rooms.contains(&monster.room_id))
            .map(|monster| (StrikeTarget::Monster(monster.id), monster.position))
            .collect::<Vec<_>>();
        let direction = aim.map_or((f64::from(facing_x), f64::from(facing_z)), |aim| {
            (f64::from(aim[0]), f64::from(aim[1]))
        });
        let contacts = self.strike_contacts(
            definition,
            StrikeTarget::Player(player_id),
            player_position,
            direction,
            candidates,
        )?;
        let strike = StrikeId {
            source: StrikeSource::Player(player_id),
            tick: self.tick,
        };

        let mut defeated_positions = Vec::new();
        for (target, obstructed) in contacts {
            let StrikeTarget::Monster(monster_id) = target else {
                continue;
            };
            let result = if obstructed {
                StrikeResult::Obstructed
            } else {
                let guard_facing = self
                    .monsters
                    .iter()
                    .find(|monster| monster.id == monster_id)
                    .and_then(|monster| self.monster_guard_facing(monster));
                let guard_tuning = self.tuning.guard;
                let monster = self
                    .monsters
                    .iter_mut()
                    .find(|monster| monster.id == monster_id)
                    .ok_or_else(|| GameError::new("strike references an unknown monster"))?;
                // A shield monster's raised guard resolves first, exactly as a player's.
                let at = monster.position;
                let guarded = match (monster.guard.as_mut(), guard_facing) {
                    (Some(guard), Some(facing)) => Self::guard_outcome(
                        &guard_tuning,
                        guard,
                        facing,
                        definition,
                        at,
                        player_position,
                    ),
                    _ => None,
                };
                if let Some(result) = guarded {
                    monster.provoke(player_id);
                    self.push_strike_outcome(StrikeOutcome {
                        order: 0,
                        strike,
                        definition: self.content.id(definition.id).clone(),
                        target,
                        result,
                    });
                    continue;
                }
                let previous_health = monster.health;
                monster.health = monster.health.saturating_sub(attack_damage);
                // A hit lowers a shield, as a hurt reaction does a player's.
                if let Some(guard) = monster.guard.as_mut() {
                    guard.stance = None;
                }
                if monster.health > 0 {
                    monster.stagger_ticks_remaining = stagger_ticks;
                    monster.action = None;
                    monster.provoke(player_id);
                }
                let defeated = previous_health > 0 && monster.health == 0;
                if defeated {
                    defeated_positions.push((monster.position, monster.definition()));
                }
                StrikeResult::Hit {
                    damage: previous_health - monster.health,
                    defeated,
                }
            };
            self.push_strike_outcome(StrikeOutcome {
                order: 0,
                strike,
                definition: self.content.id(definition.id).clone(),
                target,
                result,
            });
        }
        let connected = self.strike_outcomes.iter().any(|outcome| {
            outcome.strike == strike && matches!(outcome.result, StrikeResult::Hit { .. })
        });
        if connected
            && let Some(action) = self
                .players
                .get_mut(&player_id)
                .and_then(|state| state.action.as_mut())
        {
            action.connected = true;
        }
        for (position, defeated) in defeated_positions {
            self.award_experience(player_id, defeated.experience_reward)?;
            self.spawn_ground_loot(position)?;
        }
        self.reconcile_encounters()
    }

    fn advance_actions(&mut self) -> Result<(), GameError> {
        let player_ids = self.players.keys().copied().collect::<Vec<_>>();
        for player_id in player_ids {
            // A combo step committed this tick re-reads the aim intent and target lock.
            let aim = self.intent_aim(player_id)?;
            let effect = {
                let state = self
                    .players
                    .get_mut(&player_id)
                    .expect("player id came from player map");
                let Some(mut action) = state.action else {
                    continue;
                };
                if action.ticks_remaining > 1 {
                    action.ticks_remaining -= 1;
                    state.action = Some(action);
                    Self::commit_buffered_combo(&self.content, state, aim);
                    None
                } else {
                    match action.phase {
                        ActionPhase::Windup => {
                            action.phase = ActionPhase::Active;
                            action.ticks_remaining = action.kind.active_ticks(&self.content);
                            state.action = Some(action);
                            Some((action.kind, action.facing_x, action.facing_z, action.aim))
                        }
                        ActionPhase::Active => {
                            action.phase = ActionPhase::Recovery;
                            action.ticks_remaining = action.kind.recovery_ticks(&self.content);
                            state.action = Some(action);
                            Self::commit_buffered_combo(&self.content, state, aim);
                            None
                        }
                        ActionPhase::Recovery => {
                            state.action = None;
                            None
                        }
                    }
                }
            };
            if let Some((kind, facing_x, facing_z, aim)) = effect {
                match kind {
                    ActionKind::PrimaryAttack
                    | ActionKind::SecondaryAttack
                    | ActionKind::Counter
                    | ActionKind::LightFollowUp
                    | ActionKind::LightFinisher
                    | ActionKind::HeavyFinisher => {
                        self.resolve_attack(player_id, kind, facing_x, facing_z, aim)?;
                    }
                    ActionKind::Interact => self.resolve_interaction(player_id)?,
                    ActionKind::Shoot => self.launch_arrow(player_id, facing_x, facing_z, aim)?,
                }
            }
        }
        Ok(())
    }

    /// Launches one arrow from the shooter's centre along the direction committed at release
    /// (exact aim, else the eight-way facing). The launch point is the authoritative body
    /// centre, never a visual bow socket that could protrude through a wall.
    fn launch_arrow(
        &mut self,
        player_id: PlayerId,
        facing_x: i8,
        facing_z: i8,
        aim: Option<[i16; 2]>,
    ) -> Result<(), GameError> {
        let position = self.player_position(player_id)?;
        let charge = self
            .players
            .get(&player_id)
            .and_then(|state| state.action)
            .map(|action| action.charge)
            .ok_or_else(|| GameError::new("shot has no committed action"))?;
        let (speed, damage) = Self::arrow_launch(&self.tuning.bow, charge);
        let (velocity_x, velocity_z) = match aim {
            Some(aim) => Self::aimed_velocity(aim, speed),
            None => {
                let (fx, fz) = (i32::from(facing_x), i32::from(facing_z));
                let axis_speed = if fx != 0 && fz != 0 {
                    speed * ARROW_DIAGONAL_NUMERATOR / ARROW_DIAGONAL_DENOMINATOR
                } else {
                    speed
                };
                (fx * axis_speed, fz * axis_speed)
            }
        };
        if self.arrows.len() >= usize::from(self.tuning.bow.max_live_arrows) {
            self.arrows.remove(0);
        }
        let id = self.next_arrow_id;
        self.next_arrow_id = id
            .checked_add(1)
            .ok_or_else(|| GameError::new("arrow id overflow"))?;
        self.arrows.push(ArrowState {
            id,
            source: StrikeSource::Player(player_id),
            launched_at_tick: self.tick,
            position,
            velocity: vector(Vec3i::new(velocity_x, 0, velocity_z)),
            damage,
            ticks_remaining: self.tuning.bow.arrow_lifetime_ticks,
        });
        Ok(())
    }

    /// Moves every projectile along its committed segment for one tick. Rapier's ray query
    /// over fixed geometry and ray casts against the opposing side's hurt boxes choose the
    /// first contact: a player's arrow meets monster hurt boxes, a monster's shot player hurt
    /// boxes. Fixed geometry wins a tie with a hurt box, and equal hurt-box contacts prefer
    /// the lower id. A projectile stops at its first contact.
    fn advance_arrows(&mut self) -> Result<(), GameError> {
        if self.arrows.is_empty() {
            return Ok(());
        }
        let active_rooms = self
            .rooms
            .iter()
            .filter(|room| room.encounter_state == RoomEncounterState::Active)
            .map(|room| room.id)
            .collect::<BTreeSet<_>>();
        let arrows = std::mem::take(&mut self.arrows);
        let mut remaining = Vec::with_capacity(arrows.len());
        for mut arrow in arrows {
            // Rebuilt per arrow: an earlier arrow this tick may have killed its target.
            let hurtboxes = match arrow.source {
                StrikeSource::Player(_) => self
                    .monsters
                    .iter()
                    .filter(|monster| monster.health > 0 && active_rooms.contains(&monster.room_id))
                    .map(|monster| {
                        (
                            StrikeTarget::Monster(monster.id),
                            monster.position,
                            MONSTER_HURTBOX_HALF_EXTENTS,
                        )
                    })
                    .collect::<Vec<_>>(),
                StrikeSource::Monster(_) => self
                    .players
                    .iter()
                    .filter(|(_, state)| state.health > 0)
                    .map(|(&player_id, state)| {
                        (
                            StrikeTarget::Player(player_id),
                            state.position,
                            PLAYER_HURTBOX_HALF_EXTENTS,
                        )
                    })
                    .collect::<Vec<_>>(),
            };
            let (from, translation) = (arrow.position, arrow.velocity);
            let wall = self.physics.first_fixed_hit(from, translation);
            let hurt = hurtboxes
                .into_iter()
                // Hurt volumes are columns on the plane (ADR 0002): a shot meets a body at
                // any height, so jumping neither dodges nor loses shots.
                .filter_map(|(target, center, half_extents)| {
                    let center = Vector::new(center.x, from.y, center.z);
                    Some((box_hit(center, half_extents, from, translation)?, target))
                })
                .min_by(|(left, left_target), (right, right_target)| {
                    left.total_cmp(right).then(left_target.cmp(right_target))
                });
            let hit = match (wall, hurt) {
                (Some(wall), Some((time, target))) => (time < wall).then_some(target),
                (None, Some((_, target))) => Some(target),
                (Some(_), None) => None,
                (None, None) => {
                    arrow.position =
                        Vector::new(from.x + translation.x, from.y, from.z + translation.z);
                    arrow.ticks_remaining -= 1;
                    if arrow.ticks_remaining > 0 {
                        remaining.push(arrow);
                    }
                    continue;
                }
            };
            // Fixed geometry stops a projectile it reaches first; it is gone either way.
            match hit {
                Some(StrikeTarget::Player(player_id)) => {
                    self.resolve_projectile_hit(arrow, player_id)?;
                }
                Some(StrikeTarget::Monster(monster_id)) => {
                    self.resolve_arrow_hit(arrow, monster_id)?;
                }
                None => {}
            }
        }
        self.arrows = remaining;
        Ok(())
    }

    /// Routes a player's arrow impact through the shared strike outcome path.
    fn resolve_arrow_hit(&mut self, arrow: ArrowState, monster_id: u32) -> Result<(), GameError> {
        let StrikeSource::Player(owner_id) = arrow.source else {
            return Err(GameError::new("a monster's shot cannot hit a monster"));
        };
        let shooter_present = self.players.contains_key(&owner_id);
        let monster = self
            .monsters
            .iter_mut()
            .find(|monster| monster.id == monster_id)
            .ok_or_else(|| GameError::new("arrow hit an unknown monster"))?;
        let previous_health = monster.health;
        monster.health = monster.health.saturating_sub(arrow.damage);
        if let Some(guard) = monster.guard.as_mut() {
            guard.stance = None;
        }
        if monster.health > 0 {
            monster.stagger_ticks_remaining = self.tuning.bow.arrow_stagger_ticks;
            monster.action = None;
            if shooter_present {
                monster.provoke(owner_id);
            }
        }
        let defeated = monster.health == 0;
        let position = monster.position;
        let reward = monster.definition().experience_reward;
        let damage = previous_health - monster.health;
        self.push_strike_outcome(StrikeOutcome {
            order: 0,
            strike: StrikeId {
                source: arrow.source,
                tick: arrow.launched_at_tick,
            },
            definition: BOW_ARROW_DEFINITION.clone(),
            target: StrikeTarget::Monster(monster_id),
            result: StrikeResult::Hit { damage, defeated },
        });
        if defeated {
            // A departed shooter's arrow still kills, but nobody is credited.
            if self.players.contains_key(&owner_id) {
                self.award_experience(owner_id, reward)?;
            }
            self.spawn_ground_loot(position)?;
        }
        self.reconcile_encounters()
    }

    /// Launches a ranged monster's shot (#108) from its centre at where its target stands
    /// as the active phase opens. The direction is committed here and never steered; the
    /// shot first moves on the next tick.
    fn launch_monster_projectile(
        &mut self,
        monster_id: u32,
        target_player_id: PlayerId,
    ) -> Result<(), GameError> {
        let Some((position, definition)) = self
            .monsters
            .iter()
            .find(|monster| monster.id == monster_id && monster.health > 0)
            .map(|monster| (monster.position, monster.definition()))
        else {
            return Ok(());
        };
        let Some(projectile) = definition.projectile else {
            return Err(GameError::new("monster attack has no projectile"));
        };
        let target = self
            .players
            .get(&target_player_id)
            .filter(|state| state.health > 0)
            .map(|state| state.position);
        let Some(target) = target else {
            return Ok(());
        };
        let Some(velocity) = projectile_velocity(xz_delta(position, target), projectile.speed)
        else {
            return Ok(());
        };
        if self.arrows.len() >= usize::from(self.tuning.bow.max_live_arrows) {
            self.arrows.remove(0);
        }
        let id = self.next_arrow_id;
        self.next_arrow_id = id
            .checked_add(1)
            .ok_or_else(|| GameError::new("arrow id overflow"))?;
        self.arrows.push(ArrowState {
            id,
            source: StrikeSource::Monster(monster_id),
            launched_at_tick: self.tick,
            position,
            velocity,
            damage: definition.damage,
            ticks_remaining: projectile.lifetime_ticks,
        });
        Ok(())
    }

    /// Routes a monster's shot that reached `player_id` through the shared monster hit
    /// path: its strike definition decides block and guard cost, its damage the hurt.
    fn resolve_projectile_hit(
        &mut self,
        arrow: ArrowState,
        player_id: PlayerId,
    ) -> Result<(), GameError> {
        let StrikeSource::Monster(monster_id) = arrow.source else {
            return Err(GameError::new("a player's arrow cannot hit a player"));
        };
        let definition = self
            .monsters
            .iter()
            .find(|monster| monster.id == monster_id)
            .ok_or_else(|| GameError::new("shot from an unknown monster"))?
            .definition();
        let defender = self.player_position(player_id)?;
        self.land_monster_hit(
            StrikeId {
                source: arrow.source,
                tick: arrow.launched_at_tick,
            },
            definition.strike,
            arrow.damage,
            player_id,
            defender,
            // The shot arrives from where it was at the start of this tick.
            arrow.position,
        )
    }

    fn resolve_monster_attack(
        &mut self,
        monster_id: u32,
        target_player_id: PlayerId,
    ) -> Result<(), GameError> {
        let Some(monster) = self
            .monsters
            .iter()
            .find(|monster| monster.id == monster_id)
        else {
            return Ok(());
        };
        self.resolve_monster_strike(monster_id, target_player_id, monster.definition().strike)
    }

    /// Resolves the shield before damage: a valid block or guard break never touches health.
    /// Players and shield monsters share it; `facing` is the guard's front direction.
    fn guard_outcome(
        tuning: &GuardData,
        guard: &mut GuardState,
        facing: (f64, f64),
        definition: StrikeDefinition,
        defender: Vector,
        attacker: Vector,
    ) -> Option<StrikeResult> {
        let incoming = xz_delta(defender, attacker);
        // A zero-direction (overlapping) contact has no incoming side to block.
        if !definition.blockable
            || !guard.is_raised()
            || incoming == (0.0, 0.0)
            || !Self::target_is_in_front(facing, incoming)
        {
            return None;
        }
        if definition.guard_cost >= guard.points {
            guard.points = 0;
            guard.stance = None;
            guard.block_reaction_ticks_remaining = 0;
            guard.broken_ticks_remaining = tuning.break_ticks;
            return Some(StrikeResult::GuardBroken);
        }
        guard.points -= definition.guard_cost;
        guard.block_reaction_ticks_remaining = tuning.block_reaction_ticks;
        Some(StrikeResult::Blocked {
            guard_damage: definition.guard_cost,
        })
    }

    fn resolve_monster_strike(
        &mut self,
        monster_id: u32,
        target_player_id: PlayerId,
        definition: StrikeDefinition,
    ) -> Result<(), GameError> {
        let monster = self
            .monsters
            .iter()
            .find(|monster| monster.id == monster_id && monster.health > 0)
            .map(|monster| (monster.position, monster.definition().damage));
        let Some((monster_position, monster_damage)) = monster else {
            return Ok(());
        };
        let target_position = self
            .players
            .get(&target_player_id)
            .map(|state| state.position);
        let Some(target_position) = target_position else {
            return Ok(());
        };
        let target_alive = self
            .players
            .get(&target_player_id)
            .is_some_and(|player| player.health > 0);
        if !target_alive {
            return Ok(());
        }

        let contacts = self.strike_contacts(
            definition,
            StrikeTarget::Monster(monster_id),
            monster_position,
            (0.0, 0.0),
            [(StrikeTarget::Player(target_player_id), target_position)],
        )?;
        let strike = StrikeId {
            source: StrikeSource::Monster(monster_id),
            tick: self.tick,
        };
        for (target, obstructed) in contacts {
            if obstructed {
                self.push_strike_outcome(StrikeOutcome {
                    order: 0,
                    strike,
                    definition: self.content.id(definition.id).clone(),
                    target,
                    result: StrikeResult::Obstructed,
                });
            } else {
                self.land_monster_hit(
                    strike,
                    definition,
                    monster_damage,
                    target_player_id,
                    target_position,
                    monster_position,
                )?;
            }
        }
        Ok(())
    }

    /// A monster strike or shot that reached `player_id` unobstructed: the shield first,
    /// then damage, then the counter opportunity a block grants. Melee and projectiles
    /// share this path.
    fn land_monster_hit(
        &mut self,
        strike: StrikeId,
        definition: StrikeDefinition,
        damage: u16,
        player_id: PlayerId,
        defender: Vector,
        attacker: Vector,
    ) -> Result<(), GameError> {
        let StrikeSource::Monster(monster_id) = strike.source else {
            return Err(GameError::new("monster hit from a player source"));
        };
        let tick = self.tick;
        let tuning = self.tuning;
        let player = self
            .players
            .get_mut(&player_id)
            .ok_or_else(|| GameError::new("monster attack references an unknown player"))?;
        let facing = (f64::from(player.facing_x), f64::from(player.facing_z));
        let result = if let Some(result) = Self::guard_outcome(
            &tuning.guard,
            &mut player.guard,
            facing,
            definition,
            defender,
            attacker,
        ) {
            result
        } else {
            let previous_health = player.health;
            player.health = player.health.saturating_sub(damage);
            player.hurt_ticks_remaining = PLAYER_HURT_TICKS;
            player.action = None;
            player.guard.stance = None;
            // A hit lowers a drawn bow immediately, before any release can arrive.
            player.draw_ticks = None;
            StrikeResult::Hit {
                damage: previous_health - player.health,
                defeated: player.health == 0,
            }
        };
        match result {
            StrikeResult::Blocked { .. } => {
                player.counter = Some(CounterOpportunity::grant(
                    tuning.counter_window_ticks,
                    monster_id,
                    tick,
                ));
            }
            StrikeResult::Hit { .. } | StrikeResult::GuardBroken => player.counter = None,
            StrikeResult::Obstructed => {}
        }
        self.push_strike_outcome(StrikeOutcome {
            order: 0,
            strike,
            definition: self.content.id(definition.id).clone(),
            target: StrikeTarget::Player(player_id),
            result,
        });
        Ok(())
    }

    /// Gives monsters in running encounters a physics body and returns each body's intended
    /// per-tick motion from its behaviour: pursuers follow a room-grid path, everything else
    /// holds still.
    fn steer_monsters(&mut self) -> Result<Vec<(usize, Vector)>, GameError> {
        for monster in &mut self.monsters {
            monster.cornered = false;
        }
        #[cfg(test)]
        if self.monsters_without_bodies {
            return Ok(Vec::new());
        }
        let active_rooms = self
            .rooms
            .iter()
            .filter(|room| room.encounter_state == RoomEncounterState::Active)
            .map(|room| room.id)
            .collect::<BTreeSet<_>>();
        for monster in &self.monsters {
            let body_id = Self::monster_body_id(monster.id);
            let wants_body = monster.health > 0 && active_rooms.contains(&monster.room_id);
            match (wants_body, self.physics.character_position(body_id)) {
                (true, None) => self.physics.add_character(
                    body_id,
                    monster.position,
                    MONSTER_BODY_HALF_EXTENTS,
                )?,
                // Placed outside physics (scenario setup, tests): the body follows.
                (true, Some(position)) if position != monster.position => {
                    self.physics.place_character(body_id, monster.position)?;
                }
                (false, Some(_)) => {
                    self.physics.remove_character(body_id);
                }
                _ => {}
            }
        }

        let targets = self.monster_targets();
        #[cfg(test)]
        let mode = self.navigation_mode;
        #[cfg(not(test))]
        let use_fields = true;
        #[cfg(test)]
        let use_fields = mode != NavigationMode::PerTickReference;
        #[cfg(test)]
        if mode != NavigationMode::Retained {
            self.navigation = NavigationCache::default();
        }
        // Bodies other movers keep apart from, from positions before this tick's step.
        let bodies = self
            .monsters
            .iter()
            .filter(|monster| {
                self.physics
                    .has_character(Self::monster_body_id(monster.id))
            })
            .map(|monster| (monster.id, monster.room_id, monster.position))
            .collect::<Vec<_>>();
        let mut cache = std::mem::take(&mut self.navigation);
        let mut work = NavigationWork::default();
        let mut obstacles_checked = false;
        let mut chased_fields = BTreeSet::new();
        let mut velocities = Vec::new();
        for (index, monster) in self.monsters.iter().enumerate() {
            if !self
                .physics
                .has_character(Self::monster_body_id(monster.id))
            {
                continue;
            }
            let steering = self.steering(monster, &targets)?;
            let definition = monster.definition();
            let (goal, goal_reach) = match steering {
                Steering::Hold => {
                    velocities.push((index, Vector::ZERO, steering));
                    continue;
                }
                // The band is checked against the threat itself; reach is unused.
                Steering::Retreat { target } => (target, 0),
                Steering::Pursue { target } => {
                    work.pursuit_ticks += 1;
                    // End inside reach so the attack range check passes on arrival.
                    (target, definition.strike.reach - i64::from(NAV_CELL_SIZE))
                }
                // Any free cell whose centre is within one cell of the post.
                Steering::Return { post } => (post, i64::from(NAV_CELL_SIZE)),
            };
            if !obstacles_checked {
                obstacles_checked = true;
                let obstacles = self.fixed_footprints();
                if obstacles != cache.obstacles {
                    cache = NavigationCache {
                        obstacles,
                        ..NavigationCache::default()
                    };
                }
            }
            let grid = cache.grids.entry(monster.room_id).or_insert_with(|| {
                work.grid_builds += 1;
                self.room_grid(monster.room_id, &cache.obstacles)
            });
            let start = (monster.position.x, monster.position.z);
            let goal_xz = (goal.x, goal.z);
            // The attack's own line test from a cell centre: the physics ray against fixed
            // bodies.
            let line_from =
                |(x, z): (i32, i32)| Vector::new(x as f32, monster.position.y, z as f32);
            let path = if let Steering::Return { .. } = steering {
                // Returning is rare and short-lived: the exact plan, without a line test.
                work.exact_plans += 1;
                grid.find_path(start, goal_xz, goal_reach, |_| true, &mut work.expansions)
            } else if let Some(retreat) = definition
                .retreat_range
                .filter(|_| matches!(steering, Steering::Retreat { .. }))
            {
                // Backing off (#108) is short-lived too: an exact plan into the band from
                // one cell beyond the retreat range to one cell inside reach, with the
                // attack's own line test, through cells no nearer to the target.
                work.retreat_plans += 1;
                let band = (
                    retreat + i64::from(NAV_CELL_SIZE),
                    definition.strike.reach - i64::from(NAV_CELL_SIZE),
                );
                let clear_line = |centre: (i32, i32)| {
                    work.physics_queries += 1;
                    !self
                        .strike_obstructed(line_from(centre), goal)
                        .unwrap_or(true)
                };
                grid.find_retreat(start, goal_xz, band, clear_line, &mut work.expansions)
            } else {
                let route = grid
                    .target_cell(goal_xz)
                    .filter(|_| use_fields)
                    .map(|cell| {
                        let key = (monster.room_id, cell, goal_reach);
                        chased_fields.insert(key);
                        let field = cache.fields.entry(key).or_insert_with(|| {
                            work.field_builds += 1;
                            grid.target_field(cell, goal_reach, &cache.obstacles)
                        });
                        let mut field_work = FieldWork::default();
                        let route = grid.field_route(field, start, &mut field_work);
                        work.field_searches += field_work.searches;
                        work.expansions += field_work.expansions;
                        route
                    });
                match route {
                    Some(FieldRoute::Path(path)) => Some(path),
                    Some(FieldRoute::NoStartCell | FieldRoute::NoRoute) => None,
                    // The exact plan finishes the approach from a strict goal and decides
                    // what the field cannot.
                    None | Some(FieldRoute::AtGoal | FieldRoute::Unknown) => {
                        work.exact_plans += 1;
                        let clear_line = |centre: (i32, i32)| {
                            work.physics_queries += 1;
                            !self
                                .strike_obstructed(line_from(centre), goal)
                                .unwrap_or(true)
                        };
                        grid.find_path(start, goal_xz, goal_reach, clear_line, &mut work.expansions)
                    }
                }
            };
            let velocity = path
                .as_deref()
                .and_then(|path| Some((path_point(grid, monster.position, path)?, path.last()?)))
                .map_or(Vector::ZERO, |(point, &(end_x, end_z))| {
                    // A retreat crowds around its destination, a pursuit around its target.
                    let crowd_goal = if let Steering::Retreat { .. } = steering {
                        Vector::new(end_x as f32, goal.y, end_z as f32)
                    } else {
                        goal
                    };
                    separated_velocity(
                        monster,
                        crowd_goal,
                        point,
                        definition.pursuit_speed,
                        &bodies,
                        definition.separation_range,
                    )
                });
            velocities.push((index, velocity, steering));
        }
        cache.fields.retain(|key, _| chased_fields.contains(key));
        self.navigation = cache;
        self.navigation_work.add(work);
        for monster in &mut self.monsters {
            monster.steered = false;
            monster.retreating = false;
        }
        let mut motions = Vec::with_capacity(velocities.len());
        for (index, velocity, steering) in velocities {
            let monster = &mut self.monsters[index];
            let retreat = matches!(steering, Steering::Retreat { .. });
            monster.steered = velocity != Vector::ZERO;
            monster.retreating = retreat && monster.steered;
            monster.cornered = retreat && !monster.steered;
            motions.push((index, velocity));
        }
        Ok(motions)
    }

    /// Living players and the room each stands in.
    fn monster_targets(&self) -> Vec<(PlayerId, RoomId, Vector)> {
        self.players
            .iter()
            .filter(|(_, state)| state.health > 0 && self.reaches_base(state.region))
            .filter_map(|(&player_id, state)| {
                let position = state.position;
                let room_id = self.room_at_position(position)?;
                Some((player_id, room_id, position))
            })
            .collect()
    }

    /// Where `target_player_id` stands if it is a living player in `monster`'s room.
    fn target_in_room(
        monster: &MonsterState,
        targets: &[(PlayerId, RoomId, Vector)],
        target_player_id: PlayerId,
    ) -> Option<Vector> {
        targets
            .iter()
            .find(|&&(player_id, room_id, _)| {
                player_id == target_player_id && room_id == monster.room_id
            })
            .map(|&(_, _, position)| position)
    }

    /// Whether a monster that keeps spacing (#108) stands nearer to `target` than its
    /// retreat range.
    fn inside_retreat_range(monster: &MonsterState, target: Vector) -> bool {
        monster
            .definition()
            .retreat_range
            .is_some_and(|range| xz_distance_sq(monster.position, target) < range_sq(range))
    }

    /// Whether `monster` could start its attack on a player at `target` now: within strike
    /// reach with a clear strike line.
    fn can_strike(&self, monster: &MonsterState, target: Vector) -> Result<bool, GameError> {
        let reach = monster.definition().strike.reach;
        Ok(
            within_range(xz_distance_sq(monster.position, target), reach)
                && !self.strike_obstructed(monster.position, target)?,
        )
    }

    /// How a monster with a body moves this tick, from its engagement (#109). Staggered,
    /// attacking and dead monsters hold; an engaged monster that can already strike its
    /// target holds for the attack, which starts at the end of the tick.
    fn steering(
        &self,
        monster: &MonsterState,
        targets: &[(PlayerId, RoomId, Vector)],
    ) -> Result<Steering, GameError> {
        if monster.health == 0 || monster.stagger_ticks_remaining > 0 || monster.action.is_some() {
            return Ok(Steering::Hold);
        }
        Ok(match monster.engagement {
            Engagement::Engaged { target_player_id } => {
                match Self::target_in_room(monster, targets, target_player_id) {
                    Some(target) if Self::inside_retreat_range(monster, target) => {
                        Steering::Retreat { target }
                    }
                    Some(target) if !self.can_strike(monster, target)? => {
                        Steering::Pursue { target }
                    }
                    // In reach, or the target is gone (the end of the tick notices).
                    _ => Steering::Hold,
                }
            }
            Engagement::Returning => Steering::Return {
                post: monster.post.unwrap_or(monster.position),
            },
            Engagement::Idle | Engagement::Searching { .. } => Steering::Hold,
        })
    }

    /// The best player for `monster` to engage among `candidates`: one it can strike now
    /// first, then the nearest, then the lowest id. Each entry is (id, position, squared
    /// distance).
    fn best_candidate(
        &self,
        monster: &MonsterState,
        candidates: impl Iterator<Item = (PlayerId, Vector, f64)>,
    ) -> Result<Option<(bool, f64, PlayerId)>, GameError> {
        let mut best: Option<(bool, f64, PlayerId)> = None;
        for (player_id, position, distance_sq) in candidates {
            let key = (!self.can_strike(monster, position)?, distance_sq, player_id);
            let better = best.is_none_or(|best| {
                key.0
                    .cmp(&best.0)
                    .then(key.1.total_cmp(&best.1))
                    .then(key.2.cmp(&best.2))
                    .is_lt()
            });
            if better {
                best = Some(key);
            }
        }
        Ok(best.map(|(blocked, distance_sq, player_id)| (!blocked, distance_sq, player_id)))
    }

    /// The engagement policy (#109), evaluated at the end of every tick in which `monster`
    /// is free. A pure function of authoritative state: positions, its post, its current
    /// engagement and whether it was steered this tick.
    fn next_engagement(
        &self,
        monster: &MonsterState,
        post: Vector,
        targets: &[(PlayerId, RoomId, Vector)],
    ) -> Result<Engagement, GameError> {
        let definition = monster.definition();
        count(Counter::MonsterDecision, 1);
        let in_aggro_range = targets
            .iter()
            .filter(|&&(_, room_id, _)| {
                count(Counter::MonsterTargetEvaluation, 1);
                room_id == monster.room_id
            })
            .map(|&(player_id, _, position)| {
                (
                    player_id,
                    position,
                    xz_distance_sq(monster.position, position),
                )
            })
            .filter(|&(_, _, distance_sq)| within_range(distance_sq, definition.aggro_range));
        let from_post_sq = xz_distance_sq(monster.position, post);
        let away_from_post = !within_range(from_post_sq, POST_ARRIVAL_DISTANCE);
        match monster.engagement {
            Engagement::Engaged { target_player_id } => {
                let Some(current) = Self::target_in_room(monster, targets, target_player_id) else {
                    return Ok(Engagement::Searching {
                        ticks_remaining: definition.reacquire_ticks,
                    });
                };
                if !within_range(from_post_sq, definition.leash_range) {
                    return Ok(Engagement::Returning);
                }
                let challenger = self.best_candidate(
                    monster,
                    in_aggro_range.filter(|&(player_id, _, _)| player_id != target_player_id),
                )?;
                let Some((challenger_strikes, challenger_sq, challenger_id)) = challenger else {
                    return Ok(monster.engagement);
                };
                let current_strikes = self.can_strike(monster, current)?;
                let takes_over = if challenger_strikes == current_strikes {
                    challenger_sq.sqrt() + (definition.target_switch_margin as f64)
                        < xz_distance_sq(monster.position, current).sqrt()
                } else {
                    challenger_strikes
                };
                Ok(if takes_over {
                    Engagement::Engaged {
                        target_player_id: challenger_id,
                    }
                } else {
                    monster.engagement
                })
            }
            Engagement::Searching { ticks_remaining } if ticks_remaining > 1 => {
                Ok(Engagement::Searching {
                    ticks_remaining: ticks_remaining - 1,
                })
            }
            // Still on the way home; a returner that could not move has no way back and
            // rests where it stands.
            Engagement::Returning if away_from_post => Ok(if monster.steered {
                Engagement::Returning
            } else {
                Engagement::Idle
            }),
            Engagement::Idle | Engagement::Searching { .. } | Engagement::Returning => {
                let found = self.best_candidate(monster, in_aggro_range)?;
                Ok(match found {
                    Some((_, _, player_id)) => Engagement::Engaged {
                        target_player_id: player_id,
                    },
                    None if away_from_post
                        && matches!(monster.engagement, Engagement::Searching { .. }) =>
                    {
                        Engagement::Returning
                    }
                    None => Engagement::Idle,
                })
            }
        }
    }

    /// XZ footprints of the world's fixed bodies (walls, pillars, locked doors), in body
    /// order: the navigation topology.
    /// The footprints monsters must walk around: every fixed box except those entirely above
    /// a monster body, such as a low ceiling (#209).
    fn fixed_footprints(&self) -> Vec<Rect> {
        let monster_top = PLAYER_Y + MONSTER_BODY_HALF_EXTENTS.y;
        self.physics
            .fixed_boxes()
            .filter(|(position, half)| position.y - half.y < monster_top)
            .map(|(position, half)| Rect::centered(position.x, position.z, half.x, half.z))
            .collect()
    }

    /// Passability of `room_id` for a monster body: the fixed footprints inflated by the
    /// body's clearance.
    fn room_grid(&self, room_id: RoomId, obstacles: &[Rect]) -> RoomGrid {
        let room = self
            .rooms
            .iter()
            .find(|room| room.id == room_id)
            .expect("monsters belong to generated rooms");
        RoomGrid::new(
            Rect {
                min_x: room.min_x,
                max_x: room.max_x,
                min_z: room.min_z,
                max_z: room.max_z,
            },
            obstacles,
            // A body may sit anywhere in its cell; one cell of margin keeps it strictly clear
            // of obstacles, which physics treats as contact even when only touching.
            MONSTER_BODY_HALF_EXTENTS.x.max(MONSTER_BODY_HALF_EXTENTS.z) + NAV_CELL_SIZE,
        )
    }

    /// Advances monster attacks, then runs the engagement policy for every free monster and
    /// starts an attack on its engaged target when it can strike, and publishes behaviour.
    fn advance_monster_actions(&mut self) -> Result<(), GameError> {
        let active_rooms = self
            .rooms
            .iter()
            .filter(|room| room.encounter_state == RoomEncounterState::Active)
            .map(|room| room.id)
            .collect::<BTreeSet<_>>();
        let targets = self.monster_targets();

        let mut hits = Vec::new();
        for index in 0..self.monsters.len() {
            let mut monster = self.monsters[index];
            let definition = monster.definition();
            let active = active_rooms.contains(&monster.room_id);
            if monster.health == 0 || monster.stagger_ticks_remaining > 0 || !active {
                if monster.stagger_ticks_remaining > 0 || monster.health == 0 {
                    monster.action = None;
                }
                // Stagger lowers the guard; a staggered shield monster raises it again from
                // the start once free. Death and a stopped encounter release it.
                if let Some(guard) = monster.guard.as_mut() {
                    guard.stance = None;
                    if monster.health == 0 || !active {
                        guard.held = false;
                    }
                }
            } else if let Some(mut action) = monster.action {
                if action.ticks_remaining > 1 {
                    action.ticks_remaining -= 1;
                    monster.action = Some(action);
                } else {
                    match action.phase {
                        ActionPhase::Windup => {
                            action.phase = ActionPhase::Active;
                            action.ticks_remaining = definition.active_ticks;
                            monster.action = Some(action);
                            hits.push((monster.id, action.target_player_id));
                        }
                        ActionPhase::Active => {
                            action.phase = ActionPhase::Recovery;
                            action.ticks_remaining = definition.recovery_ticks;
                            monster.action = Some(action);
                        }
                        ActionPhase::Recovery => {
                            monster.action = None;
                        }
                    }
                }
            } else {
                let post = *monster.post.get_or_insert(monster.position);
                // Steering found no retreat from the target it was engaged on; a switch to
                // another target makes that result stale.
                let cornered_target = monster.engaged_target().filter(|_| monster.cornered);
                monster.engagement = self.next_engagement(&monster, post, &targets)?;
                let cornered =
                    cornered_target.is_some() && monster.engaged_target() == cornered_target;
                // Only the engaged target is attacked; an obstructed one is pursued
                // instead of winding up into a wall. A monster that keeps spacing attacks
                // from its band, or from where it stands when it found no way back (#108).
                if let Some(target_player_id) = monster.engaged_target()
                    && let Some(target) = Self::target_in_room(&monster, &targets, target_player_id)
                    && (cornered || !Self::inside_retreat_range(&monster, target))
                    && self.can_strike(&monster, target)?
                {
                    monster.action = Some(MonsterActionState {
                        phase: ActionPhase::Windup,
                        ticks_remaining: definition.windup_ticks,
                        target_player_id,
                    });
                }
                // A shield monster holds its guard whenever it is engaged and free, so it
                // closes on its target under a raised shield; committing to an attack (or
                // losing its target) lowers it at once, as releasing guard does a player's.
                let hold = monster.action.is_none() && monster.engaged_target().is_some();
                if let Some(guard) = monster.guard.as_mut() {
                    guard.held = hold;
                    if !hold {
                        guard.stance = None;
                    }
                }
            }
            self.monsters[index] = monster;
        }

        for (monster_id, target_player_id) in hits {
            let shoots = self.monsters.iter().any(|monster| {
                monster.id == monster_id && monster.definition().projectile.is_some()
            });
            if shoots {
                self.launch_monster_projectile(monster_id, target_player_id)?;
            } else {
                self.resolve_monster_attack(monster_id, target_player_id)?;
            }
        }
        Ok(())
    }

    fn reconcile_encounters(&mut self) -> Result<(), GameError> {
        let occupied_rooms = self
            .players
            .keys()
            .filter_map(|&player_id| {
                self.players
                    .get(&player_id)
                    .and_then(|state| self.room_at_position(state.position))
            })
            .collect::<BTreeSet<_>>();
        let rooms_with_living_monsters = self
            .monsters
            .iter()
            .filter(|monster| monster.health > 0)
            .map(|monster| monster.room_id)
            .collect::<BTreeSet<_>>();

        let encounters = self
            .rooms
            .iter()
            .map(|room| {
                self.room_encounter(room.id)
                    .map(|encounter| (encounter.activation, encounter.completion))
            })
            .collect::<Vec<_>>();
        for (room, encounter) in self.rooms.iter_mut().zip(encounters) {
            // The start room runs no encounter and stays cleared.
            let Some((activation, completion)) = encounter else {
                continue;
            };
            let activated = match activation {
                EncounterActivation::PlayerEntersRoom => occupied_rooms.contains(&room.id),
            };
            // Every monster of the room counts, including any a workbench arranged there.
            let completed = match completion {
                EncounterCompletion::SpawnGroupDefeated => {
                    !rooms_with_living_monsters.contains(&room.id)
                }
            };
            match room.encounter_state {
                RoomEncounterState::Dormant if activated => {
                    room.encounter_state = if completed {
                        RoomEncounterState::Cleared
                    } else {
                        RoomEncounterState::Active
                    };
                }
                RoomEncounterState::Active if completed => {
                    room.encounter_state = RoomEncounterState::Cleared;
                }
                _ => {}
            }
        }
        self.sync_door_locks()
    }

    /// The tick's physics pass: Rapier's character controller moves every player by its
    /// controlled velocity, then every monster with a body by its steering, each in turn
    /// against the level and the characters already moved. A player keeps as velocity the
    /// part of its intended motion that physics allowed; corrections out of overlaps never
    /// become momentum.
    fn resolve_motion(&mut self, monster_motions: &[(usize, Vector)]) -> Result<(), GameError> {
        self.physics.rebuild();
        let jump = self.content.jump;
        let player_ids = self.players.keys().copied().collect::<Vec<_>>();
        for player_id in player_ids {
            let control = self.control_rates(self.players[&player_id].grounded);
            let state = self
                .players
                .get_mut(&player_id)
                .expect("moving player exists");
            let vertical = Self::takeoff_or_fall(&jump, state);
            let horizontal = Self::controlled_movement_velocity(state.velocity, *state, control);
            let intended = Vector::new(horizontal.x, vertical, horizontal.z);
            let motion = self.physics.move_player(
                Self::player_body_id(player_id),
                state.position,
                intended,
            )?;
            let allowed =
                |moved: f32, intended: f32| moved.clamp(intended.min(0.0), intended.max(0.0));
            let moved = motion.moved;
            state.velocity = Vector::new(
                allowed(moved.x, intended.x),
                allowed(moved.y, intended.y),
                allowed(moved.z, intended.z),
            );
            state.position = motion.to;
            Self::settle_support(&jump, &self.topology, state, motion.supported);
        }
        for &(index, velocity) in monster_motions {
            let monster = &mut self.monsters[index];
            (monster.position, _) = self.physics.move_character(
                Self::monster_body_id(monster.id),
                monster.position,
                velocity,
            )?;
        }
        self.physics_steps = self
            .physics_steps
            .checked_add(1)
            .ok_or_else(|| GameError::new("physics step counter overflow"))?;
        Ok(())
    }

    /// Fall recovery (Atlas 5.1.3, #211): a player whose body ended the tick out of the world
    /// returns, at rest and supported, to where it last ended a tick supported. It takes no
    /// damage; its actions, guard and buffered jump carry on.
    fn recover_fallen_players(&mut self) -> Result<(), GameError> {
        let fallen = self
            .players
            .iter()
            .filter(|(_, state)| self.out_of_world(state.position))
            .map(|(&id, _)| id)
            .collect::<Vec<_>>();
        for player_id in fallen {
            let state = self
                .players
                .get_mut(&player_id)
                .expect("fallen player exists");
            let to = state.last_supported;
            state.position = to;
            state.velocity = Vector::ZERO;
            state.grounded = true;
            state.coyote_ticks_remaining = 0;
            state.region = self.topology.settle(
                state.region,
                to.x,
                to.z,
                to.y - PLAYER_HALF_EXTENTS.y as f32,
                GROUNDED_HEIGHT_TOLERANCE,
                PLAYER_HALF_EXTENTS.x as f32,
            );
            self.physics
                .place_character(Self::player_body_id(player_id), to)?;
        }
        Ok(())
    }

    /// Whether a body centred on `position` has left the world: its centre outside the
    /// dungeon's extents, or its feet a body height below the lowest floor (#211).
    fn out_of_world(&self, position: Vector) -> bool {
        let feet = position.y - PLAYER_HALF_EXTENTS.y as f32;
        let depth = (2 * PLAYER_HALF_EXTENTS.y) as f32;
        !position.is_finite()
            || feet < self.topology.lowest_foot() as f32 - depth
            || !self.inside_dungeon_extents(position)
    }

    /// Whether (`x`, `z`) of `position` lies within the rooms' combined extents.
    fn inside_dungeon_extents(&self, position: Vector) -> bool {
        let extent = |select: fn(&RoomSnapshot) -> i32, pick: fn(i32, i32) -> i32| {
            self.rooms
                .iter()
                .map(select)
                .reduce(pick)
                .map(|value| value as f32)
        };
        extent(|room| room.min_x, i32::min).is_some_and(|min| position.x >= min)
            && extent(|room| room.max_x, i32::max).is_some_and(|max| position.x <= max)
            && extent(|room| room.min_z, i32::min).is_some_and(|min| position.z >= min)
            && extent(|room| room.max_z, i32::max).is_some_and(|max| position.z <= max)
    }

    /// This tick's vertical intent (world units): a takeoff when a buffered jump press meets
    /// a possible jump, gravity while airborne, nothing on the ground. A takeoff consumes the
    /// press and the coyote window and lowers the guard at once, since guarding needs the
    /// ground; it never touches an action, which it cannot happen during.
    fn takeoff_or_fall(jump: &JumpTuning, state: &mut PlayerState) -> f32 {
        let can_jump = state.health > 0
            && state.action.is_none()
            && (state.grounded || state.coyote_ticks_remaining > 0);
        if can_jump && state.jump_buffer_ticks_remaining > 0 {
            state.jump_buffer_ticks_remaining = 0;
            state.coyote_ticks_remaining = 0;
            state.grounded = false;
            state.guard.stance = None;
            return jump.takeoff_speed;
        }
        state.jump_buffer_ticks_remaining = state.jump_buffer_ticks_remaining.saturating_sub(1);
        if state.grounded {
            return 0.0;
        }
        state.coyote_ticks_remaining = state.coyote_ticks_remaining.saturating_sub(1);
        state.velocity.y - jump.gravity
    }

    /// Support after this tick's motion. A rising body is never grounded; landing ends the
    /// vertical motion and settles the body's region membership (#207), and leaving the
    /// ground without jumping opens the coyote window. Airborne motion keeps the membership
    /// until it traverses a declared connection, which puts the body in transit (#210).
    fn settle_support(
        jump: &JumpTuning,
        topology: &Topology,
        state: &mut PlayerState,
        supported: bool,
    ) {
        let grounded = supported && state.velocity.y <= 0.0;
        let (x, z) = (state.position.x, state.position.z);
        let feet = state.position.y - PLAYER_HALF_EXTENTS.y as f32;
        if grounded {
            state.velocity.y = 0.0;
            state.coyote_ticks_remaining = 0;
            state.last_supported = state.position;
            state.region = topology.settle(
                state.region,
                x,
                z,
                feet,
                GROUNDED_HEIGHT_TOLERANCE,
                PLAYER_HALF_EXTENTS.x as f32,
            );
        } else {
            if state.grounded {
                state.coyote_ticks_remaining = jump.data.coyote_ticks;
            }
            state.region = topology.airborne(state.region, x, z, feet, GROUNDED_HEIGHT_TOLERANCE);
        }
        state.grounded = grounded;
    }

    fn sync_door_locks(&mut self) -> Result<(), GameError> {
        let active_rooms = self
            .rooms
            .iter()
            .filter(|room| room.encounter_state == RoomEncounterState::Active)
            .map(|room| room.id)
            .collect::<BTreeSet<_>>();
        let desired_locks = self
            .doors
            .iter()
            .map(|door| active_rooms.contains(&door.room_a) || active_rooms.contains(&door.room_b))
            .collect::<Vec<_>>();

        for (index, desired_locked) in desired_locks.into_iter().enumerate() {
            if self.doors[index].locked == desired_locked {
                continue;
            }
            let door = &self.doors[index];
            if desired_locked {
                self.physics.add_fixed(
                    door.id,
                    array_to_vec(door.position),
                    array_to_vec(door.half_extents),
                )?;
            } else {
                self.physics.remove_fixed(door.id);
            }
            self.doors[index].locked = desired_locked;
        }
        Ok(())
    }
}

#[cfg(test)]
impl ArpgGame {
    /// Puts a player at `position`, as a spawn or restore would: no contact resolution.
    fn put_player(&mut self, player_id: PlayerId, position: impl Into<Vector>) {
        let position = position.into();
        self.players
            .get_mut(&player_id)
            .expect("placed player exists")
            .position = position;
        self.physics
            .place_character(Self::player_body_id(player_id), position)
            .expect("placed player has a body");
    }
}

impl AuthoritativeGame for ArpgGame {
    type Command = ArpgCommand;
    type Snapshot = ArpgSnapshot;

    fn tick_hz(&self) -> u16 {
        TICK_HZ
    }

    fn max_players(&self) -> usize {
        MAX_PLAYERS
    }

    fn current_tick(&self) -> u64 {
        self.tick
    }

    fn add_player(&mut self, player_id: PlayerId) -> Result<(), GameError> {
        if player_id == 0 {
            return Err(GameError::new("player id must be non-zero"));
        }
        if self.players.contains_key(&player_id) {
            return Err(GameError::new("player already exists"));
        }
        if self.players.len() >= MAX_PLAYERS {
            return Err(GameError::new("ARPG MVP player capacity reached"));
        }

        let spawn = vector(self.player_spawns[self.players.len()]);
        self.physics
            .add_character(Self::player_body_id(player_id), spawn, PLAYER_HALF_EXTENTS)?;
        self.players.insert(
            player_id,
            PlayerState {
                position: spawn,
                velocity: Vector::ZERO,
                // Spawns stand on the floor.
                grounded: true,
                coyote_ticks_remaining: 0,
                jump_buffer_ticks_remaining: 0,
                region: RegionMembership::Region {
                    id: self.topology.base(),
                },
                last_supported: spawn,
                movement_x: 0,
                movement_z: 0,
                facing_x: 1,
                facing_z: 0,
                action: None,
                hurt_ticks_remaining: 0,
                guard: GuardState::ready(self.tuning.guard.max_points),
                counter: None,
                weapon: Weapon::SwordAndShield,
                draw_ticks: None,
                aim: None,
                locked_monster_id: None,
                health: self.content.progression.base_max_health,
                experience: 0,
                gold: 0,
            },
        );
        self.last_sequences.insert(player_id, 0);
        self.last_reliable_sequences.insert(player_id, 0);
        Ok(())
    }

    fn remove_player(&mut self, player_id: PlayerId) -> bool {
        let removed = self.players.remove(&player_id).is_some();
        self.last_sequences.remove(&player_id);
        self.last_reliable_sequences.remove(&player_id);
        if removed {
            self.physics
                .remove_character(Self::player_body_id(player_id));
        }
        removed
    }

    fn apply_command(&mut self, command: PlayerCommand<Self::Command>) -> Result<(), GameError> {
        self.apply_lane_command(CommandLane::Datagram, command)
    }

    fn apply_lane_command(
        &mut self,
        lane: CommandLane,
        command: PlayerCommand<Self::Command>,
    ) -> Result<(), GameError> {
        count(Counter::CommandBookkeeping, 1);
        if !self.players.contains_key(&command.player_id) {
            return Err(GameError::new("command references an unknown player"));
        }
        count(Counter::CommandBookkeeping, 1);
        let last_sequence = self
            .lane_sequences(lane)
            .get(&command.player_id)
            .copied()
            .unwrap_or_default();
        if command.sequence <= last_sequence {
            return Err(GameError::new("command sequence is stale"));
        }

        // An aimed one-shot is `SetAim` followed by the unaimed command; an invalid aim
        // rejects the whole command before anything changes.
        if let Some(aim) = command.command.embedded_aim() {
            self.set_aim(command.player_id, Some(aim))?;
        }

        match command.command {
            ArpgCommand::SetMovement { x, z } => {
                let state = self
                    .players
                    .get_mut(&command.player_id)
                    .expect("player existence checked");
                state.movement_x = x.clamp(-1, 1);
                state.movement_z = z.clamp(-1, 1);
                // Movement turns the facing only while no aim or lock owns it.
                self.refresh_facing(command.player_id)?;
            }
            ArpgCommand::SetAim { direction } => self.set_aim(command.player_id, direction)?,
            ArpgCommand::CycleTarget => {
                self.cycle_target(command.player_id)?;
                self.refresh_facing(command.player_id)?;
            }
            ArpgCommand::ClearTarget => {
                self.players
                    .get_mut(&command.player_id)
                    .expect("player existence checked")
                    .locked_monster_id = None;
                self.refresh_facing(command.player_id)?;
            }
            ArpgCommand::PrimaryAttack { .. } | ArpgCommand::SecondaryAttack { .. }
                if self.players[&command.player_id].weapon == Weapon::Bow =>
            {
                // Sword strikes need the sword; the bow uses draw/release.
            }
            ArpgCommand::PrimaryAttack { .. } => {
                let tick = self.tick;
                let aim = self.intent_aim(command.player_id)?;
                let state = self
                    .players
                    .get_mut(&command.player_id)
                    .expect("player existence checked");
                // Countering needs the ground (#59); airborne, the press is a plain attack and
                // the opportunity stays open.
                let counter = state
                    .counter
                    .filter(|counter| counter.usable_at(tick) && state.grounded);
                if state.health > 0 && state.action.is_some() {
                    Self::combo_input(&self.content, state, ComboInput::Light, aim);
                } else if counter.is_some() && state.health > 0 && state.action.is_none() {
                    // Starting the counter atomically consumes the opportunity.
                    state.counter = None;
                    self.start_action(command.player_id, ActionKind::Counter)?
                } else {
                    self.start_action(command.player_id, ActionKind::PrimaryAttack)?
                }
            }
            ArpgCommand::SecondaryAttack { .. } => {
                let aim = self.intent_aim(command.player_id)?;
                let state = self
                    .players
                    .get_mut(&command.player_id)
                    .expect("player existence checked");
                if state.health > 0 && state.action.is_some() {
                    Self::combo_input(&self.content, state, ComboInput::Heavy, aim);
                } else {
                    self.start_action(command.player_id, ActionKind::SecondaryAttack)?
                }
            }
            ArpgCommand::Interact => self.start_action(command.player_id, ActionKind::Interact)?,
            ArpgCommand::SetGuard { raised } => {
                let state = self
                    .players
                    .get_mut(&command.player_id)
                    .expect("player existence checked");
                state.guard.held = raised;
                if !raised {
                    state.guard.stance = None;
                }
            }
            ArpgCommand::EquipWeapon { weapon } => {
                let state = self
                    .players
                    .get_mut(&command.player_id)
                    .expect("player existence checked");
                if state.health > 0 && state.action.is_none() && state.weapon != weapon {
                    state.weapon = weapon;
                    // Nothing carries across a weapon change except guard-break recovery.
                    state.draw_ticks = None;
                    state.guard.stance = None;
                    state.counter = None;
                }
            }
            ArpgCommand::DrawBow => {
                let state = self
                    .players
                    .get_mut(&command.player_id)
                    .expect("player existence checked");
                if state.weapon == Weapon::Bow
                    && state.health > 0
                    && state.action.is_none()
                    && state.hurt_ticks_remaining == 0
                    && state.draw_ticks.is_none()
                {
                    state.draw_ticks = Some(0);
                }
            }
            ArpgCommand::ReleaseBow { .. } => {
                // The release commits the direction: a lock lost later never steers the shot.
                let aim = self.intent_aim(command.player_id)?;
                let state = self
                    .players
                    .get_mut(&command.player_id)
                    .expect("player existence checked");
                // Re-check life and freedom: damage this tick may have landed after the draw
                // advanced.
                if let Some(drawn) = state.draw_ticks.take()
                    && drawn >= self.tuning.bow.min_draw_ticks
                    && state.health > 0
                    && state.hurt_ticks_remaining == 0
                    && state.action.is_none()
                {
                    let mut action = Self::new_action(&self.content, ActionKind::Shoot, state, aim);
                    action.charge = drawn;
                    state.action = Some(action);
                }
            }
            ArpgCommand::CancelBow => {
                self.players
                    .get_mut(&command.player_id)
                    .expect("player existence checked")
                    .draw_ticks = None;
            }
            ArpgCommand::Jump => {
                let buffer_ticks = self.content.jump.data.buffer_ticks;
                let state = self
                    .players
                    .get_mut(&command.player_id)
                    .expect("player existence checked");
                if state.health > 0 {
                    state.jump_buffer_ticks_remaining = buffer_ticks;
                }
            }
        }
        count(Counter::CommandBookkeeping, 1);
        self.lane_sequences_mut(lane)
            .insert(command.player_id, command.sequence);
        Ok(())
    }

    fn advance_tick(&mut self) -> Result<(), GameError> {
        self.strike_outcomes.clear();
        self.interaction_events.clear();
        let tuning = self.tuning;
        for player in self.players.values_mut() {
            player.hurt_ticks_remaining = player.hurt_ticks_remaining.saturating_sub(1);
            Self::advance_guard(&tuning.guard, player);
            Self::advance_draw(&tuning.bow, player);
        }
        let active_rooms = self
            .rooms
            .iter()
            .filter(|room| room.encounter_state == RoomEncounterState::Active)
            .map(|room| room.id)
            .collect::<BTreeSet<_>>();
        for monster in &mut self.monsters {
            monster.stagger_ticks_remaining = monster.stagger_ticks_remaining.saturating_sub(1);
            let active = active_rooms.contains(&monster.room_id);
            Self::advance_monster_guard(&tuning.guard, monster, active);
        }
        let monster_motions = self.steer_monsters()?;
        self.resolve_motion(&monster_motions)?;
        self.recover_fallen_players()?;
        self.reconcile_encounters()?;
        self.advance_actions()?;
        self.advance_arrows()?;
        // Monster strikes resolve guards against the facing: a locked player has moved this
        // tick, so face the target from the new position first. Losing a lock still happens
        // once, at the end of the tick.
        self.face_held_locks()?;
        self.advance_monster_actions()?;
        self.revalidate_target_locks()?;
        self.tick = self
            .tick
            .checked_add(1)
            .ok_or_else(|| GameError::new("tick overflow"))?;
        let tick = self.tick;
        for player in self.players.values_mut() {
            if player
                .counter
                .is_some_and(|counter| tick >= counter.expires_at_tick)
            {
                player.counter = None;
            }
        }
        Ok(())
    }

    fn snapshot(&self) -> Result<Self::Snapshot, GameError> {
        operation_work::snapshot_requested(self);
        let players = self
            .players
            .iter()
            .map(|(&id, state)| {
                let level = Self::level_for_experience(&self.content, state.experience);
                Ok(PlayerSnapshot {
                    id,
                    position: vec_to_array(Vec3i::rounded(state.position)),
                    health: state.health,
                    max_health: Self::max_health_for_level(&self.content, level),
                    level,
                    experience: state.experience,
                    experience_into_level: state.experience
                        % self.content.progression.experience_per_level,
                    experience_for_next_level: self.content.progression.experience_per_level,
                    attack_damage: Self::attack_damage_for_level(&self.content, level),
                    gold: state.gold,
                    alive: state.health > 0,
                    facing: [state.facing_x, state.facing_z],
                    action: state.action.map(ActionState::snapshot),
                    reaction: Self::player_reaction(state),
                    guard: state.guard.stance,
                    guard_points: state.guard.points,
                    max_guard_points: self.tuning.guard.max_points,
                    counter: state.counter,
                    weapon: state.weapon,
                    draw_ticks: state.draw_ticks,
                    interaction: if state.health == 0 || state.action.is_some() {
                        InteractionPrompt::Unavailable {
                            reason: InteractionRefusal::Busy,
                        }
                    } else {
                        match self.interaction_choice(state.region, state.position)? {
                            Ok(target) => InteractionPrompt::Available { target },
                            Err(reason) => InteractionPrompt::Unavailable { reason },
                        }
                    },
                    aim: state.aim,
                    locked_monster_id: state.locked_monster_id,
                    motion: PlayerMotionSnapshot {
                        grounded: state.grounded,
                        vertical_velocity: state.velocity.y.round() as i32,
                        feet_y: (state.position.y - PLAYER_HALF_EXTENTS.y as f32).round() as i32,
                        ground_y: self
                            .physics
                            .ground_height(state.position)
                            .map(|height| height.round() as i32),
                    },
                    region: state.region,
                })
            })
            .collect::<Result<Vec<_>, GameError>>()?;
        let active_rooms = self
            .rooms
            .iter()
            .filter(|room| room.encounter_state == RoomEncounterState::Active)
            .map(|room| room.id)
            .collect::<BTreeSet<_>>();
        let mut static_colliders = self.static_colliders.clone();
        static_colliders.extend(self.doors.iter().filter(|door| door.locked).map(|door| {
            StaticColliderSnapshot {
                id: door.id,
                position: door.position,
                half_extents: door.half_extents,
                kind: StaticColliderKind::Door,
            }
        }));

        Ok(ArpgSnapshot {
            schema_version: 23,
            run_seed: self.run_seed,
            tick: self.tick,
            world_units_per_meter: WORLD_UNITS_PER_METER,
            rooms: self.rooms.clone(),
            doors: self.doors.clone(),
            players,
            monsters: self
                .monsters
                .iter()
                .map(|monster| MonsterSnapshot {
                    id: monster.id,
                    definition: self.content.id(monster.definition().id).to_string(),
                    room_id: monster.room_id,
                    position: vec_to_array(Vec3i::rounded(monster.position)),
                    health: monster.health,
                    max_health: monster.definition().health,
                    alive: monster.health > 0,
                    action: monster
                        .action
                        .map(|action| action.snapshot(&monster.definition())),
                    reaction: (monster.stagger_ticks_remaining > 0).then_some(
                        MonsterReactionSnapshot {
                            kind: MonsterReactionKind::Stagger,
                            ticks_remaining: monster.stagger_ticks_remaining,
                        },
                    ),
                    behavior: monster
                        .derived_behavior(active_rooms.contains(&monster.room_id), monster.steered),
                    target_player_id: monster
                        .engaged_target()
                        .filter(|_| monster.health > 0 && active_rooms.contains(&monster.room_id)),
                    guard: monster.guard.and_then(|guard| guard.stance),
                    guard_points: monster.guard.map_or(0, |guard| guard.points),
                    max_guard_points: if monster.guard.is_some() {
                        self.tuning.guard.max_points
                    } else {
                        0
                    },
                })
                .collect(),
            ground_loot: self
                .ground_loot
                .iter()
                .map(|loot| GroundLootSnapshot {
                    id: loot.id,
                    position: vec_to_array(Vec3i::rounded(loot.position)),
                    kind: loot.kind,
                    amount: loot.amount,
                })
                .collect(),
            static_colliders,
            arrows: self.arrows.iter().map(ArrowState::snapshot).collect(),
            scenario: self.scenario,
            content_revision: self.content.revision.clone(),
            chests: self
                .chests
                .iter()
                .map(|chest| ChestSnapshot {
                    available: !chest.opened && self.room_cleared(chest.room_id),
                    ..*chest
                })
                .collect(),
            interaction_events: self.interaction_events.clone(),
            strike_events: self
                .strike_outcomes
                .iter()
                .map(|outcome| StrikeEventSnapshot {
                    order: outcome.order,
                    source: outcome.strike.source,
                    strike_tick: outcome.strike.tick,
                    definition: outcome.definition.to_string(),
                    target: outcome.target,
                    result: outcome.result,
                })
                .collect(),
        })
    }
}

fn generate_dungeon(content: &Content, seed: RunSeed) -> GeneratedDungeon {
    let mut colliders = Vec::new();
    let mut doors = Vec::new();
    let mut rng = DungeonRng::new(u64::from(seed));
    let left_partition_x = rng.range_i32(-1_200, -800);
    let right_partition_x = rng.range_i32(800, 1_200);
    let horizontal_partition_z = rng.range_i32(-300, 300);

    push_wall(
        &mut colliders,
        [0, PLAYER_Y, -ARENA_HALF_DEPTH],
        [ARENA_HALF_WIDTH, WALL_HALF_HEIGHT, WALL_HALF_THICKNESS],
    );
    push_wall(
        &mut colliders,
        [0, PLAYER_Y, ARENA_HALF_DEPTH],
        [ARENA_HALF_WIDTH, WALL_HALF_HEIGHT, WALL_HALF_THICKNESS],
    );
    push_wall(
        &mut colliders,
        [-ARENA_HALF_WIDTH, PLAYER_Y, 0],
        [WALL_HALF_THICKNESS, WALL_HALF_HEIGHT, ARENA_HALF_DEPTH],
    );
    push_wall(
        &mut colliders,
        [ARENA_HALF_WIDTH, PLAYER_Y, 0],
        [WALL_HALF_THICKNESS, WALL_HALF_HEIGHT, ARENA_HALF_DEPTH],
    );

    let interior_z_min = -ARENA_HALF_DEPTH + WALL_HALF_THICKNESS;
    let interior_z_max = ARENA_HALF_DEPTH - WALL_HALF_THICKNESS;
    let lower_z_max = horizontal_partition_z - PARTITION_MARGIN;
    let upper_z_min = horizontal_partition_z + PARTITION_MARGIN;
    let vertical_connections = [((1, 2), (4, 5)), ((2, 3), (5, 6))];
    for (partition_index, partition_x) in [left_partition_x, right_partition_x]
        .into_iter()
        .enumerate()
    {
        let lower_door = rng.range_i32(
            interior_z_min + DOOR_EDGE_MARGIN,
            lower_z_max - DOOR_EDGE_MARGIN,
        );
        push_vertical_wall_with_door(
            &mut colliders,
            partition_x,
            interior_z_min,
            lower_z_max,
            lower_door,
        );
        let (lower_a, lower_b) = vertical_connections[partition_index].0;
        push_door(
            &mut doors,
            lower_a,
            lower_b,
            [partition_x, PLAYER_Y, lower_door],
            [WALL_HALF_THICKNESS, WALL_HALF_HEIGHT, DOOR_HALF_WIDTH],
        );

        let upper_door = rng.range_i32(
            upper_z_min + DOOR_EDGE_MARGIN,
            interior_z_max - DOOR_EDGE_MARGIN,
        );
        push_vertical_wall_with_door(
            &mut colliders,
            partition_x,
            upper_z_min,
            interior_z_max,
            upper_door,
        );
        let (upper_a, upper_b) = vertical_connections[partition_index].1;
        push_door(
            &mut doors,
            upper_a,
            upper_b,
            [partition_x, PLAYER_Y, upper_door],
            [WALL_HALF_THICKNESS, WALL_HALF_HEIGHT, DOOR_HALF_WIDTH],
        );
    }

    let interior_x_min = -ARENA_HALF_WIDTH + WALL_HALF_THICKNESS;
    let interior_x_max = ARENA_HALF_WIDTH - WALL_HALF_THICKNESS;
    let columns = [
        (interior_x_min, left_partition_x - PARTITION_MARGIN),
        (
            left_partition_x + PARTITION_MARGIN,
            right_partition_x - PARTITION_MARGIN,
        ),
        (right_partition_x + PARTITION_MARGIN, interior_x_max),
    ];
    let horizontal_connections = [(1, 4), (2, 5), (3, 6)];
    for (column_index, (x_min, x_max)) in columns.into_iter().enumerate() {
        let doorway = rng.range_i32(x_min + DOOR_EDGE_MARGIN, x_max - DOOR_EDGE_MARGIN);
        push_horizontal_wall_with_door(
            &mut colliders,
            horizontal_partition_z,
            x_min,
            x_max,
            doorway,
        );
        let (room_a, room_b) = horizontal_connections[column_index];
        push_door(
            &mut doors,
            room_a,
            room_b,
            [doorway, PLAYER_Y, horizontal_partition_z],
            [DOOR_HALF_WIDTH, WALL_HALF_HEIGHT, WALL_HALF_THICKNESS],
        );
    }

    let rows = [(interior_z_min, lower_z_max), (upper_z_min, interior_z_max)];
    let mut rooms = generated_rooms(columns, rows);
    for (index, room) in rooms
        .iter_mut()
        .filter(|room| room.kind == RoomKind::Combat)
        .enumerate()
    {
        room.encounter = Some(content.id(content.room_encounter(index).id).to_string());
    }
    let start_room = rooms
        .iter()
        .find(|room| room.kind == RoomKind::Start)
        .expect("generated dungeon must contain a start room");
    let player_spawns = generated_player_spawns(start_room);
    let monsters = generated_monsters(content, &rooms, &colliders, &mut rng);

    GeneratedDungeon {
        rooms,
        doors,
        static_colliders: colliders,
        player_spawns,
        monsters,
    }
}

fn generated_rooms(columns: [(i32, i32); 3], rows: [(i32, i32); 2]) -> Vec<RoomSnapshot> {
    let neighbors = [
        vec![2, 4],
        vec![1, 3, 5],
        vec![2, 6],
        vec![1, 5],
        vec![2, 4, 6],
        vec![3, 5],
    ];
    let mut rooms = Vec::with_capacity(6);
    for (row_index, &(min_z, max_z)) in rows.iter().enumerate() {
        for (column_index, &(min_x, max_x)) in columns.iter().enumerate() {
            let index = row_index * columns.len() + column_index;
            let id = u32::try_from(index + 1).expect("room id must fit u32");
            let kind = if id == 1 {
                RoomKind::Start
            } else {
                RoomKind::Combat
            };
            rooms.push(RoomSnapshot {
                id,
                min_x,
                max_x,
                min_z,
                max_z,
                kind,
                encounter_state: if kind == RoomKind::Start {
                    RoomEncounterState::Cleared
                } else {
                    RoomEncounterState::Dormant
                },
                neighbors: neighbors[index].clone(),
                encounter: None,
            });
        }
    }
    rooms
}

fn generated_player_spawns(start_room: &RoomSnapshot) -> [Vec3i; MAX_PLAYERS] {
    let (center_x, center_z) = start_room.center();
    [
        Vec3i::new(
            center_x - PLAYER_SPAWN_OFFSET,
            PLAYER_Y,
            center_z - PLAYER_SPAWN_OFFSET,
        ),
        Vec3i::new(
            center_x - PLAYER_SPAWN_OFFSET,
            PLAYER_Y,
            center_z + PLAYER_SPAWN_OFFSET,
        ),
        Vec3i::new(
            center_x + PLAYER_SPAWN_OFFSET,
            PLAYER_Y,
            center_z - PLAYER_SPAWN_OFFSET,
        ),
        Vec3i::new(
            center_x + PLAYER_SPAWN_OFFSET,
            PLAYER_Y,
            center_z + PLAYER_SPAWN_OFFSET,
        ),
    ]
}

/// Spawns each combat room's encounter group: rooms in generation order, members in group
/// order, ids from 1. Every member draws its position from the dungeon RNG exactly as a
/// lone room monster always has, so single-member groups generate the same dungeon.
fn generated_monsters(
    content: &Content,
    rooms: &[RoomSnapshot],
    colliders: &[StaticColliderSnapshot],
    rng: &mut DungeonRng,
) -> Vec<MonsterState> {
    let mut monsters = Vec::new();
    for (index, room) in rooms
        .iter()
        .filter(|room| room.kind == RoomKind::Combat)
        .enumerate()
    {
        let encounter = content.room_encounter(index);
        let mut placed = Vec::new();
        for &(definition, count) in &encounter.spawn_group {
            for _ in 0..count {
                let position = spawn_position(room, &placed, colliders, rng);
                placed.push(position);
                monsters.push(MonsterState::new(
                    u32::try_from(monsters.len() + 1).expect("monster id must fit u32"),
                    *content.monster(definition),
                    room.id,
                    position,
                    content.guard.max_points,
                ));
            }
        }
    }
    monsters
}

/// Redraws a spawn position that overlaps an earlier member or a fixed collider at most this
/// often before falling back to the first free spot on a fixed grid.
const SPAWN_ATTEMPTS: usize = 16;
/// Centre spacing (on each axis) kept between spawned group members: their bodies plus a gap.
const SPAWN_SPACING: i32 = 2 * MONSTER_BODY_HALF_EXTENTS.x + 20;
const _: () = assert!(MONSTER_BODY_HALF_EXTENTS.x == MONSTER_BODY_HALF_EXTENTS.z);

/// A spawn position inside `room` that overlaps neither an already placed member nor a fixed
/// collider. The first draw is the one a lone room monster takes; a conflicting draw is
/// redrawn a bounded number of times, then the first free spot of a fixed grid over the
/// spawn area is taken, so the result is always deterministic.
fn spawn_position(
    room: &RoomSnapshot,
    placed: &[Vec3i],
    colliders: &[StaticColliderSnapshot],
    rng: &mut DungeonRng,
) -> Vec3i {
    let (min_x, max_x) = (
        room.min_x + ROOM_SPAWN_MARGIN,
        room.max_x - ROOM_SPAWN_MARGIN,
    );
    let (min_z, max_z) = (
        room.min_z + ROOM_SPAWN_MARGIN,
        room.max_z - ROOM_SPAWN_MARGIN,
    );
    let free = |x: i32, z: i32| {
        placed.iter().all(|other| {
            (other.x - x).abs() >= SPAWN_SPACING || (other.z - z).abs() >= SPAWN_SPACING
        }) && colliders.iter().all(|collider| {
            (collider.position[0] - x).abs()
                >= collider.half_extents[0] + MONSTER_BODY_HALF_EXTENTS.x
                || (collider.position[2] - z).abs()
                    >= collider.half_extents[2] + MONSTER_BODY_HALF_EXTENTS.z
        })
    };
    for _ in 0..SPAWN_ATTEMPTS {
        let x = rng.range_i32(min_x, max_x);
        let z = rng.range_i32(min_z, max_z);
        if free(x, z) {
            return Vec3i::new(x, PLAYER_Y, z);
        }
    }
    let step = usize::try_from(SPAWN_SPACING).expect("spacing is positive");
    (min_z..=max_z)
        .step_by(step)
        .flat_map(|z| (min_x..=max_x).step_by(step).map(move |x| (x, z)))
        .find(|&(x, z)| free(x, z))
        .map(|(x, z)| Vec3i::new(x, PLAYER_Y, z))
        .expect("a combat room's spawn area holds a full encounter group")
}

fn push_door(
    doors: &mut Vec<DoorSnapshot>,
    room_a: RoomId,
    room_b: RoomId,
    position: [i32; 3],
    half_extents: [i32; 3],
) {
    let index = u64::try_from(doors.len()).expect("dungeon door count must fit u64");
    doors.push(DoorSnapshot {
        id: DOOR_BODY_BASE + index,
        room_a,
        room_b,
        position,
        half_extents,
        locked: false,
    });
}

fn push_vertical_wall_with_door(
    colliders: &mut Vec<StaticColliderSnapshot>,
    x: i32,
    z_min: i32,
    z_max: i32,
    doorway_z: i32,
) {
    push_vertical_wall_segment(colliders, x, z_min, doorway_z - DOOR_HALF_WIDTH);
    push_vertical_wall_segment(colliders, x, doorway_z + DOOR_HALF_WIDTH, z_max);
}

fn push_vertical_wall_segment(
    colliders: &mut Vec<StaticColliderSnapshot>,
    x: i32,
    z_min: i32,
    z_max: i32,
) {
    if z_max <= z_min {
        return;
    }
    let half_depth = (z_max - z_min) / 2;
    if half_depth == 0 {
        return;
    }
    push_wall(
        colliders,
        [x, PLAYER_Y, z_min + half_depth],
        [WALL_HALF_THICKNESS, WALL_HALF_HEIGHT, half_depth],
    );
}

fn push_horizontal_wall_with_door(
    colliders: &mut Vec<StaticColliderSnapshot>,
    z: i32,
    x_min: i32,
    x_max: i32,
    doorway_x: i32,
) {
    push_horizontal_wall_segment(colliders, z, x_min, doorway_x - DOOR_HALF_WIDTH);
    push_horizontal_wall_segment(colliders, z, doorway_x + DOOR_HALF_WIDTH, x_max);
}

fn push_horizontal_wall_segment(
    colliders: &mut Vec<StaticColliderSnapshot>,
    z: i32,
    x_min: i32,
    x_max: i32,
) {
    if x_max <= x_min {
        return;
    }
    let half_width = (x_max - x_min) / 2;
    if half_width == 0 {
        return;
    }
    push_wall(
        colliders,
        [x_min + half_width, PLAYER_Y, z],
        [half_width, WALL_HALF_HEIGHT, WALL_HALF_THICKNESS],
    );
}

fn push_wall(
    colliders: &mut Vec<StaticColliderSnapshot>,
    position: [i32; 3],
    half_extents: [i32; 3],
) {
    let index = u64::try_from(colliders.len()).expect("dungeon collider count must fit u64");
    colliders.push(StaticColliderSnapshot {
        id: STATIC_BODY_BASE + index,
        position,
        half_extents,
        kind: StaticColliderKind::Wall,
    });
}

const fn vec_to_array(value: Vec3i) -> [i32; 3] {
    [value.x, value.y, value.z]
}

const fn array_to_vec(value: [i32; 3]) -> Vec3i {
    Vec3i::new(value[0], value[1], value[2])
}

/// The farthest path point in direct line from `position`.
fn path_point(grid: &RoomGrid, position: Vector, path: &[(i32, i32)]) -> Option<(i32, i32)> {
    let from = (position.x, position.z);
    path.iter()
        .rev()
        .find(|&&point| grid.segment_is_free(from, point))
        // Pressed against an obstacle: first step out to the path's free start cell.
        .or_else(|| path.first())
        .copied()
}

/// Velocity toward the farthest path point in direct line, at most `speed` per tick.
#[cfg(test)]
fn pursuit_velocity(grid: &RoomGrid, position: Vector, path: &[(i32, i32)], speed: i32) -> Vector {
    path_point(grid, position, path).map_or(Vector::ZERO, |point| {
        scaled_velocity(
            xz_delta(position, layout_point(point, position.y)),
            f64::from(speed),
        )
    })
}

/// A velocity along (dx, dz) of at most `speed` and at most the vector's own length.
fn scaled_velocity((dx, dz): (f64, f64), speed: f64) -> Vector {
    let length = planar_length(dx, dz);
    if length == 0.0 || !length.is_finite() {
        return Vector::ZERO;
    }
    let scale = speed.min(length) / length;
    Vector::new((dx * scale) as f32, 0.0, (dz * scale) as f32)
}

/// A monster shot's velocity along (dx, dz) at `speed` per tick (#108). `None` for a zero
/// direction.
fn projectile_velocity((dx, dz): (f64, f64), speed: i32) -> Option<Vector> {
    let length = planar_length(dx, dz);
    if length == 0.0 || !length.is_finite() {
        return None;
    }
    let scale = f64::from(speed) / length;
    Some(Vector::new((dx * scale) as f32, 0.0, (dz * scale) as f32))
}

/// Largest difference between a shot's planar speed and its definition's that `f32`
/// rounding of the committed velocity can produce, with a wide margin.
const PROJECTILE_SPEED_TOLERANCE: f64 = 1.0e-3;

/// Whether `velocity` is one a shot at `speed` can have.
fn projectile_velocity_is_possible(velocity: Vector, speed: i32) -> bool {
    velocity.is_finite()
        && velocity.y == 0.0
        && speed > 0
        && (planar_length(f64::from(velocity.x), f64::from(velocity.z)) - f64::from(speed)).abs()
            <= PROJECTILE_SPEED_TOLERANCE
}

/// How much harder a blocker ahead pushes sideways than straight away.
const SIDESTEP_WEIGHT: f64 = 3.0;

/// The sign of `value` as -1, 0 or 1 (`f64::signum` has no zero).
fn sign(value: f64) -> i64 {
    i64::from(value > 0.0) - i64::from(value < 0.0)
}

/// Local separation (#109): the velocity toward `point`, bent away from other monster
/// bodies of the same room within `range`. Each neighbour pushes straight away with a
/// strength that grows linearly as the gap closes. A neighbour ahead of the mover also
/// pushes it sideways, `SIDESTEP_WEIGHT` times as hard, so a pursuer walks around a
/// monster in its way instead of queueing behind it. All sidesteps of one mover go to the
/// same side: the side with fewer monsters nearer to `goal` than the mover (around the
/// crowd toward free space), else the side the neighbours' pushes lean to, else the side
/// of the first blocker it is on, else by id. Physics keeps bodies from overlapping;
/// separation keeps pursuers from stacking up on one approach. With no neighbour in range
/// this is exactly the plain pursuit velocity.
fn separated_velocity(
    monster: &MonsterState,
    goal: Vector,
    point: (i32, i32),
    speed: i32,
    bodies: &[(u32, RoomId, Vector)],
    range: i64,
) -> Vector {
    let position = monster.position;
    let speed = f64::from(speed);
    let to_point = xz_delta(position, layout_point(point, position.y));
    let range_length = range as f64;
    // (id, unit vector away from the neighbour, strength in 0..=1).
    let neighbours = bodies
        .iter()
        .filter(|&&(id, room_id, other)| {
            id != monster.id
                && room_id == monster.room_id
                && xz_distance_sq(position, other) < range_sq(range)
        })
        .map(|&(other_id, _, other)| {
            let away = xz_delta(other, position);
            let distance = planar_length(away.0, away.1);
            let away = if distance == 0.0 {
                // Coincident centres: the lower id steps toward -x.
                let sign = if monster.id < other_id { -1.0 } else { 1.0 };
                (sign, 0.0)
            } else {
                (away.0 / distance, away.1 / distance)
            };
            (other_id, away, (range_length - distance) / range_length)
        })
        .collect::<Vec<_>>();
    if neighbours.is_empty() {
        return scaled_velocity(to_point, speed);
    }
    let length = planar_length(to_point.0, to_point.1);
    let direction = if length == 0.0 {
        (0.0, 0.0)
    } else {
        (to_point.0 / length, to_point.1 / length)
    };
    let left = (-direction.1, direction.0);
    let lateral = |away: (f64, f64)| left.0 * away.0 + left.1 * away.1;
    let lean = neighbours
        .iter()
        .map(|&(_, away, strength)| lateral(away) * strength)
        .sum::<f64>();
    let blockers = neighbours
        .iter()
        .filter(|&&(_, away, _)| direction.0 * away.0 + direction.1 * away.1 < 0.0)
        .collect::<Vec<_>>();
    // Monsters nearer the goal on the mover's left minus those on its right.
    let goal_distance_sq = xz_distance_sq(position, goal);
    let crowding = bodies
        .iter()
        .filter(|&&(id, room_id, other)| {
            id != monster.id
                && room_id == monster.room_id
                && xz_distance_sq(other, goal) < goal_distance_sq
        })
        .map(|&(_, _, other)| -sign(lateral(xz_delta(other, position))))
        .sum::<i64>();
    let side = match (-crowding.signum(), sign(lean), blockers.first()) {
        (0, 0, Some(&&(other_id, away, _))) => match sign(lateral(away)) {
            0 if monster.id < other_id => 1,
            0 => -1,
            sign => sign,
        },
        (0, sign, _) | (sign, _, _) => sign,
    } as f64;
    let mut steer = direction;
    for &(_, away, strength) in &neighbours {
        steer.0 += away.0 * strength;
        steer.1 += away.1 * strength;
    }
    for &&(_, _, strength) in &blockers {
        steer.0 += SIDESTEP_WEIGHT * side * left.0 * strength;
        steer.1 += SIDESTEP_WEIGHT * side * left.1 * strength;
    }
    // The steer is a direction: the mover walks it at full speed.
    let length = planar_length(steer.0, steer.1);
    if length == 0.0 || !length.is_finite() {
        return Vector::ZERO;
    }
    let scale = speed / length;
    Vector::new((steer.0 * scale) as f32, 0.0, (steer.1 * scale) as f32)
}

/// A whole-unit layout point (a navigation cell centre) on the plane at height `y`.
fn layout_point((x, z): (i32, i32), y: f32) -> Vector {
    Vector::new(x as f32, y, z as f32)
}

/// The length of a planar vector, from correctly rounded operations only (unlike
/// `f64::hypot`, whose platform library may differ), so every build agrees.
fn planar_length(x: f64, z: f64) -> f64 {
    (x * x + z * z).sqrt()
}

/// The planar offset from `from` to `to`, in `f64`: exact for `f32` positions.
fn xz_delta(from: Vector, to: Vector) -> (f64, f64) {
    (
        f64::from(to.x) - f64::from(from.x),
        f64::from(to.z) - f64::from(from.z),
    )
}

/// The squared planar distance between two positions.
fn xz_distance_sq(a: Vector, b: Vector) -> f64 {
    let (dx, dz) = xz_delta(a, b);
    dx * dx + dz * dz
}

/// A whole-unit range, squared, for comparison with [`xz_distance_sq`].
fn range_sq(range: i64) -> f64 {
    let range = range as f64;
    range * range
}

/// Whether a squared planar distance lies within `range` (inclusive).
fn within_range(distance_sq: f64, range: i64) -> bool {
    distance_sq <= range_sq(range)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BRUTE: &str = "monster.brute";
    const SKIRMISHER: &str = "monster.skirmisher";
    const ARCHER: &str = "monster.archer";
    const BRUISER: &str = "monster.bruiser";
    const DEFENDER: &str = "monster.defender";

    /// Canonical index of a monster definition in the built-in content.
    fn definition_index(id: &str) -> usize {
        content()
            .monsters
            .iter()
            .position(|definition| *content().id(definition.id) == *id)
            .expect("built-in content defines this monster")
    }

    fn brute() -> &'static MonsterDefinition {
        content().monster(definition_index(BRUTE))
    }

    fn monster_claw() -> StrikeDefinition {
        brute().strike
    }

    /// The components of a value the rules keep in whole world units (a player's arrow
    /// launched from a whole-unit position), as integers.
    fn whole(value: Vector) -> [i32; 3] {
        let components = value.to_array();
        assert!(
            components.iter().all(|component| component.fract() == 0.0),
            "{components:?} has whole components"
        );
        components.map(|component| component as i32)
    }

    #[test]
    fn command_sequence_must_be_non_zero() {
        assert_eq!(
            PlayerCommand::new(7, 0, ()).unwrap_err().message(),
            "command sequence must be non-zero"
        );
    }

    fn movement_state(x: i8, z: i8) -> PlayerState {
        PlayerState {
            position: Vector::ZERO,
            velocity: Vector::ZERO,
            grounded: true,
            coyote_ticks_remaining: 0,
            jump_buffer_ticks_remaining: 0,
            region: RegionMembership::Region {
                id: DUNGEON_FLOOR_REGION,
            },
            last_supported: Vector::ZERO,
            movement_x: x,
            movement_z: z,
            facing_x: if x == 0 { 1 } else { x },
            facing_z: z,
            action: None,
            hurt_ticks_remaining: 0,
            guard: GuardState::ready(content().guard.max_points),
            counter: None,
            weapon: Weapon::SwordAndShield,
            draw_ticks: None,
            aim: None,
            locked_monster_id: None,
            health: BASE_MAX_HEALTH,
            experience: 0,
            gold: 0,
        }
    }

    #[test]
    fn each_gameplay_tick_steps_physics_once() {
        let mut game = ArpgGame::new().unwrap();
        game.add_player(1).unwrap();
        assert_eq!(game.physics_steps(), 0);
        for _ in 0..5 {
            game.advance_tick().unwrap();
        }
        assert_eq!(game.physics_steps(), 5);
    }

    #[test]
    fn movement_speed_is_tuned_for_precise_room_navigation() {
        let cardinal = ArpgGame::movement_target_velocity(movement_state(1, 0));
        let diagonal = ArpgGame::movement_target_velocity(movement_state(1, 1));
        assert_eq!(cardinal, Vector::from(Vec3i::new(7, 0, 0)));
        assert_eq!(diagonal, Vector::from(Vec3i::new(5, 0, 5)));
    }

    #[test]
    fn locomotion_accelerates_brakes_and_reverses_over_bounded_ticks() {
        let forward = movement_state(1, 0);
        let idle = movement_state(0, 0);
        let reverse = movement_state(-1, 0);

        let first =
            ArpgGame::controlled_movement_velocity(Vector::ZERO, forward, ControlRates::GROUND);
        let second = ArpgGame::controlled_movement_velocity(first, forward, ControlRates::GROUND);
        let full = ArpgGame::controlled_movement_velocity(second, forward, ControlRates::GROUND);
        assert_eq!(first, Vector::from(Vec3i::new(3, 0, 0)));
        assert_eq!(second, Vector::from(Vec3i::new(6, 0, 0)));
        assert_eq!(full, Vector::from(Vec3i::new(7, 0, 0)));

        let braking = ArpgGame::controlled_movement_velocity(full, idle, ControlRates::GROUND);
        let stopped = ArpgGame::controlled_movement_velocity(braking, idle, ControlRates::GROUND);
        assert_eq!(braking, Vector::from(Vec3i::new(2, 0, 0)));
        assert_eq!(stopped, Vector::ZERO);

        let turning = ArpgGame::controlled_movement_velocity(full, reverse, ControlRates::GROUND);
        let crossed_zero =
            ArpgGame::controlled_movement_velocity(turning, reverse, ControlRates::GROUND);
        let accelerating_reverse =
            ArpgGame::controlled_movement_velocity(crossed_zero, reverse, ControlRates::GROUND);
        let reversed = ArpgGame::controlled_movement_velocity(
            accelerating_reverse,
            reverse,
            ControlRates::GROUND,
        );
        assert_eq!(turning, Vector::from(Vec3i::new(2, 0, 0)));
        assert_eq!(crossed_zero, Vector::from(Vec3i::new(-3, 0, 0)));
        assert_eq!(accelerating_reverse, Vector::from(Vec3i::new(-6, 0, 0)));
        assert_eq!(reversed, Vector::from(Vec3i::new(-7, 0, 0)));
    }

    #[test]
    fn diagonal_control_normalizes_acceleration_braking_and_reversal() {
        let forward = movement_state(1, 1);
        let idle = movement_state(0, 0);
        let reverse = movement_state(-1, -1);

        let first =
            ArpgGame::controlled_movement_velocity(Vector::ZERO, forward, ControlRates::GROUND);
        let second = ArpgGame::controlled_movement_velocity(first, forward, ControlRates::GROUND);
        let full = ArpgGame::controlled_movement_velocity(second, forward, ControlRates::GROUND);
        assert_eq!(first, Vector::from(Vec3i::new(2, 0, 2)));
        assert_eq!(second, Vector::from(Vec3i::new(4, 0, 4)));
        assert_eq!(full, Vector::from(Vec3i::new(5, 0, 5)));
        assert!(
            first.x * first.x + first.z * first.z <= PLAYER_ACCELERATION_PER_TICK.pow(2) as f32
        );

        let braking = ArpgGame::controlled_movement_velocity(full, idle, ControlRates::GROUND);
        let stopped = ArpgGame::controlled_movement_velocity(braking, idle, ControlRates::GROUND);
        assert_eq!(braking, Vector::from(Vec3i::new(2, 0, 2)));
        assert_eq!(stopped, Vector::ZERO);

        let turning = ArpgGame::controlled_movement_velocity(full, reverse, ControlRates::GROUND);
        let crossed_zero =
            ArpgGame::controlled_movement_velocity(turning, reverse, ControlRates::GROUND);
        let accelerating_reverse =
            ArpgGame::controlled_movement_velocity(crossed_zero, reverse, ControlRates::GROUND);
        let reversed = ArpgGame::controlled_movement_velocity(
            accelerating_reverse,
            reverse,
            ControlRates::GROUND,
        );
        assert_eq!(turning, Vector::from(Vec3i::new(2, 0, 2)));
        assert_eq!(crossed_zero, Vector::from(Vec3i::new(-1, 0, -1)));
        assert_eq!(accelerating_reverse, Vector::from(Vec3i::new(-3, 0, -3)));
        assert_eq!(reversed, Vector::from(Vec3i::new(-5, 0, -5)));
    }

    #[test]
    fn action_commitment_stops_controlled_movement_immediately() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        game.add_player(1).unwrap();
        game.apply_command(
            PlayerCommand::new(1, 1, ArpgCommand::SetMovement { x: 1, z: 0 }).unwrap(),
        )
        .unwrap();
        for _ in 0..3 {
            game.advance_tick().unwrap();
        }
        assert_eq!(player_velocity(&game, 1), Vec3i::new(7, 0, 0).into());

        game.apply_command(
            PlayerCommand::new(1, 2, ArpgCommand::PrimaryAttack { aim: None }).unwrap(),
        )
        .unwrap();
        game.advance_tick().unwrap();

        assert_eq!(player_velocity(&game, 1), Vector::ZERO);
    }

    #[test]
    fn diagonal_control_slides_along_outer_wall() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        game.add_player(1).unwrap();
        let contact_x = ARENA_HALF_WIDTH - WALL_HALF_THICKNESS - PLAYER_HALF_EXTENTS.x;
        let start_z = -1_000;
        game.put_player(1, Vec3i::new(contact_x, PLAYER_Y, start_z));
        game.apply_command(
            PlayerCommand::new(1, 1, ArpgCommand::SetMovement { x: 1, z: 1 }).unwrap(),
        )
        .unwrap();

        for _ in 0..10 {
            game.advance_tick().unwrap();
        }

        // Pressed against the wall, short only of Rapier's contact offset, and sliding.
        let position = game.players[&1].position;
        assert!(
            (contact_x as f32 - position.x).abs() <= CONTACT_TOLERANCE,
            "{position:?}"
        );
        assert!(position.z > (start_z + 30) as f32);
    }

    /// How far from an exact touching position a character resting against geometry may
    /// stand: Rapier's controller keeps a 0.25 unit contact offset.
    const CONTACT_TOLERANCE: f32 = 0.5;

    fn player_velocity(game: &ArpgGame, player_id: PlayerId) -> Vector {
        game.players[&player_id].velocity
    }

    fn run_action(game: &mut ArpgGame, player_id: PlayerId, sequence: u32, command: ArpgCommand) {
        game.apply_command(PlayerCommand::new(player_id, sequence, command).unwrap())
            .unwrap();
        while game.players.get(&player_id).unwrap().action.is_some() {
            game.advance_tick().unwrap();
        }
    }

    #[test]
    fn progression_is_derived_from_experience_and_changes_combat_stats() {
        let mut game = ArpgGame::new().unwrap();
        game.add_player(1).unwrap();
        game.award_experience(1, EXPERIENCE_PER_LEVEL).unwrap();

        let player = &game.snapshot().unwrap().players[0];
        assert_eq!(player.experience, EXPERIENCE_PER_LEVEL);
        assert_eq!(player.level, 2);
        assert_eq!(player.max_health, 110);
        assert_eq!(player.health, 110);
        assert_eq!(player.attack_damage, 30);
        assert_eq!(player.experience_into_level, 0);
        assert_eq!(player.experience_for_next_level, EXPERIENCE_PER_LEVEL);
    }

    #[test]
    fn killing_blow_gets_experience_once_in_coop() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        let room_id = 2;
        let (center_x, center_z) = game
            .rooms
            .iter()
            .find(|room| room.id == room_id)
            .unwrap()
            .center();
        game.player_spawns[0] = Vec3i::new(center_x, PLAYER_Y, center_z);
        game.player_spawns[1] = Vec3i::new(center_x + 20, PLAYER_Y, center_z);
        game.add_player(1).unwrap();
        game.add_player(2).unwrap();
        game.reconcile_encounters().unwrap();
        let monster = game
            .monsters
            .iter_mut()
            .find(|monster| monster.room_id == room_id)
            .unwrap();
        monster.position = Vec3i::new(center_x + 100, PLAYER_Y, center_z).into();

        for sequence in 1..=3 {
            run_action(
                &mut game,
                1,
                sequence,
                ArpgCommand::PrimaryAttack { aim: None },
            );
        }
        run_action(&mut game, 2, 1, ArpgCommand::PrimaryAttack { aim: None });

        let snapshot = game.snapshot().unwrap();
        let first = snapshot
            .players
            .iter()
            .find(|player| player.id == 1)
            .unwrap();
        let second = snapshot
            .players
            .iter()
            .find(|player| player.id == 2)
            .unwrap();
        assert_eq!(first.experience, 0);
        assert_eq!(second.experience, MONSTER_EXPERIENCE_REWARD);
        assert_eq!(snapshot.ground_loot.len(), 1);

        run_action(&mut game, 2, 2, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(
            game.snapshot()
                .unwrap()
                .players
                .iter()
                .find(|player| player.id == 2)
                .unwrap()
                .experience,
            MONSTER_EXPERIENCE_REWARD
        );
    }

    #[test]
    fn level_damage_is_used_by_authoritative_attack() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        let room_id = 2;
        let (center_x, center_z) = game
            .rooms
            .iter()
            .find(|room| room.id == room_id)
            .unwrap()
            .center();
        game.player_spawns[0] = Vec3i::new(center_x, PLAYER_Y, center_z);
        game.add_player(1).unwrap();
        game.award_experience(1, EXPERIENCE_PER_LEVEL).unwrap();
        game.reconcile_encounters().unwrap();
        let monster = game
            .monsters
            .iter_mut()
            .find(|monster| monster.room_id == room_id)
            .unwrap();
        monster.position = Vec3i::new(center_x + 100, PLAYER_Y, center_z).into();
        monster.health = 30;

        run_action(&mut game, 1, 1, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(
            game.monsters
                .iter()
                .find(|monster| monster.room_id == room_id)
                .unwrap()
                .health,
            0
        );
        assert_eq!(
            game.players.get(&1).unwrap().experience,
            EXPERIENCE_PER_LEVEL + MONSTER_EXPERIENCE_REWARD
        );
    }

    #[test]
    fn secondary_attack_trades_reach_for_more_damage() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        let room_id = 2;
        let (center_x, center_z) = game
            .rooms
            .iter()
            .find(|room| room.id == room_id)
            .unwrap()
            .center();
        game.player_spawns[0] = Vec3i::new(center_x, PLAYER_Y, center_z);
        game.add_player(1).unwrap();
        game.reconcile_encounters().unwrap();

        let monster = game
            .monsters
            .iter_mut()
            .find(|monster| monster.room_id == room_id)
            .unwrap();
        monster.position = Vec3i::new(center_x + 180, PLAYER_Y, center_z).into();
        monster.health = 100;

        run_action(&mut game, 1, 1, ArpgCommand::SecondaryAttack { aim: None });
        assert_eq!(
            game.monsters
                .iter()
                .find(|monster| monster.room_id == room_id)
                .unwrap()
                .health,
            100
        );

        run_action(&mut game, 1, 2, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(
            game.monsters
                .iter()
                .find(|monster| monster.room_id == room_id)
                .unwrap()
                .health,
            75
        );

        game.monsters
            .iter_mut()
            .find(|monster| monster.room_id == room_id)
            .unwrap()
            .position = Vec3i::new(center_x + 100, PLAYER_Y, center_z).into();
        run_action(&mut game, 1, 3, ArpgCommand::SecondaryAttack { aim: None });
        assert_eq!(
            game.monsters
                .iter()
                .find(|monster| monster.room_id == room_id)
                .unwrap()
                .health,
            38
        );
    }

    #[test]
    fn monster_attack_is_telegraphed_before_damage() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        let room_id = 2;
        let (center_x, center_z) = game
            .rooms
            .iter()
            .find(|room| room.id == room_id)
            .unwrap()
            .center();
        game.player_spawns[0] = Vec3i::new(center_x, PLAYER_Y, center_z);
        game.add_player(1).unwrap();
        game.reconcile_encounters().unwrap();
        let monster = game
            .monsters
            .iter_mut()
            .find(|monster| monster.room_id == room_id)
            .unwrap();
        monster.position = Vec3i::new(center_x + 100, PLAYER_Y, center_z).into();

        game.advance_tick().unwrap();

        let telegraph = game.snapshot().unwrap();
        assert_eq!(telegraph.players[0].health, BASE_MAX_HEALTH);
        let monster_action = telegraph
            .monsters
            .iter()
            .find(|monster| monster.room_id == room_id)
            .unwrap()
            .action
            .unwrap();
        assert_eq!(monster_action.phase, ActionPhase::Windup);
        assert_eq!(monster_action.ticks_remaining, MONSTER_ATTACK_WINDUP_TICKS);
        assert_eq!(monster_action.target_player_id, 1);
        assert_eq!(
            monster_action.range,
            i32::try_from(MONSTER_ATTACK_RANGE).unwrap()
        );

        for _ in 0..MONSTER_ATTACK_WINDUP_TICKS {
            game.advance_tick().unwrap();
        }

        let impact = game.snapshot().unwrap();
        assert_eq!(
            impact.players[0].health,
            BASE_MAX_HEALTH - MONSTER_ATTACK_DAMAGE
        );
        assert_eq!(
            impact.players[0].reaction.unwrap().kind,
            PlayerReactionKind::Hurt
        );
        assert_eq!(
            impact
                .monsters
                .iter()
                .find(|monster| monster.room_id == room_id)
                .unwrap()
                .action
                .unwrap()
                .phase,
            ActionPhase::Active
        );
    }

    #[test]
    fn moving_out_of_monster_telegraph_causes_a_miss() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        let room_id = 2;
        let (center_x, center_z) = game
            .rooms
            .iter()
            .find(|room| room.id == room_id)
            .unwrap()
            .center();
        game.player_spawns[0] = Vec3i::new(center_x, PLAYER_Y, center_z);
        game.add_player(1).unwrap();
        game.reconcile_encounters().unwrap();
        game.monsters
            .iter_mut()
            .find(|monster| monster.room_id == room_id)
            .unwrap()
            .position = Vec3i::new(center_x + 100, PLAYER_Y, center_z).into();

        game.advance_tick().unwrap();
        game.apply_command(
            PlayerCommand::new(1, 1, ArpgCommand::SetMovement { x: -1, z: 0 }).unwrap(),
        )
        .unwrap();
        for _ in 0..MONSTER_ATTACK_WINDUP_TICKS {
            game.advance_tick().unwrap();
        }

        let snapshot = game.snapshot().unwrap();
        assert_eq!(snapshot.players[0].health, BASE_MAX_HEALTH);
        assert!(
            snapshot.players[0].position[0] <= center_x - 100,
            "player did not leave the telegraphed melee range"
        );
    }

    #[test]
    fn defeated_player_cannot_move_or_start_actions() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        let room_id = 2;
        let (center_x, center_z) = game
            .rooms
            .iter()
            .find(|room| room.id == room_id)
            .unwrap()
            .center();
        game.player_spawns[0] = Vec3i::new(center_x, PLAYER_Y, center_z);
        game.add_player(1).unwrap();
        game.players.get_mut(&1).unwrap().health = MONSTER_ATTACK_DAMAGE;
        game.reconcile_encounters().unwrap();
        game.monsters
            .iter_mut()
            .find(|monster| monster.room_id == room_id)
            .unwrap()
            .position = Vec3i::new(center_x + 100, PLAYER_Y, center_z).into();

        game.advance_tick().unwrap();
        for _ in 0..MONSTER_ATTACK_WINDUP_TICKS {
            game.advance_tick().unwrap();
        }

        let defeated = game.snapshot().unwrap().players[0].clone();
        assert_eq!(defeated.health, 0);
        assert!(!defeated.alive);
        let position = defeated.position;

        game.apply_command(
            PlayerCommand::new(1, 1, ArpgCommand::SetMovement { x: -1, z: 0 }).unwrap(),
        )
        .unwrap();
        game.apply_command(
            PlayerCommand::new(1, 2, ArpgCommand::PrimaryAttack { aim: None }).unwrap(),
        )
        .unwrap();
        for _ in 0..PRIMARY_WINDUP_TICKS {
            game.advance_tick().unwrap();
        }

        let after = game.snapshot().unwrap().players[0].clone();
        assert_eq!(after.position, position);
        assert!(after.action.is_none());
        assert_eq!(after.health, 0);
    }

    #[test]
    fn player_stagger_interrupts_monster_windup() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        let room_id = 2;
        let (center_x, center_z) = game
            .rooms
            .iter()
            .find(|room| room.id == room_id)
            .unwrap()
            .center();
        game.player_spawns[0] = Vec3i::new(center_x, PLAYER_Y, center_z);
        game.add_player(1).unwrap();
        game.reconcile_encounters().unwrap();
        game.monsters
            .iter_mut()
            .find(|monster| monster.room_id == room_id)
            .unwrap()
            .position = Vec3i::new(center_x + 100, PLAYER_Y, center_z).into();

        game.advance_tick().unwrap();
        game.apply_command(
            PlayerCommand::new(1, 1, ArpgCommand::PrimaryAttack { aim: None }).unwrap(),
        )
        .unwrap();
        for _ in 0..PRIMARY_WINDUP_TICKS {
            game.advance_tick().unwrap();
        }

        let monster = game
            .snapshot()
            .unwrap()
            .monsters
            .into_iter()
            .find(|monster| monster.room_id == room_id)
            .unwrap();
        assert_eq!(monster.health, 75);
        assert!(monster.action.is_none());
        assert_eq!(monster.reaction.unwrap().kind, MonsterReactionKind::Stagger);
    }

    #[test]
    fn kill_drop_pickup_requires_explicit_interaction() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        let room_id = 2;
        let (center_x, center_z) = game
            .rooms
            .iter()
            .find(|room| room.id == room_id)
            .unwrap()
            .center();
        game.player_spawns[0] = Vec3i::new(center_x, PLAYER_Y, center_z);
        game.add_player(1).unwrap();
        game.reconcile_encounters().unwrap();
        let monster = game
            .monsters
            .iter_mut()
            .find(|monster| monster.room_id == room_id)
            .unwrap();
        monster.position = Vec3i::new(center_x + 100, PLAYER_Y, center_z).into();
        monster.health = BASE_ATTACK_DAMAGE;

        run_action(&mut game, 1, 1, ArpgCommand::PrimaryAttack { aim: None });

        let after_kill = game.snapshot().unwrap();
        assert_eq!(after_kill.players[0].gold, 0);
        assert_eq!(after_kill.ground_loot.len(), 1);
        assert_eq!(after_kill.ground_loot[0].kind, LootKind::Gold);
        assert_eq!(after_kill.ground_loot[0].amount, GROUND_LOOT_GOLD_AMOUNT);
        assert_eq!(
            after_kill.ground_loot[0].position,
            [center_x + 100, PLAYER_Y, center_z]
        );

        run_action(&mut game, 1, 2, ArpgCommand::Interact);

        let after_pickup = game.snapshot().unwrap();
        assert!(after_pickup.ground_loot.is_empty());
        assert_eq!(after_pickup.players[0].gold, GROUND_LOOT_GOLD_AMOUNT);

        run_action(&mut game, 1, 3, ArpgCommand::Interact);
        assert_eq!(
            game.snapshot().unwrap().players[0].gold,
            GROUND_LOOT_GOLD_AMOUNT
        );
    }

    #[test]
    fn interaction_respects_pickup_range() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        game.add_player(1).unwrap();
        let player_position = game.players[&1].position;
        game.ground_loot.push(GroundLootState {
            id: GROUND_LOOT_ID_BASE,
            position: Vector::new(
                player_position.x + INTERACT_RANGE as f32 + 1.0,
                PLAYER_Y as f32,
                player_position.z,
            ),
            kind: LootKind::Gold,
            amount: GROUND_LOOT_GOLD_AMOUNT,
        });

        run_action(&mut game, 1, 1, ArpgCommand::Interact);

        assert_eq!(game.snapshot().unwrap().ground_loot.len(), 1);
        assert_eq!(game.snapshot().unwrap().players[0].gold, 0);
    }

    #[test]
    fn attacks_have_authoritative_commitment_active_and_recovery_phases() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        let room_id = 2;
        let (center_x, center_z) = game
            .rooms
            .iter()
            .find(|room| room.id == room_id)
            .unwrap()
            .center();
        game.player_spawns[0] = Vec3i::new(center_x, PLAYER_Y, center_z);
        game.add_player(1).unwrap();
        game.reconcile_encounters().unwrap();
        let monster = game
            .monsters
            .iter_mut()
            .find(|monster| monster.room_id == room_id)
            .unwrap();
        monster.position = Vec3i::new(center_x + 100, PLAYER_Y, center_z).into();

        game.apply_command(
            PlayerCommand::new(1, 1, ArpgCommand::SetMovement { x: 1, z: 0 }).unwrap(),
        )
        .unwrap();
        game.apply_command(
            PlayerCommand::new(1, 2, ArpgCommand::PrimaryAttack { aim: None }).unwrap(),
        )
        .unwrap();

        let start_position = game.snapshot().unwrap().players[0].position;
        let action = game.snapshot().unwrap().players[0].action.unwrap();
        assert_eq!(action.kind, ActionKind::PrimaryAttack);
        assert_eq!(action.phase, ActionPhase::Windup);
        assert_eq!(action.ticks_remaining, PRIMARY_WINDUP_TICKS);

        for _ in 0..PRIMARY_WINDUP_TICKS {
            game.advance_tick().unwrap();
        }

        let active = game.snapshot().unwrap();
        assert_eq!(active.players[0].position, start_position);
        assert_eq!(active.players[0].action.unwrap().phase, ActionPhase::Active);
        assert_eq!(
            active
                .monsters
                .iter()
                .find(|monster| monster.room_id == room_id)
                .unwrap()
                .health,
            75
        );
        assert_eq!(
            active
                .monsters
                .iter()
                .find(|monster| monster.room_id == room_id)
                .unwrap()
                .reaction
                .unwrap()
                .kind,
            MonsterReactionKind::Stagger
        );

        game.advance_tick().unwrap();
        assert_eq!(
            game.snapshot().unwrap().players[0].action.unwrap().phase,
            ActionPhase::Recovery
        );
        for _ in 0..PRIMARY_RECOVERY_TICKS {
            game.advance_tick().unwrap();
        }
        assert!(game.snapshot().unwrap().players[0].action.is_none());
        game.advance_tick().unwrap();
        assert!(game.snapshot().unwrap().players[0].position[0] > start_position[0]);
    }

    #[test]
    fn attack_targeting_respects_committed_facing() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        let room_id = 2;
        let (center_x, center_z) = game
            .rooms
            .iter()
            .find(|room| room.id == room_id)
            .unwrap()
            .center();
        game.player_spawns[0] = Vec3i::new(center_x, PLAYER_Y, center_z);
        game.add_player(1).unwrap();
        game.reconcile_encounters().unwrap();
        let monster = game
            .monsters
            .iter_mut()
            .find(|monster| monster.room_id == room_id)
            .unwrap();
        monster.position = Vec3i::new(center_x - 100, PLAYER_Y, center_z).into();
        monster.health = 100;

        run_action(&mut game, 1, 1, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(
            game.monsters
                .iter()
                .find(|monster| monster.room_id == room_id)
                .unwrap()
                .health,
            100
        );

        game.apply_command(
            PlayerCommand::new(1, 2, ArpgCommand::SetMovement { x: -1, z: 0 }).unwrap(),
        )
        .unwrap();
        run_action(&mut game, 1, 3, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(
            game.monsters
                .iter()
                .find(|monster| monster.room_id == room_id)
                .unwrap()
                .health,
            75
        );
    }

    const STRIKE_ROOM: RoomId = 2;

    /// Player 1 at the centre of an active combat room facing +x, with every generated
    /// monster removed so fixtures place exactly the targets they describe.
    fn strike_arena() -> (ArpgGame, i32, i32) {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        let (center_x, center_z) = game
            .rooms
            .iter()
            .find(|room| room.id == STRIKE_ROOM)
            .unwrap()
            .center();
        game.player_spawns[0] = Vec3i::new(center_x, PLAYER_Y, center_z);
        game.add_player(1).unwrap();
        game.reconcile_encounters().unwrap();
        assert_eq!(
            game.rooms
                .iter()
                .find(|room| room.id == STRIKE_ROOM)
                .unwrap()
                .encounter_state,
            RoomEncounterState::Active
        );
        game.monsters.clear();
        (game, center_x, center_z)
    }

    fn place_monster(game: &mut ArpgGame, id: u32, x: i32, z: i32) {
        place_monster_of(game, id, BRUTE, x, z);
    }

    fn place_monster_of(game: &mut ArpgGame, id: u32, definition: &str, x: i32, z: i32) {
        game.monsters.push(MonsterState::new(
            id,
            *content().monster(definition_index(definition)),
            STRIKE_ROOM,
            Vec3i::new(x, PLAYER_Y, z),
            content().guard.max_points,
        ));
    }

    fn place_blocker(game: &mut ArpgGame, index: u64, x: i32, z: i32, half_extents: Vec3i) {
        game.physics
            .add_fixed(
                STATIC_BODY_BASE + 900 + index,
                Vec3i::new(x, PLAYER_Y, z),
                half_extents,
            )
            .unwrap();
    }

    fn monster_health(game: &ArpgGame, id: u32) -> u16 {
        game.monsters
            .iter()
            .find(|monster| monster.id == id)
            .unwrap()
            .health
    }

    /// Runs one player action to completion and returns the strike outcomes it produced.
    fn strike(game: &mut ArpgGame, sequence: u32, command: ArpgCommand) -> Vec<StrikeOutcome> {
        game.apply_command(PlayerCommand::new(1, sequence, command).unwrap())
            .unwrap();
        let mut outcomes = Vec::new();
        while game.players.get(&1).unwrap().action.is_some() {
            game.advance_tick().unwrap();
            outcomes.extend(
                game.strike_outcomes()
                    .iter()
                    .filter(|outcome| outcome.strike.source == StrikeSource::Player(1))
                    .cloned(),
            );
        }
        outcomes
    }

    fn targets(outcomes: &[StrikeOutcome]) -> Vec<(StrikeTarget, StrikeResult)> {
        outcomes
            .iter()
            .map(|outcome| (outcome.target, outcome.result))
            .collect()
    }

    const LIGHT_HIT: StrikeResult = StrikeResult::Hit {
        damage: BASE_ATTACK_DAMAGE,
        defeated: false,
    };

    #[test]
    fn light_swing_hits_every_target_in_front_and_nothing_behind_or_beside() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 100, z);
        place_monster(&mut game, 2, x + 150, z + 60);
        place_monster(&mut game, 3, x - 100, z);
        place_monster(&mut game, 4, x, z + 100);
        place_monster(&mut game, 5, x, z - 100);

        let outcomes = strike(&mut game, 1, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(
            targets(&outcomes),
            [
                (StrikeTarget::Monster(1), LIGHT_HIT),
                (StrikeTarget::Monster(2), LIGHT_HIT),
            ]
        );
        assert!(outcomes.iter().all(|outcome| {
            outcome.definition == "sword.lightSwing"
                && outcome.strike.source == StrikeSource::Player(1)
        }));
        for id in [3, 4, 5] {
            assert_eq!(monster_health(&game, id), 100);
        }
    }

    #[test]
    fn heavy_thrust_connects_with_one_target_by_distance_then_identity() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 8, x + 120, z + 20);
        place_monster(&mut game, 7, x + 120, z - 20);
        place_monster(&mut game, 6, x + 140, z);

        let outcomes = strike(&mut game, 1, ArpgCommand::SecondaryAttack { aim: None });
        let heavy_damage = BASE_ATTACK_DAMAGE * SECONDARY_ATTACK_DAMAGE_NUMERATOR
            / SECONDARY_ATTACK_DAMAGE_DENOMINATOR;
        assert_eq!(
            targets(&outcomes),
            [(
                StrikeTarget::Monster(7),
                StrikeResult::Hit {
                    damage: heavy_damage,
                    defeated: false
                }
            )]
        );
        assert_eq!(outcomes[0].definition, "sword.heavyThrust");
        assert_eq!(monster_health(&game, 8), 100);
        assert_eq!(monster_health(&game, 6), 100);
    }

    #[test]
    fn strike_reach_and_arc_boundaries_are_inclusive() {
        let (mut game, x, z) = strike_arena();
        // The targets stay exactly where they are placed: no pursuit, no depenetration.
        game.monsters_without_bodies = true;
        // Reach is centre-to-centre; 60 degrees either side holds when 3·dx² ≥ dz².
        place_monster(&mut game, 1, x + 220, z);
        place_monster(&mut game, 2, x + 100, z + 173);
        place_monster(&mut game, 3, x + 100, z - 173);
        place_monster(&mut game, 4, x + 221, z);
        place_monster(&mut game, 5, x + 100, z + 174);
        place_monster(&mut game, 6, x, z);

        let mut hit = targets(&strike(
            &mut game,
            1,
            ArpgCommand::PrimaryAttack { aim: None },
        ))
        .into_iter()
        .map(|(target, _)| target)
        .collect::<Vec<_>>();
        hit.sort();
        assert_eq!(
            hit,
            [1, 2, 3, 6].map(StrikeTarget::Monster),
            "overlapping, reach-edge and arc-edge targets are hit; just outside is not"
        );
    }

    /// Rules read the authoritative `f32` positions (#182): a fraction of a unit beyond
    /// reach or outside the arc misses, where a rounded view would have hit.
    #[test]
    fn sub_unit_offsets_decide_strike_reach_and_arc() {
        let (mut game, x, z) = strike_arena();
        game.monsters_without_bodies = true;
        let reach = ATTACK_RANGE as f32;
        let (x, z) = (x as f32, z as f32);
        let arranged = [
            // Exactly at reach, and 0.4 inside it: hit.
            (1, x + reach, z),
            (2, x + reach - 0.4, z + 0.0),
            // 0.4 beyond reach (rounds to reach): miss.
            (3, x + reach + 0.4, z),
            // On the 60° edge 3·dx² ≥ dz²: 173.2 is inside, 173.3 (rounds to 173) is not.
            (4, x + 100.0, z + 173.2),
            (5, x + 100.0, z - 173.3),
        ];
        for (id, mx, mz) in arranged {
            place_monster(&mut game, id, 0, 0);
            game.monsters.last_mut().unwrap().position = Vector::new(mx, PLAYER_Y as f32, mz);
        }
        let mut hit = targets(&strike(
            &mut game,
            1,
            ArpgCommand::PrimaryAttack { aim: None },
        ))
        .into_iter()
        .map(|(target, _)| target)
        .collect::<Vec<_>>();
        hit.sort();
        assert_eq!(hit, [1, 2, 4].map(StrikeTarget::Monster));

        // A player a fraction of a unit off the whole-unit grid is still measured exactly.
        let (mut game, gx, gz) = strike_arena();
        game.monsters_without_bodies = true;
        game.put_player(1, Vector::new(gx as f32 - 0.4, PLAYER_Y as f32, gz as f32));
        place_monster(&mut game, 1, gx + 220, gz);
        assert!(strike(&mut game, 1, ArpgCommand::PrimaryAttack { aim: None }).is_empty());
    }

    /// Monster reach, target-lock range and the interaction range compare exact `f32`
    /// distances (#182): 0.4 units beyond each no longer counts as inside.
    #[test]
    fn sub_unit_offsets_decide_monster_reach_lock_range_and_interaction_range() {
        let (mut game, x, z) = strike_arena();
        let player = Vector::new(x as f32, PLAYER_Y as f32, z as f32);
        game.put_player(1, player);
        place_monster(&mut game, 1, x, z);
        let monster_at = |offset: f32| {
            let mut monster = game.monsters[0];
            monster.position = player + Vector::X * offset;
            monster
        };
        let reach = brute().strike.reach as f32;
        assert!(game.can_strike(&monster_at(reach), player).unwrap());
        assert!(!game.can_strike(&monster_at(reach + 0.4), player).unwrap());
        let lock_range = content().targeting.lock_range;
        let lock = lock_range as f32;
        let floor = RegionMembership::Region {
            id: DUNGEON_FLOOR_REGION,
        };
        assert!(game.monster_lockable(&monster_at(-lock), floor, player, lock_range));
        assert!(!game.monster_lockable(&monster_at(-lock - 0.4), floor, player, lock_range));

        // End to end: a brute that cannot move winds up only from within its reach.
        for (offset, winds_up) in [(reach, true), (reach + 0.4, false)] {
            let (mut game, x, z) = strike_arena();
            game.monsters_without_bodies = true;
            place_monster(&mut game, 1, x, z);
            game.monsters[0].position = Vector::new(x as f32 + offset, PLAYER_Y as f32, z as f32);
            game.advance_tick().unwrap();
            assert_eq!(
                game.monsters[0].action.is_some(),
                winds_up,
                "offset {offset}"
            );
        }

        let mut game = ArpgGame::new_with_seed(42).unwrap();
        game.add_player(1).unwrap();
        let player = game.players[&1].position;
        let mut prompt = |offset: f32| {
            game.ground_loot = vec![GroundLootState {
                id: GROUND_LOOT_ID_BASE,
                position: player + Vector::X * offset,
                kind: LootKind::Gold,
                amount: GROUND_LOOT_GOLD_AMOUNT,
            }];
            game.snapshot().unwrap().players[0].interaction
        };
        assert_eq!(
            prompt(INTERACT_RANGE as f32),
            InteractionPrompt::Available {
                target: InteractionTarget::Loot(GROUND_LOOT_ID_BASE)
            }
        );
        assert_eq!(
            prompt(INTERACT_RANGE as f32 + 0.4),
            InteractionPrompt::Unavailable {
                reason: InteractionRefusal::NothingInRange
            }
        );
    }

    /// A lock's direction follows the exact offset: 0.4 units across 100 still turns it.
    #[test]
    fn lock_directions_keep_sub_unit_offsets() {
        let origin = Vector::new(0.0, PLAYER_Y as f32, 0.0);
        let target = Vector::new(0.4, PLAYER_Y as f32, 100.0);
        assert_eq!(
            ArpgGame::direction_between(origin, target),
            Some([4, AIM_COMPONENT_LIMIT])
        );
        assert_eq!(ArpgGame::direction_between(origin, origin), None);
    }

    #[test]
    fn strike_volume_matches_an_analytic_reference_across_a_grid() {
        for (facing, seed_offset) in [((1, 0), 0), ((0, -1), 1), ((-1, 1), 2)] {
            let (mut game, x, z) = strike_arena();
            game.monsters_without_bodies = true;
            game.players.get_mut(&1).unwrap().facing_x = facing.0;
            game.players.get_mut(&1).unwrap().facing_z = facing.1;
            let mut expected = Vec::new();
            let mut id = 0;
            for dx in (-260..=260).step_by(40) {
                for dz in (-260..=260).step_by(40) {
                    id += 1;
                    place_monster(&mut game, id, x + dx + seed_offset, z + dz);
                    let (dx, dz) = (i64::from(dx + seed_offset), i64::from(dz));
                    let (fx, fz) = (i64::from(facing.0), i64::from(facing.1));
                    let distance_sq = dx * dx + dz * dz;
                    let along = dx * fx + dz * fz;
                    let across = dx * fz - dz * fx;
                    // Inside 60° of facing: positive projection and |across| ≤ √3·along.
                    let in_arc =
                        distance_sq == 0 || (along > 0 && across * across <= 3 * along * along);
                    if distance_sq <= ATTACK_RANGE * ATTACK_RANGE && in_arc {
                        expected.push((distance_sq, id));
                    }
                }
            }
            expected.sort_unstable();
            let outcomes = strike(&mut game, 1, ArpgCommand::PrimaryAttack { aim: None });
            assert!(outcomes.iter().all(|outcome| outcome.result == LIGHT_HIT));
            assert_eq!(
                outcomes
                    .iter()
                    .map(|outcome| outcome.target)
                    .collect::<Vec<_>>(),
                expected
                    .iter()
                    .map(|&(_, id)| StrikeTarget::Monster(id))
                    .collect::<Vec<_>>(),
                "facing {facing:?}"
            );
        }
    }

    #[test]
    fn empty_swings_and_targets_leaving_before_the_active_window_miss() {
        let (mut game, x, z) = strike_arena();
        // Behind the player and beyond its own reach, keeping the encounter active.
        place_monster(&mut game, 1, x - 250, z);
        assert!(strike(&mut game, 1, ArpgCommand::PrimaryAttack { aim: None }).is_empty());

        game.monsters[0].position.x = (x + 100) as f32;
        game.apply_command(
            PlayerCommand::new(1, 2, ArpgCommand::PrimaryAttack { aim: None }).unwrap(),
        )
        .unwrap();
        game.advance_tick().unwrap();
        game.monsters[0].position.x = (x + 300) as f32;
        while game.players.get(&1).unwrap().action.is_some() {
            game.advance_tick().unwrap();
            assert!(game.strike_outcomes().is_empty());
        }
        assert_eq!(monster_health(&game, 1), 100);

        game.monsters[0].position.x = (x + 100) as f32;
        assert_eq!(
            strike(&mut game, 3, ArpgCommand::PrimaryAttack { aim: None }).len(),
            1
        );
    }

    #[test]
    fn targets_moving_into_the_volume_before_the_active_window_are_hit() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 400, z);
        game.apply_command(
            PlayerCommand::new(1, 1, ArpgCommand::PrimaryAttack { aim: None }).unwrap(),
        )
        .unwrap();
        game.advance_tick().unwrap();
        game.monsters[0].position.x = (x + 100) as f32;
        let mut outcomes = Vec::new();
        while game.players.get(&1).unwrap().action.is_some() {
            game.advance_tick().unwrap();
            outcomes.extend_from_slice(game.strike_outcomes());
        }
        assert_eq!(targets(&outcomes), [(StrikeTarget::Monster(1), LIGHT_HIT)]);
    }

    #[test]
    fn dead_and_dormant_targets_are_not_eligible() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 100, z);
        game.monsters[0].health = 0;
        place_monster(&mut game, 2, x + 120, z);
        game.monsters[1].room_id = STRIKE_ROOM + 1;
        assert!(strike(&mut game, 1, ArpgCommand::PrimaryAttack { aim: None }).is_empty());
        assert_eq!(monster_health(&game, 2), 100);
    }

    #[test]
    fn each_strike_hits_a_target_once_and_repeated_strikes_have_distinct_ids() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 100, z);
        let first = strike(&mut game, 1, ArpgCommand::PrimaryAttack { aim: None });
        let second = strike(&mut game, 2, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_ne!(first[0].strike, second[0].strike);
        assert_eq!(monster_health(&game, 1), 100 - 2 * BASE_ATTACK_DAMAGE);
    }

    #[test]
    fn walls_obstruct_strikes_without_consuming_the_target_limit() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 120, z);
        place_monster(&mut game, 2, x + 130, z + 60);
        place_blocker(
            &mut game,
            0,
            x + 70,
            z,
            Vec3i::new(10, WALL_HALF_HEIGHT, 15),
        );

        let outcomes = strike(&mut game, 1, ArpgCommand::SecondaryAttack { aim: None });
        assert_eq!(outcomes.len(), 2);
        assert_eq!(
            (outcomes[0].target, outcomes[0].result),
            (StrikeTarget::Monster(1), StrikeResult::Obstructed)
        );
        assert_eq!(outcomes[1].target, StrikeTarget::Monster(2));
        assert!(matches!(outcomes[1].result, StrikeResult::Hit { .. }));
        assert_eq!(monster_health(&game, 1), 100);
    }

    #[test]
    fn high_id_player_bodies_never_obstruct_strikes() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        let (x, z) = game
            .rooms
            .iter()
            .find(|room| room.id == STRIKE_ROOM)
            .unwrap()
            .center();
        // Player 10_000's body id lands in the fixed-collider id range.
        game.player_spawns[0] = Vec3i::new(x, PLAYER_Y, z);
        game.add_player(10_000).unwrap();
        game.reconcile_encounters().unwrap();
        game.monsters.clear();
        place_monster(&mut game, 1, x + 100, z);
        game.apply_command(
            PlayerCommand::new(10_000, 1, ArpgCommand::PrimaryAttack { aim: None }).unwrap(),
        )
        .unwrap();
        let mut outcomes = Vec::new();
        while game.players.get(&10_000).unwrap().action.is_some() {
            game.advance_tick().unwrap();
            outcomes.extend_from_slice(game.strike_outcomes());
        }
        assert_eq!(targets(&outcomes), [(StrikeTarget::Monster(1), LIGHT_HIT)]);
    }

    #[test]
    fn closed_doors_obstruct_strikes() {
        let (mut game, _, _) = strike_arena();
        let door = game
            .doors
            .iter()
            .find(|door| door.locked && (door.room_a == STRIKE_ROOM || door.room_b == STRIKE_ROOM))
            .unwrap()
            .clone();
        let (room_x, room_z) = game
            .rooms
            .iter()
            .find(|room| room.id == STRIKE_ROOM)
            .unwrap()
            .center();
        let [door_x, _, door_z] = door.position;
        // Step through the door along its thin axis: player inside the room, target beyond it.
        let (step_x, step_z) = if door.half_extents[0] < door.half_extents[2] {
            ((room_x - door_x).signum(), 0)
        } else {
            (0, (room_z - door_z).signum())
        };
        let player = Vec3i::new(door_x + step_x * 80, PLAYER_Y, door_z + step_z * 80);
        game.put_player(1, player);
        let state = game.players.get_mut(&1).unwrap();
        state.facing_x = i8::try_from(-step_x).unwrap();
        state.facing_z = i8::try_from(-step_z).unwrap();
        place_monster(&mut game, 1, door_x - step_x * 80, door_z - step_z * 80);

        let outcomes = strike(&mut game, 1, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(
            targets(&outcomes),
            [(StrikeTarget::Monster(1), StrikeResult::Obstructed)]
        );
    }

    #[test]
    fn monster_strikes_share_the_result_path_and_respect_walls() {
        // A wall already between them: the monster does not wind up into it.
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 120, z);
        place_blocker(
            &mut game,
            0,
            x + 70,
            z,
            Vec3i::new(10, WALL_HALF_HEIGHT, 15),
        );
        game.advance_tick().unwrap();
        assert!(game.monsters[0].action.is_none());

        // A wall that appears during the wind-up obstructs the strike.
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 120, z);
        game.advance_tick().unwrap();
        assert!(game.monsters[0].action.is_some());
        let striking_from = monster_position(&game, 1);
        place_blocker(
            &mut game,
            0,
            (x + striking_from.x as i32) / 2,
            z,
            Vec3i::new(10, WALL_HALF_HEIGHT, 15),
        );
        let mut outcomes = Vec::new();
        for _ in 0..MONSTER_ATTACK_WINDUP_TICKS + 2 {
            game.advance_tick().unwrap();
            outcomes.extend_from_slice(game.strike_outcomes());
        }
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].strike.source, StrikeSource::Monster(1));
        assert_eq!(outcomes[0].definition, "monster.claw");
        assert_eq!(
            (outcomes[0].target, outcomes[0].result),
            (StrikeTarget::Player(1), StrikeResult::Obstructed)
        );
        assert_eq!(game.players.get(&1).unwrap().health, BASE_MAX_HEALTH);
    }

    fn hold_guard(game: &mut ArpgGame, sequence: u32, raised: bool) {
        game.apply_command(
            PlayerCommand::new(1, sequence, ArpgCommand::SetGuard { raised }).unwrap(),
        )
        .unwrap();
    }

    fn raise_guard_fully(game: &mut ArpgGame) {
        let player = game.players.get_mut(&1).unwrap();
        player.guard.held = true;
        player.guard.stance = Some(GuardStance {
            phase: GuardPhase::Raised,
            ticks_remaining: 0,
        });
    }

    fn claw(game: &mut ArpgGame, monster_id: u32) -> StrikeResult {
        game.strike_outcomes.clear();
        game.resolve_monster_strike(monster_id, 1, monster_claw())
            .unwrap();
        assert_eq!(game.strike_outcomes.len(), 1);
        game.strike_outcomes[0].result
    }

    const CLAW_HIT: StrikeResult = StrikeResult::Hit {
        damage: MONSTER_ATTACK_DAMAGE,
        defeated: false,
    };
    const CLAW_BLOCKED: StrikeResult = StrikeResult::Blocked {
        guard_damage: MONSTER_CLAW_GUARD_COST,
    };

    #[test]
    fn guard_rises_over_authored_ticks_before_it_protects() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 120, z);
        hold_guard(&mut game, 1, true);
        let mut phases = Vec::new();
        for _ in 0..6 {
            game.advance_tick().unwrap();
            phases.push(game.snapshot().unwrap().players[0].guard);
        }
        let raising = |ticks_remaining| {
            Some(GuardStance {
                phase: GuardPhase::Raising,
                ticks_remaining,
            })
        };
        let raised = Some(GuardStance {
            phase: GuardPhase::Raised,
            ticks_remaining: 0,
        });
        assert_eq!(
            phases,
            [
                raising(4),
                raising(3),
                raising(2),
                raising(1),
                raised,
                raised
            ]
        );

        let player = game.players.get_mut(&1).unwrap();
        player.guard.stance = raising(1);
        assert_eq!(
            claw(&mut game, 1),
            CLAW_HIT,
            "a rising shield does not protect"
        );
    }

    #[test]
    fn a_raised_shield_blocks_its_front_sector_only() {
        // (dx, dz) of the attacker relative to a defender facing +x, with the expected
        // result from the 60-degree rule 3·dx² ≥ dz², dx > 0.
        let fixtures = [
            ((120, 0), true),
            ((50, 86), true),
            ((50, -86), true),
            ((50, 87), false),
            ((0, 120), false),
            ((0, -120), false),
            ((-120, 0), false),
            ((-80, 80), false),
            ((0, 0), false),
        ];
        for ((dx, dz), blocked) in fixtures {
            let (mut game, x, z) = strike_arena();
            place_monster(&mut game, 1, x + dx, z + dz);
            raise_guard_fully(&mut game);
            let expected = if blocked { CLAW_BLOCKED } else { CLAW_HIT };
            assert_eq!(claw(&mut game, 1), expected, "attacker at ({dx}, {dz})");
            let player = game.players.get(&1).unwrap();
            if blocked {
                assert_eq!(player.health, BASE_MAX_HEALTH);
                assert_eq!(
                    player.guard.points,
                    MAX_GUARD_POINTS - MONSTER_CLAW_GUARD_COST
                );
                assert_eq!(
                    game.snapshot().unwrap().players[0].reaction,
                    Some(PlayerReactionSnapshot {
                        kind: PlayerReactionKind::Blocked,
                        ticks_remaining: GUARD_BLOCK_REACTION_TICKS,
                    })
                );
            } else {
                assert_eq!(player.health, BASE_MAX_HEALTH - MONSTER_ATTACK_DAMAGE);
                assert_eq!(
                    player.guard.stance, None,
                    "an unblocked hit lowers the shield"
                );
            }
        }
    }

    #[test]
    fn guard_breaks_when_the_cost_reaches_the_remaining_guard() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 120, z);
        raise_guard_fully(&mut game);
        game.players.get_mut(&1).unwrap().guard.points = MONSTER_CLAW_GUARD_COST + 1;
        assert_eq!(claw(&mut game, 1), CLAW_BLOCKED);
        assert_eq!(game.players.get(&1).unwrap().guard.points, 1);

        game.players.get_mut(&1).unwrap().guard.points = MONSTER_CLAW_GUARD_COST;
        assert_eq!(claw(&mut game, 1), StrikeResult::GuardBroken);
        let player = *game.players.get(&1).unwrap();
        assert_eq!(
            player.health, BASE_MAX_HEALTH,
            "a guard break absorbs that strike"
        );
        assert_eq!(player.guard.points, 0);
        assert_eq!(player.guard.stance, None);
        assert_eq!(player.guard.broken_ticks_remaining, GUARD_BREAK_TICKS);

        assert_eq!(
            claw(&mut game, 1),
            CLAW_HIT,
            "a broken guard does not block"
        );
        game.players.get_mut(&1).unwrap().hurt_ticks_remaining = 0;
        game.monsters.clear();
        place_monster(&mut game, 1, x - 600, z);
        for _ in 0..GUARD_BREAK_TICKS {
            assert_eq!(game.players.get(&1).unwrap().guard.stance, None);
            game.advance_tick().unwrap();
        }
        assert_eq!(
            game.players.get(&1).unwrap().guard.points,
            0,
            "no regen while broken"
        );
        game.advance_tick().unwrap();
        assert!(
            game.players.get(&1).unwrap().guard.stance.is_some(),
            "held guard rises again"
        );
    }

    #[test]
    fn same_tick_strikes_each_spend_guard_once() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 120, z);
        place_monster(&mut game, 2, x + 120, z + 40);
        raise_guard_fully(&mut game);
        game.players.get_mut(&1).unwrap().guard.points = 50;
        game.strike_outcomes.clear();
        game.resolve_monster_strike(1, 1, monster_claw()).unwrap();
        game.resolve_monster_strike(2, 1, monster_claw()).unwrap();
        assert_eq!(
            targets(&game.strike_outcomes),
            [
                (StrikeTarget::Player(1), CLAW_BLOCKED),
                (StrikeTarget::Player(1), StrikeResult::GuardBroken),
            ]
        );
        assert_eq!(game.players.get(&1).unwrap().health, BASE_MAX_HEALTH);
    }

    #[test]
    fn unblockable_strikes_ignore_a_raised_shield() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 120, z);
        raise_guard_fully(&mut game);
        let unblockable = StrikeDefinition {
            blockable: false,
            ..monster_claw()
        };
        game.resolve_monster_strike(1, 1, unblockable).unwrap();
        assert_eq!(game.strike_outcomes[0].result, CLAW_HIT);
        assert_eq!(game.players.get(&1).unwrap().guard.points, MAX_GUARD_POINTS);
    }

    #[test]
    fn monster_blocked_through_the_tick_loop_and_an_attacker_crossing_behind_hits() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 120, z);
        hold_guard(&mut game, 1, true);
        let mut outcomes = Vec::new();
        for _ in 0..MONSTER_ATTACK_WINDUP_TICKS + 2 {
            game.advance_tick().unwrap();
            outcomes.extend_from_slice(game.strike_outcomes());
        }
        assert_eq!(
            targets(&outcomes),
            [(StrikeTarget::Player(1), CLAW_BLOCKED)]
        );
        assert_eq!(game.players.get(&1).unwrap().health, BASE_MAX_HEALTH);

        // Next attack: wind up in front, then circle behind before contact.
        outcomes.clear();
        while game.monsters[0].action.map(|action| action.phase) != Some(ActionPhase::Windup) {
            game.advance_tick().unwrap();
        }
        game.monsters[0].position.x = (x - 120) as f32;
        while game.monsters[0].action.map(|action| action.phase) == Some(ActionPhase::Windup) {
            game.advance_tick().unwrap();
            outcomes.extend_from_slice(game.strike_outcomes());
        }
        assert_eq!(targets(&outcomes), [(StrikeTarget::Player(1), CLAW_HIT)]);
    }

    #[test]
    fn attacking_lowers_the_guard_and_a_held_guard_rises_again_afterwards() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x - 600, z);
        hold_guard(&mut game, 1, true);
        for _ in 0..6 {
            game.advance_tick().unwrap();
        }
        assert!(game.players.get(&1).unwrap().guard.is_raised());
        game.apply_command(
            PlayerCommand::new(1, 2, ArpgCommand::PrimaryAttack { aim: None }).unwrap(),
        )
        .unwrap();
        assert_eq!(game.players.get(&1).unwrap().guard.stance, None);
        while game.players.get(&1).unwrap().action.is_some() {
            game.advance_tick().unwrap();
            assert_eq!(game.players.get(&1).unwrap().guard.stance, None);
        }
        game.advance_tick().unwrap();
        assert_eq!(
            game.players
                .get(&1)
                .unwrap()
                .guard
                .stance
                .map(|stance| stance.phase),
            Some(GuardPhase::Raising)
        );

        hold_guard(&mut game, 3, false);
        assert_eq!(game.players.get(&1).unwrap().guard.stance, None);
        game.advance_tick().unwrap();
        assert_eq!(game.players.get(&1).unwrap().guard.stance, None);
    }

    #[test]
    fn death_clears_held_guard() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x - 600, z);
        raise_guard_fully(&mut game);
        game.players.get_mut(&1).unwrap().health = 0;
        game.advance_tick().unwrap();
        let guard = game.players.get(&1).unwrap().guard;
        assert!(!guard.held);
        assert_eq!(guard.stance, None);
    }

    #[test]
    fn guard_state_survives_save_and_restore_and_invalid_guard_is_rejected() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        game.add_player(1).unwrap();
        hold_guard(&mut game, 1, true);
        game.advance_tick().unwrap();
        game.advance_tick().unwrap();
        game.players.get_mut(&1).unwrap().guard.points = 42;
        let save = game.save_state().unwrap();
        let mut restored = ArpgGame::from_save_state(save.clone()).unwrap();
        assert_eq!(restored.snapshot().unwrap(), game.snapshot().unwrap());
        for _ in 0..4 {
            game.advance_tick().unwrap();
            restored.advance_tick().unwrap();
        }
        assert_eq!(restored.snapshot().unwrap(), game.snapshot().unwrap());

        for corrupt in [
            GuardState {
                points: MAX_GUARD_POINTS + 1,
                ..save.players[0].guard
            },
            GuardState {
                held: false,
                ..save.players[0].guard
            },
            GuardState {
                stance: Some(GuardStance {
                    phase: GuardPhase::Raised,
                    ticks_remaining: 3,
                }),
                ..save.players[0].guard
            },
        ] {
            let mut tampered = save.clone();
            tampered.players[0].guard = corrupt;
            assert!(
                ArpgGame::from_save_state(tampered)
                    .unwrap_err()
                    .message()
                    .contains("guard")
            );
        }
    }

    /// Arena with a guarding player and one monster in front whose claw was just blocked
    /// during the tick before `game.tick`.
    fn blocked_once(monster_offset: i32) -> (ArpgGame, i32, i32) {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + monster_offset, z);
        raise_guard_fully(&mut game);
        assert_eq!(claw(&mut game, 1), CLAW_BLOCKED);
        game.tick += 1;
        (game, x, z)
    }

    fn command(game: &mut ArpgGame, command: ArpgCommand) {
        let sequence = game.last_sequences.get(&1).copied().unwrap_or_default() + 1;
        game.apply_command(PlayerCommand::new(1, sequence, command).unwrap())
            .unwrap();
    }

    fn action_kind(game: &ArpgGame) -> Option<ActionKind> {
        game.players
            .get(&1)
            .unwrap()
            .action
            .map(|action| action.kind)
    }

    #[test]
    fn a_successful_block_grants_one_bounded_counter_opportunity() {
        let (game, _, _) = blocked_once(120);
        let counter = game.players.get(&1).unwrap().counter.unwrap();
        assert_eq!(counter.usable_from_tick, game.tick);
        assert_eq!(counter.expires_at_tick, game.tick + COUNTER_WINDOW_TICKS);
        assert_eq!(counter.blocked_monster_id, 1);
        assert_eq!(game.snapshot().unwrap().players[0].counter, Some(counter));
    }

    #[test]
    fn raising_guard_or_being_hit_grants_nothing() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x - 120, z);
        hold_guard(&mut game, 1, true);
        for _ in 0..6 {
            game.advance_tick().unwrap();
        }
        assert_eq!(game.players.get(&1).unwrap().counter, None);
        assert_eq!(claw(&mut game, 1), CLAW_HIT);
        assert_eq!(game.players.get(&1).unwrap().counter, None);
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(
            action_kind(&game),
            Some(ActionKind::PrimaryAttack),
            "no counter benefit"
        );
    }

    #[test]
    fn a_timed_primary_attack_executes_the_counter_and_consumes_it() {
        let (mut game, x, _) = blocked_once(120);
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(action_kind(&game), Some(ActionKind::Counter));
        assert_eq!(game.players.get(&1).unwrap().counter, None);
        let mut outcomes = Vec::new();
        while game.players.get(&1).unwrap().action.is_some() {
            game.advance_tick().unwrap();
            outcomes.extend(
                game.strike_outcomes()
                    .iter()
                    .filter(|outcome| outcome.strike.source == StrikeSource::Player(1))
                    .cloned(),
            );
        }
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].definition, "sword.counterSlash");
        assert_eq!(
            outcomes[0].result,
            StrikeResult::Hit {
                damage: BASE_ATTACK_DAMAGE * COUNTER_DAMAGE_NUMERATOR,
                defeated: false
            }
        );
        let _ = x;

        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(
            action_kind(&game),
            Some(ActionKind::PrimaryAttack),
            "spent once only"
        );
    }

    #[test]
    fn counter_eligibility_is_the_half_open_window() {
        for (delay, expect_counter) in [
            (0, true),
            (COUNTER_WINDOW_TICKS - 1, true),
            (COUNTER_WINDOW_TICKS, false),
        ] {
            let (mut game, x, z) = blocked_once(120);
            // Keep the monster out of the way while time passes.
            game.monsters[0].position = Vec3i::new(x - 600, PLAYER_Y, z).into();
            for _ in 0..delay {
                game.advance_tick().unwrap();
            }
            command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
            let expected = if expect_counter {
                ActionKind::Counter
            } else {
                ActionKind::PrimaryAttack
            };
            assert_eq!(action_kind(&game), Some(expected), "after {delay} ticks");
            if !expect_counter {
                assert_eq!(game.players.get(&1).unwrap().counter, None);
            }
        }
    }

    #[test]
    fn a_denied_start_keeps_the_opportunity_and_duplicates_cannot_spend_twice() {
        let (mut game, _, _) = blocked_once(120);
        let state = game.players.get_mut(&1).unwrap();
        state.action = Some(ActionState {
            kind: ActionKind::Interact,
            phase: ActionPhase::Recovery,
            ticks_remaining: 1,
            facing_x: 1,
            facing_z: 0,
            connected: false,
            buffered: None,
            charge: 0,
            aim: None,
        });
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(action_kind(&game), Some(ActionKind::Interact));
        assert!(game.players.get(&1).unwrap().counter.is_some());

        game.players.get_mut(&1).unwrap().action = None;
        let sequence = game.last_sequences[&1] + 1;
        game.apply_command(
            PlayerCommand::new(1, sequence, ArpgCommand::PrimaryAttack { aim: None }).unwrap(),
        )
        .unwrap();
        assert_eq!(action_kind(&game), Some(ActionKind::Counter));
        assert!(
            game.apply_command(
                PlayerCommand::new(1, sequence, ArpgCommand::PrimaryAttack { aim: None }).unwrap()
            )
            .is_err(),
            "a duplicated delivery is stale"
        );
    }

    #[test]
    fn the_counter_can_miss_a_retreating_attacker() {
        let (mut game, x, z) = blocked_once(120);
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        game.monsters[0].position = Vec3i::new(x + 400, PLAYER_Y, z).into();
        while game.players.get(&1).unwrap().action.is_some() {
            game.advance_tick().unwrap();
            assert!(
                game.strike_outcomes()
                    .iter()
                    .all(|outcome| outcome.strike.source != StrikeSource::Player(1))
            );
        }
        assert_eq!(monster_health(&game, 1), 100);
    }

    #[test]
    fn guard_break_hurt_and_death_invalidate_and_new_blocks_replace() {
        let (mut game, x, z) = blocked_once(120);
        place_monster(&mut game, 2, x + 100, z + 40);
        assert_eq!(claw(&mut game, 2), CLAW_BLOCKED);
        assert_eq!(
            game.players
                .get(&1)
                .unwrap()
                .counter
                .unwrap()
                .blocked_monster_id,
            2,
            "a newer genuine block replaces the opportunity"
        );
        game.players.get_mut(&1).unwrap().guard.points = 1;
        assert_eq!(claw(&mut game, 1), StrikeResult::GuardBroken);
        assert_eq!(game.players.get(&1).unwrap().counter, None);

        let (mut game, x, z) = blocked_once(120);
        place_monster(&mut game, 2, x - 120, z);
        assert_eq!(claw(&mut game, 2), CLAW_HIT);
        assert_eq!(game.players.get(&1).unwrap().counter, None);

        let (mut game, _, _) = blocked_once(120);
        game.players.get_mut(&1).unwrap().health = 0;
        game.advance_tick().unwrap();
        assert_eq!(game.players.get(&1).unwrap().counter, None);
    }

    #[test]
    fn a_save_right_after_a_counter_hit_restores() {
        // Generated monsters only: saves must match the generated dungeon.
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        let (x, z) = game
            .rooms
            .iter()
            .find(|room| room.id == STRIKE_ROOM)
            .unwrap()
            .center();
        game.player_spawns[0] = Vec3i::new(x, PLAYER_Y, z);
        game.add_player(1).unwrap();
        game.reconcile_encounters().unwrap();
        let index = game
            .monsters
            .iter()
            .position(|monster| monster.room_id == STRIKE_ROOM)
            .unwrap();
        game.monsters[index].position = Vec3i::new(x + 120, PLAYER_Y, z).into();
        let monster_id = game.monsters[index].id;
        raise_guard_fully(&mut game);
        assert_eq!(claw(&mut game, monster_id), CLAW_BLOCKED);
        game.tick += 1;
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(action_kind(&game), Some(ActionKind::Counter));
        while game.monsters[index].stagger_ticks_remaining == 0 {
            game.advance_tick().unwrap();
        }
        assert_eq!(
            game.monsters[0].stagger_ticks_remaining,
            COUNTER_STAGGER_TICKS
        );
        let mut restored = ArpgGame::from_save_state(game.save_state().unwrap()).unwrap();
        // Strike events belong to the tick that produced them and are not saved.
        game.advance_tick().unwrap();
        restored.advance_tick().unwrap();
        assert_eq!(restored.snapshot().unwrap(), game.snapshot().unwrap());
    }

    #[test]
    fn a_pending_counter_survives_save_and_restore_without_extra_time() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        game.add_player(1).unwrap();
        game.players.get_mut(&1).unwrap().counter =
            Some(CounterOpportunity::grant(COUNTER_WINDOW_TICKS, 1, 0));
        game.tick = 5;
        let save = game.save_state().unwrap();
        let mut restored = ArpgGame::from_save_state(save.clone()).unwrap();
        assert_eq!(restored.snapshot().unwrap(), game.snapshot().unwrap());
        for _ in 0..COUNTER_WINDOW_TICKS {
            game.advance_tick().unwrap();
            restored.advance_tick().unwrap();
            assert_eq!(restored.snapshot().unwrap(), game.snapshot().unwrap());
        }
        assert_eq!(restored.players.get(&1).unwrap().counter, None);

        for corrupt in [
            CounterOpportunity {
                expires_at_tick: 100,
                ..CounterOpportunity::grant(COUNTER_WINDOW_TICKS, 1, 0)
            },
            CounterOpportunity::grant(COUNTER_WINDOW_TICKS, 1, 5),
            CounterOpportunity {
                usable_from_tick: 0,
                expires_at_tick: 5,
                ..CounterOpportunity::grant(COUNTER_WINDOW_TICKS, 1, 0)
            },
        ] {
            let mut tampered = save.clone();
            tampered.players[0].counter = Some(corrupt);
            assert!(
                ArpgGame::from_save_state(tampered)
                    .unwrap_err()
                    .message()
                    .contains("counter")
            );
        }
    }

    fn current_action(game: &ArpgGame) -> Option<ActionState> {
        game.players.get(&1).unwrap().action
    }

    /// Advances until the player's action is `kind` in recovery with `elapsed` recovery ticks.
    fn advance_to_recovery(game: &mut ArpgGame, kind: ActionKind, elapsed: u8) {
        for _ in 0..200 {
            if let Some(action) = current_action(game)
                && action.kind == kind
                && action.phase == ActionPhase::Recovery
                && kind.recovery_ticks(content()) - action.ticks_remaining == elapsed
            {
                return;
            }
            game.advance_tick().unwrap();
        }
        panic!("never reached {kind:?} recovery tick {elapsed}");
    }

    fn advance_to_phase(game: &mut ArpgGame, kind: ActionKind, phase: ActionPhase) {
        for _ in 0..200 {
            if current_action(game)
                .is_some_and(|action| action.kind == kind && action.phase == phase)
            {
                return;
            }
            game.advance_tick().unwrap();
        }
        panic!("never reached {kind:?} {phase:?}");
    }

    fn combo_arena(target: bool) -> ArpgGame {
        let (mut game, x, z) = strike_arena();
        // A sturdy target ahead, or one far behind that keeps the encounter active.
        place_monster(&mut game, 1, if target { x + 120 } else { x - 600 }, z);
        game.monsters[0].health = 1_000;
        game
    }

    fn player_definitions(game: &mut ArpgGame) -> Vec<DefinitionId> {
        let mut definitions = Vec::new();
        while current_action(game).is_some() {
            game.advance_tick().unwrap();
            definitions.extend(
                game.strike_outcomes()
                    .iter()
                    .filter(|outcome| outcome.strike.source == StrikeSource::Player(1))
                    .map(|outcome| outcome.definition.clone()),
            );
        }
        definitions
    }

    #[test]
    fn light_light_light_chains_into_the_finisher() {
        let mut game = combo_arena(true);
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        advance_to_recovery(&mut game, ActionKind::PrimaryAttack, 2);
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(action_kind(&game), Some(ActionKind::LightFollowUp));
        advance_to_recovery(&mut game, ActionKind::LightFollowUp, 3);
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(action_kind(&game), Some(ActionKind::LightFinisher));
        assert_eq!(player_definitions(&mut game), ["sword.lightFinisher"]);
        // The chain ends with the finisher; the next press starts over.
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(action_kind(&game), Some(ActionKind::PrimaryAttack));
    }

    #[test]
    fn light_light_heavy_finisher_requires_the_second_strike_to_connect() {
        let mut game = combo_arena(true);
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        advance_to_recovery(&mut game, ActionKind::PrimaryAttack, 2);
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        advance_to_recovery(&mut game, ActionKind::LightFollowUp, 2);
        assert!(current_action(&game).unwrap().connected);
        command(&mut game, ArpgCommand::SecondaryAttack { aim: None });
        assert_eq!(action_kind(&game), Some(ActionKind::HeavyFinisher));
        assert_eq!(player_definitions(&mut game), ["sword.heavyFinisher"]);

        let mut whiff = combo_arena(false);
        command(&mut whiff, ArpgCommand::PrimaryAttack { aim: None });
        advance_to_recovery(&mut whiff, ActionKind::PrimaryAttack, 2);
        command(&mut whiff, ArpgCommand::PrimaryAttack { aim: None });
        advance_to_recovery(&mut whiff, ActionKind::LightFollowUp, 2);
        assert!(!current_action(&whiff).unwrap().connected);
        command(&mut whiff, ArpgCommand::SecondaryAttack { aim: None });
        assert_eq!(
            action_kind(&whiff),
            Some(ActionKind::LightFollowUp),
            "no hit, no heavy branch"
        );
        command(&mut whiff, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(
            action_kind(&whiff),
            Some(ActionKind::LightFinisher),
            "whiffs may continue light"
        );
    }

    #[test]
    fn transition_interval_boundaries_are_half_open_and_early_inputs_buffer() {
        // Pressing at recovery tick `elapsed` of the opening light strike (recovery 8).
        for (elapsed, expected) in [
            (0, ActionKind::PrimaryAttack),
            (1, ActionKind::PrimaryAttack),
            (2, ActionKind::LightFollowUp),
            (7, ActionKind::LightFollowUp),
        ] {
            let mut game = combo_arena(false);
            command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
            advance_to_recovery(&mut game, ActionKind::PrimaryAttack, elapsed);
            command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
            assert_eq!(
                action_kind(&game),
                Some(expected),
                "pressed at recovery {elapsed}"
            );
            if elapsed < 2 {
                assert_eq!(
                    current_action(&game).unwrap().buffered,
                    Some(ComboInput::Light)
                );
                advance_to_recovery(&mut game, ActionKind::PrimaryAttack, 1);
                game.advance_tick().unwrap();
                assert_eq!(
                    action_kind(&game),
                    Some(ActionKind::LightFollowUp),
                    "a buffered input commits as the interval opens"
                );
                assert_eq!(current_action(&game).unwrap().buffered, None);
            }
        }

        // After the interval the opener has ended and a press starts over.
        let mut game = combo_arena(false);
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        advance_to_recovery(&mut game, ActionKind::PrimaryAttack, 7);
        game.advance_tick().unwrap();
        assert_eq!(action_kind(&game), None);
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(action_kind(&game), Some(ActionKind::PrimaryAttack));
    }

    #[test]
    fn wind_up_presses_are_ignored_and_spam_keeps_one_intent() {
        let mut game = combo_arena(false);
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(
            current_action(&game).unwrap().buffered,
            None,
            "wind-up press ignored"
        );

        advance_to_phase(&mut game, ActionKind::PrimaryAttack, ActionPhase::Active);
        for _ in 0..5 {
            command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
            command(&mut game, ArpgCommand::SecondaryAttack { aim: None });
        }
        assert_eq!(
            current_action(&game).unwrap().buffered,
            Some(ComboInput::Light)
        );
        advance_to_phase(&mut game, ActionKind::LightFollowUp, ActionPhase::Windup);
        assert_eq!(current_action(&game).unwrap().buffered, None);
        // The follow-up then runs its full course with no hidden queued successor.
        while current_action(&game).is_some() {
            assert_ne!(action_kind(&game), Some(ActionKind::LightFinisher));
            game.advance_tick().unwrap();
        }
    }

    #[test]
    fn an_unearned_buffered_heavy_branch_is_dropped_at_the_interval() {
        let mut game = combo_arena(false);
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        advance_to_recovery(&mut game, ActionKind::PrimaryAttack, 2);
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        advance_to_phase(&mut game, ActionKind::LightFollowUp, ActionPhase::Active);
        command(&mut game, ArpgCommand::SecondaryAttack { aim: None });
        assert_eq!(
            current_action(&game).unwrap().buffered,
            Some(ComboInput::Heavy)
        );
        advance_to_recovery(&mut game, ActionKind::LightFollowUp, 2);
        let action = current_action(&game).unwrap();
        assert_eq!(
            (action.kind, action.buffered),
            (ActionKind::LightFollowUp, None)
        );
    }

    #[test]
    fn same_tick_inputs_resolve_in_sequence_order() {
        for (first, second, expected) in [
            (
                ArpgCommand::SecondaryAttack { aim: None },
                ArpgCommand::PrimaryAttack { aim: None },
                ActionKind::HeavyFinisher,
            ),
            (
                ArpgCommand::PrimaryAttack { aim: None },
                ArpgCommand::SecondaryAttack { aim: None },
                ActionKind::LightFinisher,
            ),
        ] {
            let mut game = combo_arena(true);
            command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
            advance_to_recovery(&mut game, ActionKind::PrimaryAttack, 2);
            command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
            advance_to_recovery(&mut game, ActionKind::LightFollowUp, 2);
            command(&mut game, first);
            command(&mut game, second);
            let action = current_action(&game).unwrap();
            assert_eq!((action.kind, action.phase), (expected, ActionPhase::Windup));
            assert_eq!(
                action.buffered, None,
                "the second input hits the successor's wind-up"
            );
        }
    }

    #[test]
    fn heavy_openers_and_interactions_have_no_light_combo() {
        let mut game = combo_arena(false);
        command(&mut game, ArpgCommand::SecondaryAttack { aim: None });
        advance_to_recovery(&mut game, ActionKind::SecondaryAttack, 3);
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(action_kind(&game), Some(ActionKind::SecondaryAttack));
    }

    #[test]
    fn interruption_and_death_clear_the_chain() {
        let mut game = combo_arena(false);
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        advance_to_phase(&mut game, ActionKind::PrimaryAttack, ActionPhase::Active);
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        place_monster(&mut game, 2, 0, 0);
        let (x, z) = {
            let position = Vec3i::rounded(game.players[&1].position);
            (position.x, position.z)
        };
        game.monsters[1].position = Vec3i::new(x - 100, PLAYER_Y, z).into();
        game.monsters[1].room_id = STRIKE_ROOM;
        assert_eq!(claw(&mut game, 2), CLAW_HIT);
        assert_eq!(current_action(&game).map(|action| action.kind), None);
        game.advance_tick().unwrap();
        game.advance_tick().unwrap();
        assert_eq!(
            action_kind(&game),
            None,
            "the buffered successor died with the action"
        );
    }

    #[test]
    fn a_counter_continues_into_the_declared_follow_up() {
        let (mut game, _, _) = blocked_once(120);
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(action_kind(&game), Some(ActionKind::Counter));
        advance_to_recovery(&mut game, ActionKind::Counter, 2);
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(action_kind(&game), Some(ActionKind::LightFollowUp));
    }

    #[test]
    fn saving_mid_chain_with_a_buffered_input_continues_identically() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        game.add_player(1).unwrap();
        game.apply_command(
            PlayerCommand::new(1, 1, ArpgCommand::PrimaryAttack { aim: None }).unwrap(),
        )
        .unwrap();
        advance_to_phase(&mut game, ActionKind::PrimaryAttack, ActionPhase::Active);
        game.apply_command(
            PlayerCommand::new(1, 2, ArpgCommand::PrimaryAttack { aim: None }).unwrap(),
        )
        .unwrap();
        assert!(current_action(&game).unwrap().buffered.is_some());
        let save = game.save_state().unwrap();
        let mut restored = ArpgGame::from_save_state(save.clone()).unwrap();
        for _ in 0..30 {
            game.advance_tick().unwrap();
            restored.advance_tick().unwrap();
            assert_eq!(restored.snapshot().unwrap(), game.snapshot().unwrap());
        }

        let mut tampered = save.clone();
        let action = tampered.players[0].action.as_mut().unwrap();
        action.buffered = Some(ComboInput::Heavy);
        assert!(
            ArpgGame::from_save_state(tampered)
                .unwrap_err()
                .message()
                .contains("combo")
        );
        let mut tampered = save;
        let action = tampered.players[0].action.as_mut().unwrap();
        action.phase = ActionPhase::Windup;
        action.ticks_remaining = 1;
        action.buffered = None;
        action.connected = true;
        assert!(
            ArpgGame::from_save_state(tampered)
                .unwrap_err()
                .message()
                .contains("hit confirmation")
        );
    }

    fn bow_arena() -> (ArpgGame, i32, i32) {
        let (mut game, x, z) = strike_arena();
        // A dormant-room monster far away keeps nothing in range but the room active.
        place_monster(&mut game, 99, x - 900, z);
        command(
            &mut game,
            ArpgCommand::EquipWeapon {
                weapon: Weapon::Bow,
            },
        );
        (game, x, z)
    }

    fn draw_for(game: &mut ArpgGame, ticks: u8) {
        command(game, ArpgCommand::DrawBow);
        for _ in 0..ticks {
            game.advance_tick().unwrap();
        }
    }

    /// Releases and runs until the shot's arrows are gone, collecting arrow outcomes.
    fn release_and_fly(game: &mut ArpgGame) -> Vec<StrikeOutcome> {
        command(game, ArpgCommand::ReleaseBow { aim: None });
        let mut outcomes = Vec::new();
        for _ in 0..80 {
            game.advance_tick().unwrap();
            outcomes.extend(
                game.strike_outcomes()
                    .iter()
                    .filter(|outcome| outcome.definition == "bow.arrow")
                    .cloned(),
            );
            if game.arrows.is_empty() && current_action(game).is_none() {
                break;
            }
        }
        outcomes
    }

    #[test]
    fn a_full_draw_launches_exactly_one_committed_arrow() {
        let (mut game, x, z) = bow_arena();
        draw_for(&mut game, BOW_FULL_DRAW_TICKS + 5);
        assert_eq!(game.players[&1].draw_ticks, Some(BOW_FULL_DRAW_TICKS));
        command(&mut game, ArpgCommand::ReleaseBow { aim: None });
        command(&mut game, ArpgCommand::ReleaseBow { aim: None });
        assert_eq!(action_kind(&game), Some(ActionKind::Shoot));
        while game.arrows.is_empty() {
            game.advance_tick().unwrap();
        }
        assert_eq!(game.arrows.len(), 1);
        let arrow = game.arrows[0];
        assert_eq!(arrow.source, StrikeSource::Player(1));
        assert_eq!(arrow.damage, ARROW_FULL_DAMAGE);
        assert_eq!(whole(arrow.velocity), [ARROW_FULL_SPEED, 0, 0]);
        // Launched from the body centre during the action step, then moved by the same
        // tick's arrow step.
        assert_eq!(whole(arrow.position), [x + ARROW_FULL_SPEED, PLAYER_Y, z]);
        while current_action(&game).is_some() {
            game.advance_tick().unwrap();
        }
        assert_eq!(game.next_arrow_id, ARROW_ID_BASE + 1, "one shot, one arrow");
    }

    #[test]
    fn short_draws_cancels_and_sword_loadouts_never_shoot() {
        let (mut game, _, _) = bow_arena();
        draw_for(&mut game, BOW_MIN_DRAW_TICKS - 1);
        command(&mut game, ArpgCommand::ReleaseBow { aim: None });
        assert_eq!(action_kind(&game), None);

        draw_for(&mut game, BOW_FULL_DRAW_TICKS);
        command(&mut game, ArpgCommand::CancelBow);
        command(&mut game, ArpgCommand::ReleaseBow { aim: None });
        assert_eq!(action_kind(&game), None);

        draw_for(&mut game, BOW_MIN_DRAW_TICKS);
        let charge_ok = {
            command(&mut game, ArpgCommand::ReleaseBow { aim: None });
            current_action(&game).map(|action| action.charge)
        };
        assert_eq!(
            charge_ok,
            Some(BOW_MIN_DRAW_TICKS),
            "the minimum draw is inclusive"
        );

        let (mut sword, _, _) = strike_arena();
        command(&mut sword, ArpgCommand::DrawBow);
        assert_eq!(sword.players[&1].draw_ticks, None);
        command(&mut sword, ArpgCommand::ReleaseBow { aim: None });
        assert!(sword.arrows.is_empty() && current_action(&sword).is_none());
    }

    #[test]
    fn arrows_hit_targets_ahead_through_the_shared_outcome_path() {
        let (mut game, x, z) = bow_arena();
        place_monster(&mut game, 1, x + 600, z);
        draw_for(&mut game, BOW_FULL_DRAW_TICKS);
        let outcomes = release_and_fly(&mut game);
        assert_eq!(
            targets(&outcomes),
            [(
                StrikeTarget::Monster(1),
                StrikeResult::Hit {
                    damage: ARROW_FULL_DAMAGE,
                    defeated: false
                }
            )]
        );
        assert_eq!(monster_health(&game, 1), 100 - ARROW_FULL_DAMAGE);
        assert!(
            game.arrows.is_empty(),
            "a non-piercing arrow stops at its first contact"
        );
    }

    #[test]
    fn arrows_miss_targets_that_move_away_and_expire_after_their_lifetime() {
        let (mut game, x, z) = bow_arena();
        place_monster(&mut game, 1, x + 900, z);
        draw_for(&mut game, BOW_FULL_DRAW_TICKS);
        command(&mut game, ArpgCommand::ReleaseBow { aim: None });
        while game.arrows.is_empty() {
            game.advance_tick().unwrap();
        }
        game.monsters
            .iter_mut()
            .find(|monster| monster.id == 1)
            .unwrap()
            .position
            .z = (z + 300) as f32;
        while !game.arrows.is_empty() {
            game.advance_tick().unwrap();
            assert!(game.strike_outcomes().is_empty());
        }
        assert_eq!(monster_health(&game, 1), 100);

        // In open space an arrow despawns when its lifetime runs out.
        let (mut game, x, z) = bow_arena();
        game.arrows.push(ArrowState {
            id: game.next_arrow_id,
            source: StrikeSource::Player(1),
            launched_at_tick: game.tick,
            position: Vec3i::new(x, PLAYER_Y, z).into(),
            velocity: Vector::Z,
            damage: ARROW_MIN_DAMAGE,
            ticks_remaining: 3,
        });
        game.next_arrow_id += 1;
        for expected in [2, 1] {
            game.advance_tick().unwrap();
            assert_eq!(game.arrows[0].ticks_remaining, expected);
        }
        game.advance_tick().unwrap();
        assert!(game.arrows.is_empty());
    }

    #[test]
    fn fast_arrows_stop_at_thin_walls_and_walls_win_contact_ties() {
        let (mut game, x, z) = bow_arena();
        place_monster(&mut game, 1, x + 600, z);
        place_blocker(
            &mut game,
            0,
            x + 300,
            z,
            Vec3i::new(2, WALL_HALF_HEIGHT, 60),
        );
        draw_for(&mut game, BOW_FULL_DRAW_TICKS);
        assert!(release_and_fly(&mut game).is_empty());
        assert_eq!(monster_health(&game, 1), 100);

        // Wall face and hurt-box face both at x + 200: equal contact time, wall first.
        let (mut tie, x, z) = bow_arena();
        place_monster(&mut tie, 1, x + 200 + MONSTER_HURTBOX_HALF_EXTENTS.x, z);
        place_blocker(&mut tie, 0, x + 205, z, Vec3i::new(5, WALL_HALF_HEIGHT, 60));
        draw_for(&mut tie, BOW_FULL_DRAW_TICKS);
        assert!(release_and_fly(&mut tie).is_empty());
    }

    #[test]
    fn a_muzzle_against_a_wall_cannot_shoot_through_it() {
        let (mut game, x, z) = bow_arena();
        place_blocker(
            &mut game,
            0,
            x + PLAYER_HALF_EXTENTS.x + 3,
            z,
            Vec3i::new(2, WALL_HALF_HEIGHT, 60),
        );
        place_monster(&mut game, 1, x + 150, z);
        draw_for(&mut game, BOW_FULL_DRAW_TICKS);
        assert!(release_and_fly(&mut game).is_empty());
        assert_eq!(monster_health(&game, 1), 100);
    }

    #[test]
    fn diagonal_release_commits_a_normalised_direction() {
        let (mut game, _, _) = bow_arena();
        let state = game.players.get_mut(&1).unwrap();
        state.facing_x = 1;
        state.facing_z = -1;
        draw_for(&mut game, BOW_FULL_DRAW_TICKS);
        command(&mut game, ArpgCommand::ReleaseBow { aim: None });
        while game.arrows.is_empty() {
            game.advance_tick().unwrap();
        }
        let diagonal = ARROW_FULL_SPEED * ARROW_DIAGONAL_NUMERATOR / ARROW_DIAGONAL_DENOMINATOR;
        assert_eq!(whole(game.arrows[0].velocity), [diagonal, 0, -diagonal]);
    }

    #[test]
    fn hurt_death_and_weapon_switches_cancel_a_draw_without_shooting() {
        let (mut game, x, z) = bow_arena();
        draw_for(&mut game, BOW_FULL_DRAW_TICKS);
        place_monster(&mut game, 2, x - 100, z);
        assert_eq!(claw(&mut game, 2), CLAW_HIT);
        game.advance_tick().unwrap();
        assert_eq!(game.players[&1].draw_ticks, None);
        command(&mut game, ArpgCommand::ReleaseBow { aim: None });
        assert_eq!(action_kind(&game), None);

        let (mut game, _, _) = bow_arena();
        draw_for(&mut game, BOW_FULL_DRAW_TICKS);
        command(
            &mut game,
            ArpgCommand::EquipWeapon {
                weapon: Weapon::SwordAndShield,
            },
        );
        command(&mut game, ArpgCommand::ReleaseBow { aim: None });
        assert_eq!(game.players[&1].draw_ticks, None);
        assert_eq!(action_kind(&game), None);

        let (mut game, _, _) = bow_arena();
        draw_for(&mut game, BOW_FULL_DRAW_TICKS);
        game.players.get_mut(&1).unwrap().health = 0;
        game.advance_tick().unwrap();
        assert_eq!(game.players[&1].draw_ticks, None);
    }

    #[test]
    fn restored_arrows_must_stay_in_launch_order() {
        let (mut game, _, _) = bow_arena();
        game.tick = 50;
        let mut shot = ArpgGame::new_action(content(), ActionKind::Shoot, &game.players[&1], None);
        shot.charge = BOW_FULL_DRAW_TICKS;
        game.players.get_mut(&1).unwrap().action = Some(shot);
        game.launch_arrow(1, 1, 0, None).unwrap();
        game.launch_arrow(1, -1, 0, None).unwrap();
        for arrow in &mut game.arrows {
            arrow.ticks_remaining -= 1;
        }
        game.tick += 1;
        game.players.get_mut(&1).unwrap().action = None;
        // Saves must carry the generated monster set, not the arena fixture's.
        game.monsters = ArpgGame::new_with_seed(42).unwrap().monsters;
        let save = game.save_state().unwrap();
        assert!(ArpgGame::from_save_state(save.clone()).is_ok());
        let mut reordered = save;
        reordered.arrows.reverse();
        assert!(
            ArpgGame::from_save_state(reordered)
                .unwrap_err()
                .message()
                .contains("arrow")
        );
    }

    #[test]
    fn the_minimum_and_full_draws_span_the_declared_arrow_range() {
        assert_eq!(
            ArpgGame::arrow_launch(&content().bow, BOW_MIN_DRAW_TICKS),
            (ARROW_MIN_SPEED, ARROW_MIN_DAMAGE)
        );
        assert_eq!(
            ArpgGame::arrow_launch(&content().bow, BOW_FULL_DRAW_TICKS),
            (ARROW_FULL_SPEED, ARROW_FULL_DAMAGE)
        );
        // Halfway through the accepted draw (8 + 11 = 19 of 30 ticks).
        assert_eq!(ArpgGame::arrow_launch(&content().bow, 19), (45, 25));
    }

    #[test]
    fn a_release_after_a_same_tick_hit_does_not_shoot() {
        let (mut game, x, z) = bow_arena();
        draw_for(&mut game, BOW_FULL_DRAW_TICKS);
        place_monster(&mut game, 2, x - 100, z);
        // The claw lands after this tick's draw step; the release arrives before the next.
        assert_eq!(claw(&mut game, 2), CLAW_HIT);
        command(&mut game, ArpgCommand::ReleaseBow { aim: None });
        assert_eq!(action_kind(&game), None);

        let (mut game, _, _) = bow_arena();
        draw_for(&mut game, BOW_FULL_DRAW_TICKS);
        let state = game.players.get_mut(&1).unwrap();
        state.health = 0;
        command(&mut game, ArpgCommand::ReleaseBow { aim: None });
        assert_eq!(action_kind(&game), None);
        assert!(game.arrows.is_empty());
    }

    #[test]
    fn the_bow_disables_sword_strikes_and_guard_without_resetting_guard_break() {
        let (mut game, _, _) = strike_arena();
        game.players
            .get_mut(&1)
            .unwrap()
            .guard
            .broken_ticks_remaining = 20;
        command(
            &mut game,
            ArpgCommand::EquipWeapon {
                weapon: Weapon::Bow,
            },
        );
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        command(&mut game, ArpgCommand::SecondaryAttack { aim: None });
        assert_eq!(action_kind(&game), None);
        command(&mut game, ArpgCommand::SetGuard { raised: true });
        for _ in 0..10 {
            game.advance_tick().unwrap();
            assert_eq!(game.players[&1].guard.stance, None);
        }
        command(
            &mut game,
            ArpgCommand::EquipWeapon {
                weapon: Weapon::SwordAndShield,
            },
        );
        assert_eq!(game.players[&1].guard.broken_ticks_remaining, 10);

        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        command(
            &mut game,
            ArpgCommand::EquipWeapon {
                weapon: Weapon::Bow,
            },
        );
        assert_eq!(
            game.players[&1].weapon,
            Weapon::SwordAndShield,
            "no switch mid-action"
        );
    }

    #[test]
    fn a_released_arrow_outlives_its_shooter() {
        let (mut game, x, z) = bow_arena();
        place_monster(&mut game, 1, x + 600, z);
        game.monsters
            .iter_mut()
            .find(|monster| monster.id == 1)
            .unwrap()
            .health = 10;
        draw_for(&mut game, BOW_FULL_DRAW_TICKS);
        command(&mut game, ArpgCommand::ReleaseBow { aim: None });
        while game.arrows.is_empty() {
            game.advance_tick().unwrap();
        }
        assert!(game.remove_player(1));
        while !game.arrows.is_empty() {
            game.advance_tick().unwrap();
        }
        assert_eq!(monster_health(&game, 1), 0);
    }

    #[test]
    fn live_arrows_are_bounded() {
        let (mut game, _, _) = bow_arena();
        let mut shot = ArpgGame::new_action(content(), ActionKind::Shoot, &game.players[&1], None);
        shot.charge = BOW_FULL_DRAW_TICKS;
        game.players.get_mut(&1).unwrap().action = Some(shot);
        for _ in 0..=MAX_LIVE_ARROWS {
            game.launch_arrow(1, 1, 0, None).unwrap();
        }
        assert_eq!(game.arrows.len(), MAX_LIVE_ARROWS);
        assert_eq!(
            game.arrows[0].id,
            ARROW_ID_BASE + 1,
            "the oldest arrow retires"
        );
    }

    #[test]
    fn saves_before_release_and_mid_flight_continue_identically() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        game.add_player(1).unwrap();
        let mut sequence = 0;
        let mut send = |game: &mut ArpgGame, command: ArpgCommand| {
            sequence += 1;
            game.apply_command(PlayerCommand::new(1, sequence, command).unwrap())
                .unwrap();
        };
        send(
            &mut game,
            ArpgCommand::EquipWeapon {
                weapon: Weapon::Bow,
            },
        );
        send(&mut game, ArpgCommand::DrawBow);
        for _ in 0..12 {
            game.advance_tick().unwrap();
        }
        let before_release = game.save_state().unwrap();
        send(&mut game, ArpgCommand::ReleaseBow { aim: None });
        while game.arrows.is_empty() {
            game.advance_tick().unwrap();
        }
        game.advance_tick().unwrap();
        let mid_flight = game.save_state().unwrap();
        assert_eq!(mid_flight.arrows.len(), 1);

        let mut restored = ArpgGame::from_save_state(mid_flight.clone()).unwrap();
        for _ in 0..50 {
            game.advance_tick().unwrap();
            restored.advance_tick().unwrap();
            assert_eq!(restored.snapshot().unwrap(), game.snapshot().unwrap());
        }

        let mut drawn = ArpgGame::from_save_state(before_release).unwrap();
        assert_eq!(drawn.players[&1].draw_ticks, Some(12));
        drawn.advance_tick().unwrap();
        assert_eq!(drawn.players[&1].draw_ticks, Some(13));

        for corrupt in [
            ArrowSaveState {
                velocity: [0.0; 3],
                ..mid_flight.arrows[0]
            },
            ArrowSaveState {
                id: mid_flight.next_arrow_id,
                ..mid_flight.arrows[0]
            },
            ArrowSaveState {
                damage: ARROW_FULL_DAMAGE + 1,
                ..mid_flight.arrows[0]
            },
            ArrowSaveState {
                velocity: [1.0, 0.0, 0.0],
                ..mid_flight.arrows[0]
            },
            // A launch commits whole-unit components.
            ArrowSaveState {
                velocity: [
                    f32::from(i16::try_from(ARROW_FULL_SPEED).unwrap()) - 0.5,
                    0.0,
                    0.0,
                ],
                ..mid_flight.arrows[0]
            },
            ArrowSaveState {
                velocity: [f32::NAN, 0.0, 0.0],
                ..mid_flight.arrows[0]
            },
            ArrowSaveState {
                position: [f32::INFINITY, PLAYER_Y as f32, 0.0],
                ..mid_flight.arrows[0]
            },
            ArrowSaveState {
                launched_at_tick: 0,
                ticks_remaining: ARROW_LIFETIME_TICKS,
                ..mid_flight.arrows[0]
            },
            ArrowSaveState {
                damage: ARROW_FULL_DAMAGE,
                ..mid_flight.arrows[0]
            },
        ] {
            let mut tampered = mid_flight.clone();
            tampered.arrows[0] = corrupt;
            assert!(
                ArpgGame::from_save_state(tampered)
                    .unwrap_err()
                    .message()
                    .contains("arrow")
            );
        }
        let mut shooting = ArpgGame::from_save_state(mid_flight.clone()).unwrap();
        let mut shot =
            ArpgGame::new_action(content(), ActionKind::Shoot, &shooting.players[&1], None);
        shot.charge = BOW_FULL_DRAW_TICKS;
        shooting.players.get_mut(&1).unwrap().action = Some(shot);
        let mut dead_shot = shooting.save_state().unwrap();
        dead_shot.players[0].health = 0;
        assert!(
            ArpgGame::from_save_state(dead_shot)
                .unwrap_err()
                .message()
                .contains("loadout")
        );

        let mut tampered = mid_flight;
        tampered.players[0].weapon = Weapon::SwordAndShield;
        tampered.players[0].draw_ticks = Some(3);
        assert!(
            ArpgGame::from_save_state(tampered)
                .unwrap_err()
                .message()
                .contains("loadout")
        );
    }

    fn locked(game: &ArpgGame) -> Option<u32> {
        game.players[&1].locked_monster_id
    }

    fn facing(game: &ArpgGame) -> [i8; 2] {
        game.snapshot().unwrap().players[0].facing
    }

    #[test]
    fn aim_commands_are_validated_in_core() {
        let (mut game, _, _) = strike_arena();
        for invalid in [
            [0, 0],
            [AIM_COMPONENT_LIMIT + 1, 0],
            [0, -AIM_COMPONENT_LIMIT - 1],
        ] {
            let error = game
                .apply_command(
                    PlayerCommand::new(
                        1,
                        1,
                        ArpgCommand::SetAim {
                            direction: Some(invalid),
                        },
                    )
                    .unwrap(),
                )
                .unwrap_err();
            assert!(error.message().contains("aim direction"), "{invalid:?}");
            assert_eq!(game.players[&1].aim, None);
        }
        // A rejected command does not consume its sequence number.
        game.apply_command(
            PlayerCommand::new(
                1,
                1,
                ArpgCommand::SetAim {
                    direction: Some([-AIM_COMPONENT_LIMIT, AIM_COMPONENT_LIMIT]),
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(game.players[&1].aim, Some([-1_000, 1_000]));
        assert_eq!(facing(&game), [-1, 1]);
    }

    #[test]
    fn aim_turns_the_facing_independently_of_movement() {
        let (mut game, x, _) = strike_arena();
        command(
            &mut game,
            ArpgCommand::SetAim {
                direction: Some([0, -1]),
            },
        );
        command(&mut game, ArpgCommand::SetMovement { x: 1, z: 0 });
        for _ in 0..10 {
            game.advance_tick().unwrap();
        }
        let snapshot = game.snapshot().unwrap();
        // The body walks east while facing (and aiming) north.
        assert!(snapshot.players[0].position[0] > x);
        assert_eq!(snapshot.players[0].facing, [0, -1]);
        assert_eq!(snapshot.players[0].aim, Some([0, -1]));

        // Clearing the aim restores the default: facing follows movement.
        command(&mut game, ArpgCommand::SetAim { direction: None });
        assert_eq!(facing(&game), [1, 0]);
        assert_eq!(game.snapshot().unwrap().players[0].aim, None);
    }

    #[test]
    fn exact_aim_directs_a_strike_between_the_eight_facings() {
        // 22° below the x axis: still the east facing, but the target 55° further round is
        // inside the swing only when measured from the exact aim.
        let aim = [927, -375];
        assert_eq!(ArpgGame::facing_for(aim), (1, 0));
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 40, z - 175);
        assert!(strike(&mut game, 1, ArpgCommand::PrimaryAttack { aim: None }).is_empty());

        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 40, z - 175);
        command(
            &mut game,
            ArpgCommand::SetAim {
                direction: Some(aim),
            },
        );
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        let action = game.snapshot().unwrap().players[0].action.unwrap();
        assert_eq!(action.aim, Some(aim));
        assert_eq!(action.facing, [1, 0]);
        let mut outcomes = Vec::new();
        while current_action(&game).is_some() {
            game.advance_tick().unwrap();
            outcomes.extend_from_slice(game.strike_outcomes());
        }
        assert_eq!(
            targets(&outcomes),
            vec![(StrikeTarget::Monster(1), LIGHT_HIT)]
        );
    }

    #[test]
    fn aimed_arrows_fly_along_the_exact_direction_and_are_never_steered() {
        let (mut game, _, _) = bow_arena();
        command(
            &mut game,
            ArpgCommand::SetAim {
                direction: Some([1_000, 500]),
            },
        );
        draw_for(&mut game, BOW_FULL_DRAW_TICKS);
        command(&mut game, ArpgCommand::ReleaseBow { aim: None });
        // Re-aiming after the release changes nothing about the committed shot.
        command(
            &mut game,
            ArpgCommand::SetAim {
                direction: Some([-1_000, 0]),
            },
        );
        while game.arrows.is_empty() {
            game.advance_tick().unwrap();
        }
        // 60 units per tick along (2, 1), each component rounded toward zero.
        assert_eq!(whole(game.arrows[0].velocity), [53, 0, 26]);
        let launched = game.arrows[0];
        game.advance_tick().unwrap();
        assert_eq!(game.arrows[0].velocity, launched.velocity);
        assert_eq!(game.arrows[0].position.x, launched.position.x + 53.0);
        assert!(ArpgGame::arrow_launch_is_possible(
            &content().bow,
            &game.arrows[0].save()
        ));
        // No aim or facing launches faster than the draw speed.
        let mut fast = game.arrows[0].save();
        fast.velocity = [(ARROW_FULL_SPEED + 1) as f32, 0.0, 0.0];
        assert!(!ArpgGame::arrow_launch_is_possible(&content().bow, &fast));
        // Every exact aim stays within the draw speed after rounding.
        for x in (-1_000..=1_000).step_by(37) {
            for z in (-1_000..=1_000).step_by(41) {
                if x == 0 && z == 0 {
                    continue;
                }
                let (vx, vz) = ArpgGame::aimed_velocity([x, z], ARROW_FULL_SPEED);
                let length_sq = i64::from(vx).pow(2) + i64::from(vz).pow(2);
                assert!(length_sq <= i64::from(ARROW_FULL_SPEED).pow(2), "{x},{z}");
            }
        }
    }

    #[test]
    fn target_lock_cycles_nearest_first_with_identity_ties_and_line_of_sight() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 4, x - 500, z);
        place_monster(&mut game, 2, x, z + 300);
        place_monster(&mut game, 1, x + 300, z);
        // Out of lock range, and behind a wall: neither can be selected.
        place_monster(&mut game, 5, x + 1_000, z + 1_000);
        place_monster(&mut game, 3, x, z - 400);
        place_blocker(
            &mut game,
            1,
            x,
            z - 200,
            Vec3i::new(80, WALL_HALF_HEIGHT, 10),
        );

        let mut order = Vec::new();
        for _ in 0..4 {
            command(&mut game, ArpgCommand::CycleTarget);
            order.push(locked(&game).unwrap());
        }
        assert_eq!(order, [1, 2, 4, 1]);
        assert_eq!(
            game.snapshot().unwrap().players[0].locked_monster_id,
            Some(1)
        );
        command(&mut game, ArpgCommand::ClearTarget);
        assert_eq!(locked(&game), None);

        // Nothing eligible: cycling selects nothing.
        let (mut empty, x, z) = strike_arena();
        place_monster(&mut empty, 5, x + 1_000, z + 1_000);
        command(&mut empty, ArpgCommand::CycleTarget);
        assert_eq!(locked(&empty), None);
    }

    #[test]
    fn a_lock_is_sticky_to_the_break_range_and_drops_with_its_target() {
        let targeting = content().targeting;
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x, z + 300);
        place_monster(&mut game, 2, x - 300, z);
        command(&mut game, ArpgCommand::CycleTarget);
        assert_eq!(locked(&game), Some(1));
        game.advance_tick().unwrap();
        // The locked player turns to the target although it stands still.
        assert_eq!(facing(&game), [0, 1]);

        // Beyond the lock range but within the break range, behind a wall: still held.
        let far = i32::try_from(targeting.break_range).unwrap();
        game.monsters[0].position = Vec3i::new(x, PLAYER_Y, z + far).into();
        place_blocker(
            &mut game,
            1,
            x,
            z + 200,
            Vec3i::new(80, WALL_HALF_HEIGHT, 10),
        );
        game.advance_tick().unwrap();
        assert_eq!(locked(&game), Some(1));
        // Cycling moves a lock held outside the eligible set to the nearest eligible one.
        command(&mut game, ArpgCommand::CycleTarget);
        assert_eq!(locked(&game), Some(2));
        game.advance_tick().unwrap();
        assert_eq!(facing(&game), [-1, 0]);
        // One unit past the break range drops the lock.
        game.monsters[1].position = Vec3i::new(x - far - 1, PLAYER_Y, z).into();
        game.advance_tick().unwrap();
        assert_eq!(locked(&game), None);
        // Facing keeps its last direction when nothing owns it.
        assert_eq!(facing(&game), [-1, 0]);

        game.monsters[1].position = Vec3i::new(x - 300, PLAYER_Y, z).into();
        command(&mut game, ArpgCommand::CycleTarget);
        assert_eq!(locked(&game), Some(2));
        game.monsters[1].health = 0;
        game.advance_tick().unwrap();
        assert_eq!(locked(&game), None);
    }

    #[test]
    fn a_locked_strike_turns_towards_the_target() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x, z - 150);
        assert!(strike(&mut game, 1, ArpgCommand::PrimaryAttack { aim: None }).is_empty());

        command(&mut game, ArpgCommand::CycleTarget);
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        let action = current_action(&game).unwrap();
        // The lock's direction, at full aim resolution (its length carries no meaning).
        assert_eq!(action.aim, Some([0, -AIM_COMPONENT_LIMIT]));
        assert_eq!((action.facing_x, action.facing_z), (0, -1));
        let mut outcomes = Vec::new();
        while current_action(&game).is_some() {
            game.advance_tick().unwrap();
            outcomes.extend_from_slice(game.strike_outcomes());
        }
        assert_eq!(
            targets(&outcomes),
            vec![(StrikeTarget::Monster(1), LIGHT_HIT)]
        );
    }

    #[test]
    fn a_held_lock_faces_the_target_from_where_physics_left_the_player() {
        // Monster strikes resolve guards against this facing within the same tick.
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x, z - 150);
        command(&mut game, ArpgCommand::CycleTarget);
        assert_eq!(locked(&game), Some(1));
        let state = &game.players[&1];
        assert_eq!((state.facing_x, state.facing_z), (0, -1));

        // The player ends a physics step east of the target.
        game.put_player(1, Vec3i::new(x + 150, PLAYER_Y, z - 150));
        game.face_held_locks().unwrap();
        let state = &game.players[&1];
        assert_eq!((state.facing_x, state.facing_z), (-1, 0));
    }

    #[test]
    fn losing_the_lock_never_suppresses_an_empty_swing() {
        // The target dies before the press: the swing still happens along the facing.
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x, z - 150);
        place_monster(&mut game, 2, x - 600, z);
        command(&mut game, ArpgCommand::CycleTarget);
        assert_eq!(locked(&game), Some(1));
        game.monsters[0].health = 0;
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        let action = current_action(&game).unwrap();
        assert_eq!(action.kind, ActionKind::PrimaryAttack);
        // The lock had turned the player; the swing keeps that facing without an aim.
        assert_eq!(action.aim, None);
        assert_eq!((action.facing_x, action.facing_z), (0, -1));
        advance_to_phase(&mut game, ActionKind::PrimaryAttack, ActionPhase::Recovery);
        assert_eq!(locked(&game), None);

        // The target dies during the wind-up: the committed strike still goes active.
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x, z - 150);
        place_monster(&mut game, 2, x - 600, z);
        command(&mut game, ArpgCommand::CycleTarget);
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        game.advance_tick().unwrap();
        game.monsters[0].health = 0;
        let mut went_active = false;
        while let Some(action) = current_action(&game) {
            assert_eq!(action.aim, Some([0, -AIM_COMPONENT_LIMIT]));
            went_active |= action.phase == ActionPhase::Active;
            game.advance_tick().unwrap();
            assert!(game.strike_outcomes().is_empty());
        }
        assert!(went_active);
        assert_eq!(locked(&game), None);
    }

    #[test]
    fn a_released_arrow_is_never_redirected_by_the_lock() {
        let (mut game, x, z) = bow_arena();
        place_monster(&mut game, 1, x, z + 400);
        command(&mut game, ArpgCommand::CycleTarget);
        assert_eq!(locked(&game), Some(1));
        draw_for(&mut game, BOW_FULL_DRAW_TICKS);
        command(&mut game, ArpgCommand::ReleaseBow { aim: None });
        // Losing the lock between release and launch does not steer the shot.
        command(&mut game, ArpgCommand::ClearTarget);
        while game.arrows.is_empty() {
            game.advance_tick().unwrap();
        }
        assert_eq!(whole(game.arrows[0].velocity), [0, 0, ARROW_FULL_SPEED]);
        // Nor does the target moving or dying in flight.
        game.monsters[1].position = Vec3i::new(x + 300, PLAYER_Y, z).into();
        game.monsters[1].health = 0;
        game.advance_tick().unwrap();
        assert_eq!(whole(game.arrows[0].velocity), [0, 0, ARROW_FULL_SPEED]);
    }

    #[test]
    fn aim_and_lock_survive_save_and_continue_identically() {
        let mut game = ArpgGame::new_scenario(ScenarioId::Enemy, 42).unwrap();
        game.add_player(1).unwrap();
        // The first tick activates the encounter, making its enemy lockable.
        game.advance_tick().unwrap();
        command(
            &mut game,
            ArpgCommand::SetAim {
                direction: Some([-3, 7]),
            },
        );
        command(&mut game, ArpgCommand::CycleTarget);
        let target = locked(&game).expect("the scenario enemy is in lock range");
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        game.advance_tick().unwrap();
        let saved = game.save_state().unwrap();
        assert_eq!(saved.players[0].aim, Some([-3, 7]));
        assert_eq!(saved.players[0].locked_monster_id, Some(target));
        assert!(saved.players[0].action.unwrap().aim.is_some());

        let mut restored = ArpgGame::from_save_state(saved.clone()).unwrap();
        for tick in 0..60 {
            if tick == 30 {
                for game in [&mut game, &mut restored] {
                    command(game, ArpgCommand::SecondaryAttack { aim: None });
                }
            }
            game.advance_tick().unwrap();
            restored.advance_tick().unwrap();
            assert_eq!(restored.snapshot().unwrap(), game.snapshot().unwrap());
        }
        assert_eq!(restored.save_state().unwrap(), game.save_state().unwrap());

        let mut zero_aim = saved.clone();
        zero_aim.players[0].aim = Some([0, 0]);
        let mut wide_aim = saved.clone();
        wide_aim.players[0].aim = Some([AIM_COMPONENT_LIMIT + 1, 0]);
        let mut unknown_lock = saved.clone();
        unknown_lock.players[0].locked_monster_id = Some(77);
        // Without a lock the saved facing must follow the aim.
        let mut stale_facing = saved.clone();
        stale_facing.players[0].locked_monster_id = None;
        stale_facing.players[0].action = None;
        stale_facing.players[0].aim = Some([1000, 0]);
        stale_facing.players[0].facing = [-1, 0];
        // A locked player faces its target once restored, so a reversed facing is corrupt.
        let mut locked_facing = saved.clone();
        locked_facing.players[0].facing = [
            -locked_facing.players[0].facing[0],
            -locked_facing.players[0].facing[1],
        ];
        let mut aligned = stale_facing.clone();
        aligned.players[0].facing = [1, 0];
        assert!(ArpgGame::from_save_state(aligned).is_ok());
        let mut mismatched = saved;
        let action = mismatched.players[0].action.as_mut().unwrap();
        action.facing = [-action.facing[0], -action.facing[1]];
        for (corrupt, reason) in [
            (zero_aim, "aim direction"),
            (wide_aim, "aim direction"),
            (unknown_lock, "target lock"),
            (stale_facing, "facing does not match its aim"),
            (locked_facing, "facing does not match its target lock"),
            (mismatched, "does not match its aim"),
        ] {
            let error = ArpgGame::from_save_state(corrupt).unwrap_err();
            assert!(error.message().contains(reason), "{}", error.message());
        }
    }

    #[test]
    fn aimed_and_locked_reproductions_replay_identically() {
        let at = |tick, sequence, command| ReproductionCommand {
            tick,
            player_id: 1,
            sequence,
            command,
        };
        let reproduction = Reproduction {
            scenario: ScenarioId::Archery,
            seed: 11,
            players: vec![1],
            ticks: 150,
            commands: vec![
                at(1, 1, ArpgCommand::CycleTarget),
                at(
                    2,
                    2,
                    ArpgCommand::EquipWeapon {
                        weapon: Weapon::Bow,
                    },
                ),
                at(3, 3, ArpgCommand::DrawBow),
                at(40, 4, ArpgCommand::ReleaseBow { aim: None }),
                at(41, 5, ArpgCommand::ClearTarget),
                at(
                    42,
                    6,
                    ArpgCommand::SetAim {
                        direction: Some([700, -250]),
                    },
                ),
                at(43, 7, ArpgCommand::SetMovement { x: -1, z: 0 }),
            ],
            operations: Vec::new(),
        };
        let decoded: Reproduction =
            serde_json::from_str(&serde_json::to_string(&reproduction).unwrap()).unwrap();
        assert_eq!(decoded, reproduction);
        let first = replay_reproduction(&reproduction).unwrap();
        assert_eq!(first, replay_reproduction(&decoded).unwrap());
        assert!(first[5].players[0].locked_monster_id.is_some());
        assert!(first.iter().any(|snapshot| {
            snapshot
                .strike_events
                .iter()
                .any(|event| event.definition == "bow.arrow")
        }));
        let last = &first[149].players[0];
        assert_eq!(last.aim, Some([700, -250]));
        assert_eq!(last.facing, [1, 0]);
        assert_eq!(last.locked_monster_id, None);
    }

    fn scenario_player_events(
        scenario: ScenarioId,
        commands: &[(u64, ArpgCommand)],
        ticks: u64,
    ) -> Vec<StrikeEventSnapshot> {
        let reproduction = Reproduction {
            scenario,
            seed: 42,
            players: vec![1],
            ticks,
            commands: commands
                .iter()
                .enumerate()
                .map(|(index, &(tick, command))| ReproductionCommand {
                    tick,
                    player_id: 1,
                    sequence: u32::try_from(index + 1).unwrap(),
                    command,
                })
                .collect(),
            operations: Vec::new(),
        };
        replay_reproduction(&reproduction)
            .unwrap()
            .into_iter()
            .flat_map(|snapshot| snapshot.strike_events)
            .collect()
    }

    #[test]
    fn every_scenario_arranges_the_real_runtime_for_many_seeds() {
        for scenario in ScenarioId::ALL {
            assert_eq!(ScenarioId::parse(scenario.name()), Some(scenario));
            assert_eq!(
                serde_json::to_value(scenario).unwrap(),
                serde_json::json!(scenario.name())
            );
            for seed in [0, 42, 0xdead_beef, 0xa420_0916] {
                let mut game = ArpgGame::new_scenario(scenario, seed).unwrap();
                game.add_player(1).unwrap();
                game.advance_tick().unwrap();
                let snapshot = game.snapshot().unwrap();
                assert_eq!(snapshot.scenario, scenario);
                let Some(offset) = scenario.target_offset() else {
                    assert_eq!(snapshot, {
                        let mut plain = ArpgGame::new_with_seed(seed).unwrap();
                        plain.add_player(1).unwrap();
                        plain.advance_tick().unwrap();
                        plain.snapshot().unwrap()
                    });
                    continue;
                };
                let room_id = scenario.room(&game.content, &game.monsters).unwrap();
                let room = snapshot
                    .rooms
                    .iter()
                    .find(|room| room.id == room_id)
                    .unwrap();
                assert_eq!(
                    room.encounter_state,
                    RoomEncounterState::Active,
                    "{scenario:?}/{seed}"
                );
                let (x, z) = room.center();
                assert_eq!(snapshot.players[0].position, [x, PLAYER_Y, z]);
                let target = snapshot
                    .monsters
                    .iter()
                    .find(|monster| monster.room_id == room_id)
                    .unwrap();
                assert_eq!(target.position, [x + offset, PLAYER_Y, z]);
                assert_eq!(
                    target.definition,
                    scenario.enemy().unwrap_or(BRUTE),
                    "{scenario:?}/{seed}"
                );
                let pillars = snapshot
                    .static_colliders
                    .iter()
                    .filter(|collider| collider.kind == StaticColliderKind::Pillar)
                    .count();
                assert_eq!(pillars, usize::from(scenario.pillar_offset().is_some()));
            }
        }
        assert_eq!(ScenarioId::parse("nonsense"), None);
    }

    #[test]
    fn scenarios_demonstrate_melee_reach_obstruction_and_monster_attacks() {
        let swing = [(0, ArpgCommand::PrimaryAttack { aim: None })];
        let hits = scenario_player_events(ScenarioId::Dummy, &swing, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].definition, "sword.lightSwing");
        assert!(matches!(hits[0].result, StrikeResult::Hit { .. }));

        let blocked = scenario_player_events(ScenarioId::Obstructed, &swing, 10);
        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0].result, StrikeResult::Obstructed);

        let empty = scenario_player_events(
            ScenarioId::Dummy,
            &[
                (0, ArpgCommand::SetMovement { x: -1, z: 0 }),
                (1, ArpgCommand::PrimaryAttack { aim: None }),
            ],
            10,
        );
        assert!(
            empty.is_empty(),
            "an unaimed swing away from the dummy whiffs"
        );

        let enemy = scenario_player_events(ScenarioId::Enemy, &[], 30);
        assert!(enemy.iter().any(|event| {
            event.source == StrikeSource::Monster(event_monster(&enemy))
                && event.definition == "monster.claw"
        }));
    }

    fn event_monster(events: &[StrikeEventSnapshot]) -> u32 {
        events
            .iter()
            .find_map(|event| match event.source {
                StrikeSource::Monster(id) => Some(id),
                StrikeSource::Player(_) => None,
            })
            .unwrap()
    }

    #[test]
    fn archery_scenarios_hit_and_stop_at_the_pillar() {
        let shot = [
            (
                0,
                ArpgCommand::EquipWeapon {
                    weapon: Weapon::Bow,
                },
            ),
            (0, ArpgCommand::DrawBow),
            (31, ArpgCommand::ReleaseBow { aim: None }),
        ];
        let hit = scenario_player_events(ScenarioId::Archery, &shot, 60);
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].definition, "bow.arrow");
        assert!(scenario_player_events(ScenarioId::ArcheryObstructed, &shot, 60).is_empty());
    }

    #[test]
    fn a_swing_at_the_retreating_enemy_misses_once_it_backs_out_of_reach() {
        let light = content().action(ActionKind::PrimaryAttack);
        let reach = light.strike.unwrap().reach;
        let windup = u64::from(light.windup_ticks);
        for seed in [0, 42, 0xdead_beef, 0xa420_0916] {
            let reproduction = Reproduction {
                scenario: ScenarioId::Retreating,
                seed,
                players: vec![1],
                ticks: windup + 2,
                commands: vec![ReproductionCommand {
                    tick: 0,
                    player_id: 1,
                    sequence: 1,
                    command: ArpgCommand::PrimaryAttack { aim: None },
                }],
                operations: Vec::new(),
            };
            let snapshots = replay_reproduction(&reproduction).unwrap();
            let distance = |snapshot: &ArpgSnapshot| {
                let player = snapshot.players[0].position;
                let archer = snapshot
                    .monsters
                    .iter()
                    .find(|monster| monster.definition == "monster.archer")
                    .unwrap();
                (i64::from(archer.position[0] - player[0]), archer.behavior)
            };
            // Standing still, the archer would be inside the swing; it backs off instead.
            let (start, _) = distance(&snapshots[0]);
            assert!(start <= reach, "{seed}: {start} > {reach}");
            // The swing resolves on the tick its active phase opens.
            let strike_tick = snapshots
                .iter()
                .find(|snapshot| {
                    snapshot.players[0]
                        .action
                        .is_some_and(|action| action.phase == ActionPhase::Active)
                })
                .unwrap();
            assert_eq!(strike_tick.tick, windup, "{seed}");
            let (at_strike, behavior) = distance(strike_tick);
            assert_eq!(behavior, MonsterBehavior::Retreating, "{seed}");
            assert!(at_strike > reach, "{seed}: {at_strike} <= {reach}");
            assert!(
                snapshots
                    .iter()
                    .all(|snapshot| snapshot.strike_events.is_empty()),
                "{seed}: the swing whiffs and the archer has not shot yet"
            );
        }
    }

    #[test]
    fn a_counter_against_the_archer_that_backed_off_starts_but_misses() {
        let counter = content().action(ActionKind::Counter);
        let reach = counter.strike.unwrap().reach;
        for seed in [0, 42, 0xdead_beef, 0xa420_0916] {
            let mut game = ArpgGame::new_scenario(ScenarioId::Retreating, seed).unwrap();
            game.add_player(1).unwrap();
            command(&mut game, ArpgCommand::SetGuard { raised: true });
            let archer_distance = |game: &ArpgGame| {
                let player = game.snapshot().unwrap().players[0].position;
                let snapshot = game.snapshot().unwrap();
                let archer = snapshot
                    .monsters
                    .iter()
                    .find(|monster| monster.definition == "monster.archer")
                    .unwrap();
                (i64::from(archer.position[0] - player[0]), archer.behavior)
            };
            let mut blocked = false;
            let mut backed_off = false;
            for _ in 0..240 {
                game.advance_tick().unwrap();
                backed_off |= archer_distance(&game).1 == MonsterBehavior::Retreating;
                blocked = game.snapshot().unwrap().strike_events.iter().any(|event| {
                    event.definition == "monster.bolt"
                        && matches!(event.result, StrikeResult::Blocked { .. })
                });
                if blocked {
                    break;
                }
            }
            assert!(blocked, "{seed}: the archer's bolt is never blocked");
            assert!(
                backed_off,
                "{seed}: the archer shoots without backing off first"
            );
            assert!(game.players[&1].counter.is_some(), "{seed}");
            command(&mut game, ArpgCommand::SetGuard { raised: false });
            command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
            // The block opened a real counter: it starts and consumes the opportunity.
            assert_eq!(action_kind(&game), Some(ActionKind::Counter), "{seed}");
            assert!(game.players[&1].counter.is_none(), "{seed}");
            let mut resolved = false;
            for _ in 0..u64::from(counter.windup_ticks) + 2 {
                game.advance_tick().unwrap();
                let snapshot = game.snapshot().unwrap();
                let active = snapshot.players[0]
                    .action
                    .is_some_and(|action| action.phase == ActionPhase::Active);
                if active {
                    resolved = true;
                    // The archer kept its distance: the counter honours reach and whiffs.
                    let (distance, _) = archer_distance(&game);
                    assert!(distance > reach, "{seed}: {distance} <= {reach}");
                    assert!(
                        !snapshot
                            .strike_events
                            .iter()
                            .any(|event| event.source == StrikeSource::Player(1)),
                        "{seed}: the counter must not reach the retreating archer"
                    );
                }
            }
            assert!(
                resolved,
                "{seed}: the counter never reached its active phase"
            );
        }
    }

    #[test]
    fn reproductions_replay_identically_and_reject_malformed_input() {
        let reproduction = Reproduction {
            scenario: ScenarioId::Enemy,
            seed: 7,
            players: vec![1, 2],
            ticks: 120,
            commands: vec![
                ReproductionCommand {
                    tick: 3,
                    player_id: 1,
                    sequence: 1,
                    command: ArpgCommand::SetGuard { raised: true },
                },
                ReproductionCommand {
                    tick: 40,
                    player_id: 2,
                    sequence: 1,
                    command: ArpgCommand::PrimaryAttack { aim: None },
                },
            ],
            operations: Vec::new(),
        };
        let encoded = serde_json::to_string(&reproduction).unwrap();
        let decoded: Reproduction = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, reproduction);
        let first = replay_reproduction(&reproduction).unwrap();
        assert_eq!(first.len(), 120);
        assert_eq!(first, replay_reproduction(&decoded).unwrap());

        let mut late = reproduction.clone();
        late.commands[1].tick = 500;
        assert!(replay_reproduction(&late).is_err());
        let mut unordered = reproduction.clone();
        unordered.commands.swap(0, 1);
        assert!(replay_reproduction(&unordered).is_err());
        let mut long = reproduction;
        long.ticks = MAX_REPRODUCTION_TICKS + 1;
        assert!(replay_reproduction(&long).is_err());
    }

    #[test]
    fn recorded_sessions_replay_to_the_same_snapshots() {
        let mut game = ArpgGame::new_scenario(ScenarioId::Enemy, 42).unwrap();
        let mut recorder = ReproductionRecorder::start(&game).unwrap();
        game.add_player(1).unwrap();
        recorder.record_player_added(1);
        let script = [
            (0, ArpgCommand::SetGuard { raised: true }),
            (25, ArpgCommand::SetGuard { raised: false }),
            (26, ArpgCommand::PrimaryAttack { aim: None }),
            (26, ArpgCommand::SetMovement { x: 1, z: 0 }),
            (40, ArpgCommand::SetMovement { x: 0, z: 0 }),
            (41, ArpgCommand::SecondaryAttack { aim: None }),
        ];
        let mut sequence = 0;
        let mut live = Vec::new();
        for tick in 0..90 {
            for &(_, command) in script.iter().filter(|(at, _)| *at == tick) {
                sequence += 1;
                let command = PlayerCommand::new(1, sequence, command).unwrap();
                game.apply_command(command.clone()).unwrap();
                recorder.record_command(&command);
            }
            // A command the authority rejects (stale sequence) never reaches the recording.
            if tick == 30 {
                let stale =
                    PlayerCommand::new(1, 1, ArpgCommand::PrimaryAttack { aim: None }).unwrap();
                if game.apply_command(stale.clone()).is_ok() {
                    recorder.record_command(&stale);
                }
            }
            game.advance_tick().unwrap();
            recorder.record_tick();
            live.push(game.snapshot().unwrap());
        }
        let exported = serde_json::to_string(&recorder.reproduction().unwrap()).unwrap();
        let imported: Reproduction = serde_json::from_str(&exported).unwrap();
        assert_eq!(imported.scenario, ScenarioId::Enemy);
        assert_eq!(imported.seed, 42);
        assert_eq!(imported.players, vec![1]);
        assert_eq!(imported.ticks, 90);
        assert_eq!(imported.commands.len(), script.len());
        assert_eq!(replay_reproduction(&imported).unwrap(), live);
    }

    #[test]
    fn commands_after_the_last_completed_tick_are_left_out_of_the_export() {
        let mut game = ArpgGame::new_scenario(ScenarioId::Dummy, 7).unwrap();
        let mut recorder = ReproductionRecorder::start(&game).unwrap();
        game.add_player(1).unwrap();
        recorder.record_player_added(1);
        let mut live = Vec::new();
        for sequence in 1..=2 {
            let command =
                PlayerCommand::new(1, sequence, ArpgCommand::PrimaryAttack { aim: None }).unwrap();
            game.apply_command(command.clone()).unwrap();
            recorder.record_command(&command);
            if sequence == 1 {
                game.advance_tick().unwrap();
                recorder.record_tick();
                live.push(game.snapshot().unwrap());
            }
        }
        // The second command was accepted while paused, after the last completed tick.
        let reproduction = recorder.reproduction().unwrap();
        assert_eq!(reproduction.ticks, 1);
        assert_eq!(reproduction.commands.len(), 1);
        assert_eq!(replay_reproduction(&reproduction).unwrap(), live);
    }

    #[test]
    fn recordings_stop_where_a_reproduction_cannot_follow() {
        assert!(
            ReproductionRecorder::start(&{
                let mut game = ArpgGame::new_scenario(ScenarioId::Dummy, 1).unwrap();
                game.add_player(1).unwrap();
                game
            })
            .is_err()
        );
        let game = ArpgGame::new_scenario(ScenarioId::Dummy, 1).unwrap();
        let fresh = ReproductionRecorder::start(&game).unwrap();

        let mut late_join = fresh.clone();
        late_join.record_player_added(1);
        late_join.record_tick();
        late_join.record_player_added(2);
        let error = late_join.reproduction().unwrap_err();
        assert!(
            error.message().contains("joined after"),
            "{}",
            error.message()
        );

        let mut left = fresh.clone();
        left.record_player_added(1);
        left.record_player_removed();
        assert!(left.reproduction().is_err());

        let mut long = fresh.clone();
        for _ in 0..MAX_REPRODUCTION_TICKS {
            long.record_tick();
        }
        assert_eq!(long.reproduction().unwrap().ticks, MAX_REPRODUCTION_TICKS);
        long.record_tick();
        assert!(long.reproduction().is_err());

        let mut chatty = fresh;
        let command = PlayerCommand::new(1, 1, ArpgCommand::Interact).unwrap();
        for _ in 0..MAX_REPRODUCTION_COMMANDS {
            chatty.record_command(&command);
        }
        assert!(chatty.reproduction().is_ok());
        chatty.record_command(&command);
        assert!(chatty.reproduction().is_err());

        let mut flood = Reproduction {
            scenario: ScenarioId::Dummy,
            seed: 1,
            players: vec![1],
            ticks: 1,
            commands: Vec::new(),
            operations: Vec::new(),
        };
        flood.commands = vec![
            ReproductionCommand {
                tick: 0,
                player_id: 1,
                sequence: 1,
                command: ArpgCommand::Interact,
            };
            MAX_REPRODUCTION_COMMANDS + 1
        ];
        assert!(replay_reproduction(&flood).is_err());
    }

    #[test]
    fn players_joining_a_restored_scenario_spawn_at_the_authored_points() {
        let mut game = ArpgGame::new_scenario(ScenarioId::Enemy, 42).unwrap();
        game.add_player(1).unwrap();
        game.advance_tick().unwrap();
        let mut restored = ArpgGame::from_save_state(game.save_state().unwrap()).unwrap();
        game.add_player(2).unwrap();
        restored.add_player(2).unwrap();
        game.advance_tick().unwrap();
        restored.advance_tick().unwrap();
        assert_eq!(restored.snapshot().unwrap(), game.snapshot().unwrap());
    }

    #[test]
    fn scenario_saves_restore_their_authored_geometry() {
        let mut game = ArpgGame::new_scenario(ScenarioId::Obstructed, 42).unwrap();
        game.add_player(1).unwrap();
        for _ in 0..5 {
            game.advance_tick().unwrap();
        }
        let save = game.save_state().unwrap();
        assert_eq!(save.scenario, ScenarioId::Obstructed);
        let mut restored = ArpgGame::from_save_state(save).unwrap();
        game.apply_command(
            PlayerCommand::new(1, 1, ArpgCommand::PrimaryAttack { aim: None }).unwrap(),
        )
        .unwrap();
        restored
            .apply_command(
                PlayerCommand::new(1, 1, ArpgCommand::PrimaryAttack { aim: None }).unwrap(),
            )
            .unwrap();
        for _ in 0..12 {
            game.advance_tick().unwrap();
            restored.advance_tick().unwrap();
            assert_eq!(restored.snapshot().unwrap(), game.snapshot().unwrap());
        }
    }

    fn room2_chest(game: &ArpgGame) -> ChestSnapshot {
        *game
            .chests
            .iter()
            .find(|chest| chest.room_id == STRIKE_ROOM)
            .unwrap()
    }

    fn move_player(game: &mut ArpgGame, player_id: PlayerId, x: i32, z: i32) {
        game.put_player(player_id, Vec3i::new(x, PLAYER_Y, z));
    }

    /// Interacts and returns the interaction events resolved for that action.
    fn interact(game: &mut ArpgGame, player_id: PlayerId) -> Vec<InteractionEventSnapshot> {
        let sequence = game
            .last_sequences
            .get(&player_id)
            .copied()
            .unwrap_or_default()
            + 1;
        game.apply_command(PlayerCommand::new(player_id, sequence, ArpgCommand::Interact).unwrap())
            .unwrap();
        let mut events = Vec::new();
        while game.players[&player_id].action.is_some() {
            game.advance_tick().unwrap();
            events.extend(game.snapshot().unwrap().interaction_events);
        }
        events
    }

    fn prompt(game: &ArpgGame) -> InteractionPrompt {
        game.snapshot().unwrap().players[0].interaction
    }

    #[test]
    fn a_room_chest_stays_locked_until_its_encounter_is_cleared_and_opens_once() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x - 700, z);
        let chest = room2_chest(&game);
        let [cx, _, cz] = chest.position;
        move_player(&mut game, 1, cx, cz - 100);
        let locked = InteractionResult::Refused {
            reason: InteractionRefusal::ChestLocked,
        };
        assert_eq!(
            prompt(&game),
            InteractionPrompt::Unavailable {
                reason: InteractionRefusal::ChestLocked
            }
        );
        assert_eq!(interact(&mut game, 1)[0].result, locked);

        game.monsters[0].health = 0;
        game.reconcile_encounters().unwrap();
        assert!(
            game.snapshot()
                .unwrap()
                .chests
                .iter()
                .any(|c| c.id == chest.id && c.available)
        );
        let target = InteractionTarget::Chest(chest.id);
        assert_eq!(prompt(&game), InteractionPrompt::Available { target });
        let gold = game.players[&1].gold;
        assert_eq!(
            interact(&mut game, 1)[0].result,
            InteractionResult::Opened {
                target,
                gold: CHEST_GOLD_AMOUNT
            }
        );
        assert_eq!(game.players[&1].gold, gold + CHEST_GOLD_AMOUNT);
        assert_eq!(
            interact(&mut game, 1)[0].result,
            InteractionResult::Refused {
                reason: InteractionRefusal::NothingInRange
            },
            "an opened chest cannot pay out twice"
        );
    }

    #[test]
    fn interaction_prefers_the_nearest_target_then_loot_then_identity() {
        let (mut game, x, z) = strike_arena();
        game.ground_loot.push(GroundLootState {
            id: GROUND_LOOT_ID_BASE + 7,
            position: Vec3i::new(x + 100, PLAYER_Y, z).into(),
            kind: LootKind::Gold,
            amount: GROUND_LOOT_GOLD_AMOUNT,
        });
        game.ground_loot.push(GroundLootState {
            id: GROUND_LOOT_ID_BASE + 3,
            position: Vec3i::new(x - 100, PLAYER_Y, z).into(),
            kind: LootKind::Gold,
            amount: GROUND_LOOT_GOLD_AMOUNT,
        });
        let chest = room2_chest(&game);
        game.chests
            .iter_mut()
            .find(|c| c.id == chest.id)
            .unwrap()
            .position = [x, PLAYER_Y, z + 100];
        game.reconcile_encounters().unwrap();
        // Equal distances: loot before chests, then the lower loot id.
        let order = (0..3)
            .map(|_| match interact(&mut game, 1)[0].result {
                InteractionResult::PickedUp { target, .. }
                | InteractionResult::Opened { target, .. } => target,
                InteractionResult::Refused { reason } => panic!("refused: {reason:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            order,
            [
                InteractionTarget::Loot(GROUND_LOOT_ID_BASE + 3),
                InteractionTarget::Loot(GROUND_LOOT_ID_BASE + 7),
                InteractionTarget::Chest(chest.id),
            ]
        );
    }

    #[test]
    fn walls_block_interaction_and_explain_why() {
        let (mut game, x, z) = strike_arena();
        game.ground_loot.push(GroundLootState {
            id: GROUND_LOOT_ID_BASE,
            position: Vec3i::new(x + 120, PLAYER_Y, z).into(),
            kind: LootKind::Gold,
            amount: GROUND_LOOT_GOLD_AMOUNT,
        });
        place_blocker(&mut game, 0, x + 70, z, Vec3i::new(5, WALL_HALF_HEIGHT, 40));
        assert_eq!(
            interact(&mut game, 1)[0].result,
            InteractionResult::Refused {
                reason: InteractionRefusal::Obstructed
            }
        );
        game.ground_loot.push(GroundLootState {
            id: GROUND_LOOT_ID_BASE + 1,
            position: Vec3i::new(x - 140, PLAYER_Y, z).into(),
            kind: LootKind::Gold,
            amount: GROUND_LOOT_GOLD_AMOUNT,
        });
        assert!(matches!(
            interact(&mut game, 1)[0].result,
            InteractionResult::PickedUp {
                target: InteractionTarget::Loot(id),
                ..
            } if id == GROUND_LOOT_ID_BASE + 1
        ));
        assert_eq!(game.ground_loot.len(), 1, "the walled-off loot stays");
    }

    #[test]
    fn simultaneous_pickups_pay_exactly_once() {
        let (mut game, x, z) = strike_arena();
        game.add_player(2).unwrap();
        move_player(&mut game, 2, x, z + 40);
        game.ground_loot.push(GroundLootState {
            id: GROUND_LOOT_ID_BASE,
            position: Vec3i::new(x + 50, PLAYER_Y, z + 20).into(),
            kind: LootKind::Gold,
            amount: GROUND_LOOT_GOLD_AMOUNT,
        });
        for player in [1, 2] {
            game.apply_command(PlayerCommand::new(player, 1, ArpgCommand::Interact).unwrap())
                .unwrap();
        }
        let mut events = Vec::new();
        while game.players[&1].action.is_some() || game.players[&2].action.is_some() {
            game.advance_tick().unwrap();
            events.extend(game.snapshot().unwrap().interaction_events);
        }
        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0],
            InteractionEventSnapshot {
                player_id: 1,
                result: InteractionResult::PickedUp { .. },
                ..
            }
        ));
        assert_eq!(
            events[1].result,
            InteractionResult::Refused {
                reason: InteractionRefusal::NothingInRange
            }
        );
        assert_eq!(
            game.players[&1].gold + game.players[&2].gold,
            GROUND_LOOT_GOLD_AMOUNT
        );
    }

    #[test]
    fn prompts_are_busy_mid_action_and_events_share_one_tick_order() {
        let (mut game, x, z) = strike_arena();
        game.add_player(2).unwrap();
        move_player(&mut game, 2, x, z + 300);
        place_monster(&mut game, 1, x + 100, z);
        game.ground_loot.push(GroundLootState {
            id: GROUND_LOOT_ID_BASE,
            position: Vec3i::new(x, PLAYER_Y, z + 340).into(),
            kind: LootKind::Gold,
            amount: GROUND_LOOT_GOLD_AMOUNT,
        });
        // Player 1 attacks; player 2 interacts so both resolve in the same tick.
        game.apply_command(
            PlayerCommand::new(1, 1, ArpgCommand::PrimaryAttack { aim: None }).unwrap(),
        )
        .unwrap();
        for _ in 0..(PRIMARY_WINDUP_TICKS - INTERACT_WINDUP_TICKS) {
            game.advance_tick().unwrap();
        }
        game.apply_command(PlayerCommand::new(2, 1, ArpgCommand::Interact).unwrap())
            .unwrap();
        assert_eq!(
            game.snapshot().unwrap().players[1].interaction,
            InteractionPrompt::Unavailable {
                reason: InteractionRefusal::Busy
            }
        );
        let mut both = None;
        for _ in 0..10 {
            game.advance_tick().unwrap();
            let snapshot = game.snapshot().unwrap();
            if !snapshot.strike_events.is_empty() && !snapshot.interaction_events.is_empty() {
                both = Some(snapshot);
                break;
            }
        }
        let snapshot = both.expect("a strike and an interaction in the same tick");
        // Players resolve in id order: player 1's strike, then player 2's pickup.
        assert_eq!(snapshot.strike_events[0].order, 0);
        assert_eq!(snapshot.interaction_events[0].order, 1);
    }

    #[test]
    fn interaction_events_report_the_gold_actually_credited() {
        let (mut game, x, z) = strike_arena();
        game.players.get_mut(&1).unwrap().gold = u32::MAX - 3;
        game.ground_loot.push(GroundLootState {
            id: GROUND_LOOT_ID_BASE,
            position: Vec3i::new(x + 50, PLAYER_Y, z).into(),
            kind: LootKind::Gold,
            amount: GROUND_LOOT_GOLD_AMOUNT,
        });
        let events = interact(&mut game, 1);
        assert_eq!(
            events[0].result,
            InteractionResult::PickedUp {
                target: InteractionTarget::Loot(GROUND_LOOT_ID_BASE),
                gold: 3
            }
        );
        assert_eq!(game.players[&1].gold, u32::MAX);
    }

    #[test]
    fn opened_chests_survive_saves_and_unknown_ones_are_rejected() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        game.add_player(1).unwrap();
        let id = game.chests[0].id;
        let room = game.chests[0].room_id;
        let mut uncleared = game.save_state().unwrap();
        uncleared.opened_chests = vec![id];
        assert!(
            ArpgGame::from_save_state(uncleared).is_err(),
            "a chest cannot be open while its room is not cleared"
        );
        game.rooms
            .iter_mut()
            .find(|r| r.id == room)
            .unwrap()
            .encounter_state = RoomEncounterState::Cleared;
        for monster in game.monsters.iter_mut().filter(|m| m.room_id == room) {
            monster.health = 0;
        }
        game.chests[0].opened = true;
        let save = game.save_state().unwrap();
        assert_eq!(save.opened_chests, [id]);
        let restored = ArpgGame::from_save_state(save.clone()).unwrap();
        assert!(restored.chests[0].opened);
        for corrupt in [vec![1], vec![id, id]] {
            let mut tampered = save.clone();
            tampered.opened_chests = corrupt;
            assert!(ArpgGame::from_save_state(tampered).is_err());
        }
    }

    /// The extracted base content must reproduce the previous hard-coded tuning exactly; the
    /// constants below are the pre-extraction values kept as an independent reference.
    #[test]
    fn base_content_reproduces_the_previous_tuning() {
        let expected = [
            (
                ActionKind::PrimaryAttack,
                (
                    PRIMARY_WINDUP_TICKS,
                    PRIMARY_ACTIVE_TICKS,
                    PRIMARY_RECOVERY_TICKS,
                ),
                Some((ATTACK_RANGE, usize::MAX)),
                (1, 1, PRIMARY_STAGGER_TICKS),
            ),
            (
                ActionKind::SecondaryAttack,
                (
                    SECONDARY_WINDUP_TICKS,
                    SECONDARY_ACTIVE_TICKS,
                    SECONDARY_RECOVERY_TICKS,
                ),
                Some((SECONDARY_ATTACK_RANGE, 1)),
                (
                    SECONDARY_ATTACK_DAMAGE_NUMERATOR,
                    SECONDARY_ATTACK_DAMAGE_DENOMINATOR,
                    SECONDARY_STAGGER_TICKS,
                ),
            ),
            (
                ActionKind::Interact,
                (
                    INTERACT_WINDUP_TICKS,
                    INTERACT_ACTIVE_TICKS,
                    INTERACT_RECOVERY_TICKS,
                ),
                None,
                (0, 1, 0),
            ),
            (
                ActionKind::Counter,
                (
                    COUNTER_WINDUP_TICKS,
                    COUNTER_ACTIVE_TICKS,
                    COUNTER_RECOVERY_TICKS,
                ),
                Some((COUNTER_RANGE, 1)),
                (
                    COUNTER_DAMAGE_NUMERATOR,
                    COUNTER_DAMAGE_DENOMINATOR,
                    COUNTER_STAGGER_TICKS,
                ),
            ),
            (
                ActionKind::LightFollowUp,
                (
                    LIGHT_FOLLOW_UP_WINDUP_TICKS,
                    LIGHT_FOLLOW_UP_ACTIVE_TICKS,
                    LIGHT_FOLLOW_UP_RECOVERY_TICKS,
                ),
                Some((ATTACK_RANGE, usize::MAX)),
                (1, 1, PRIMARY_STAGGER_TICKS),
            ),
            (
                ActionKind::LightFinisher,
                (
                    LIGHT_FINISHER_WINDUP_TICKS,
                    LIGHT_FINISHER_ACTIVE_TICKS,
                    LIGHT_FINISHER_RECOVERY_TICKS,
                ),
                Some((LIGHT_FINISHER_RANGE, usize::MAX)),
                (
                    LIGHT_FINISHER_DAMAGE_NUMERATOR,
                    LIGHT_FINISHER_DAMAGE_DENOMINATOR,
                    LIGHT_FINISHER_STAGGER_TICKS,
                ),
            ),
            (
                ActionKind::HeavyFinisher,
                (
                    HEAVY_FINISHER_WINDUP_TICKS,
                    HEAVY_FINISHER_ACTIVE_TICKS,
                    HEAVY_FINISHER_RECOVERY_TICKS,
                ),
                Some((HEAVY_FINISHER_RANGE, 1)),
                (
                    HEAVY_FINISHER_DAMAGE_NUMERATOR,
                    HEAVY_FINISHER_DAMAGE_DENOMINATOR,
                    HEAVY_FINISHER_STAGGER_TICKS,
                ),
            ),
            (
                ActionKind::Shoot,
                (SHOOT_WINDUP_TICKS, SHOOT_ACTIVE_TICKS, SHOOT_RECOVERY_TICKS),
                None,
                (0, 1, 0),
            ),
        ];
        for (kind, timings, strike, damage) in expected {
            let action = content().action(kind);
            assert_eq!(
                (
                    action.windup_ticks,
                    action.active_ticks,
                    action.recovery_ticks
                ),
                timings,
                "{kind:?}"
            );
            assert_eq!(
                action
                    .strike
                    .map(|strike| (strike.reach, strike.max_targets)),
                strike,
                "{kind:?}"
            );
            assert_eq!(
                (
                    action.damage_numerator,
                    action.damage_denominator,
                    action.stagger_ticks
                ),
                damage,
                "{kind:?}"
            );
        }
        let monster = *brute();
        assert_eq!(
            (
                monster.health,
                monster.strike.reach,
                monster.damage,
                monster.experience_reward
            ),
            (
                100,
                MONSTER_ATTACK_RANGE,
                MONSTER_ATTACK_DAMAGE,
                MONSTER_EXPERIENCE_REWARD
            )
        );
        assert_eq!(
            (
                monster.windup_ticks,
                monster.active_ticks,
                monster.recovery_ticks
            ),
            (
                MONSTER_ATTACK_WINDUP_TICKS,
                MONSTER_ATTACK_ACTIVE_TICKS,
                MONSTER_ATTACK_RECOVERY_TICKS
            )
        );
        assert_eq!(monster.strike.guard_cost, MONSTER_CLAW_GUARD_COST);
        assert!(!monster.strike.frontal && monster.strike.blockable);
        assert_eq!(content().counter_window_ticks, COUNTER_WINDOW_TICKS);
        assert_eq!(content().combos.len(), 4);
        assert_eq!(content().max_stagger_ticks(), HEAVY_FINISHER_STAGGER_TICKS);

        let guard = content().guard;
        assert_eq!(
            (
                guard.raise_ticks,
                guard.max_points,
                guard.regen_per_tick,
                guard.block_reaction_ticks,
                guard.break_ticks
            ),
            (
                GUARD_RAISE_TICKS,
                MAX_GUARD_POINTS,
                GUARD_REGEN_PER_TICK,
                GUARD_BLOCK_REACTION_TICKS,
                GUARD_BREAK_TICKS
            )
        );
        let bow = content().bow;
        assert_eq!(
            (bow.min_draw_ticks, bow.full_draw_ticks),
            (BOW_MIN_DRAW_TICKS, BOW_FULL_DRAW_TICKS)
        );
        assert_eq!(
            (
                i32::from(bow.arrow_min_speed),
                i32::from(bow.arrow_full_speed)
            ),
            (ARROW_MIN_SPEED, ARROW_FULL_SPEED)
        );
        assert_eq!(
            (
                bow.arrow_min_damage,
                bow.arrow_full_damage,
                bow.arrow_lifetime_ticks,
                bow.arrow_stagger_ticks,
                usize::from(bow.max_live_arrows)
            ),
            (
                ARROW_MIN_DAMAGE,
                ARROW_FULL_DAMAGE,
                ARROW_LIFETIME_TICKS,
                ARROW_STAGGER_TICKS,
                MAX_LIVE_ARROWS
            )
        );
        let progression = content().progression;
        assert_eq!(
            (
                progression.experience_per_level,
                progression.base_max_health,
                progression.max_health_per_level,
                progression.base_attack_damage,
                progression.attack_damage_per_level
            ),
            (
                EXPERIENCE_PER_LEVEL,
                BASE_MAX_HEALTH,
                MAX_HEALTH_PER_LEVEL,
                BASE_ATTACK_DAMAGE,
                ATTACK_DAMAGE_PER_LEVEL
            )
        );
        assert_eq!(content().loot.monster_gold, GROUND_LOOT_GOLD_AMOUNT);
        // Every combat room's encounter pays the previous chest gold.
        for index in 0..content().room_encounters.len() {
            let encounter = content().room_encounter(index);
            assert_eq!(
                content().reward(encounter.reward).gold,
                CHEST_GOLD_AMOUNT,
                "{}",
                content().id(encounter.id)
            );
        }
    }

    /// Room whose generated monster the built-in content makes a skirmisher (#105).
    const SKIRMISHER_ROOM: RoomId = 4;

    fn skirmisher() -> &'static MonsterDefinition {
        content().monster(definition_index(SKIRMISHER))
    }

    /// Player 1 at the centre of the skirmisher's room, activated, with the generated
    /// skirmisher `offset` units along +x.
    fn skirmisher_room(offset: i32) -> (ArpgGame, u32) {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        let (center_x, center_z) = game
            .rooms
            .iter()
            .find(|room| room.id == SKIRMISHER_ROOM)
            .unwrap()
            .center();
        game.player_spawns[0] = Vec3i::new(center_x, PLAYER_Y, center_z);
        game.add_player(1).unwrap();
        game.reconcile_encounters().unwrap();
        let monster = game
            .monsters
            .iter_mut()
            .find(|monster| monster.room_id == SKIRMISHER_ROOM)
            .unwrap();
        assert_eq!(content().id(monster.definition().id), SKIRMISHER);
        monster.position = Vec3i::new(center_x + offset, PLAYER_Y, center_z).into();
        let id = monster.id;
        (game, id)
    }

    #[test]
    fn combat_rooms_spawn_the_monster_their_content_assigns() {
        let skirmisher = skirmisher();
        assert_ne!(
            (
                skirmisher.health,
                skirmisher.strike.reach,
                skirmisher.pursuit_speed
            ),
            (brute().health, brute().strike.reach, brute().pursuit_speed)
        );
        for seed in [0, 42, 0xA420_0916, u32::MAX] {
            let snapshot = ArpgGame::new_with_seed(seed).unwrap().snapshot().unwrap();
            let rooms = snapshot
                .monsters
                .iter()
                .map(|monster| (monster.room_id, monster.definition.as_str()))
                .collect::<Vec<_>>();
            assert_eq!(
                rooms,
                [
                    (2, BRUTE),
                    (3, ARCHER),
                    (4, SKIRMISHER),
                    (5, BRUISER),
                    (6, DEFENDER)
                ],
                "seed {seed}"
            );
            for monster in &snapshot.monsters {
                let definition = content().monster(definition_index(&monster.definition));
                assert_eq!(monster.health, definition.health);
                assert_eq!(monster.max_health, definition.health);
            }
        }
    }

    #[test]
    fn a_skirmisher_attacks_with_its_own_timing_reach_and_damage() {
        let definition = skirmisher();
        let (mut game, id) = skirmisher_room(100);

        game.advance_tick().unwrap();

        let telegraph = game.snapshot().unwrap();
        let monster = telegraph.monsters.iter().find(|m| m.id == id).unwrap();
        let action = monster.action.unwrap();
        assert_eq!(action.phase, ActionPhase::Windup);
        assert_eq!(action.ticks_remaining, definition.windup_ticks);
        assert_eq!(i64::from(action.range), definition.strike.reach);
        assert_eq!(telegraph.players[0].health, BASE_MAX_HEALTH);

        for _ in 0..definition.windup_ticks {
            game.advance_tick().unwrap();
        }
        let impact = game.snapshot().unwrap();
        assert_eq!(
            impact.players[0].health,
            BASE_MAX_HEALTH - definition.damage
        );
        assert!(impact.strike_events.iter().any(|event| {
            event.definition == "monster.lunge"
                && event.source == StrikeSource::Monster(id)
                && event.result
                    == StrikeResult::Hit {
                        damage: definition.damage,
                        defeated: false,
                    }
        }));
        let monster = impact.monsters.iter().find(|m| m.id == id).unwrap();
        assert_eq!(monster.action.unwrap().phase, ActionPhase::Active);
    }

    #[test]
    fn a_skirmisher_blocked_spends_its_own_guard_cost() {
        let definition = skirmisher();
        let (mut game, _) = skirmisher_room(100);
        game.apply_command(
            PlayerCommand::new(1, 1, ArpgCommand::SetGuard { raised: true }).unwrap(),
        )
        .unwrap();
        for _ in 0..=definition.windup_ticks {
            game.advance_tick().unwrap();
        }
        let player = &game.snapshot().unwrap().players[0];
        assert_eq!(player.health, BASE_MAX_HEALTH);
        assert_eq!(
            player.guard_points,
            MAX_GUARD_POINTS - definition.strike.guard_cost
        );
    }

    #[test]
    fn defeating_a_skirmisher_awards_its_own_experience_and_drop() {
        let (mut game, id) = skirmisher_room(100);
        game.monsters
            .iter_mut()
            .find(|monster| monster.id == id)
            .unwrap()
            .health = 1;

        run_action(&mut game, 1, 1, ArpgCommand::PrimaryAttack { aim: None });

        let snapshot = game.snapshot().unwrap();
        assert!(!snapshot.monsters.iter().find(|m| m.id == id).unwrap().alive);
        assert_eq!(
            snapshot.players[0].experience,
            skirmisher().experience_reward
        );
        assert_ne!(skirmisher().experience_reward, brute().experience_reward);
        assert_eq!(snapshot.ground_loot.len(), 1);
        assert_eq!(snapshot.ground_loot[0].amount, content().loot.monster_gold);
    }

    #[test]
    fn a_skirmisher_outpaces_a_brute_but_never_its_own_speed() {
        let (mut game, x, z) = strike_arena();
        place_monster_of(&mut game, 1, SKIRMISHER, x + 700, z + 200);
        let start = monster_position(&game, 1);
        let mut previous = start;
        let mut fastest = 0.0_f64;
        let arrived = (1..=400)
            .find(|_| {
                game.advance_tick().unwrap();
                let position = monster_position(&game, 1);
                fastest = fastest.max(xz_distance_sq(previous, position).sqrt());
                previous = position;
                game.monsters[0].action.is_some()
            })
            .expect("the skirmisher reaches its target");
        let reach = skirmisher().strike.reach;
        assert!(within_range(
            xz_distance_sq(previous, player_position(&game)),
            reach
        ));
        // Speeds are exact up to `f32` rounding of the per-tick motion.
        assert!(fastest <= f64::from(skirmisher().pursuit_speed) + 1.0e-3);
        assert!(
            fastest > f64::from(brute().pursuit_speed),
            "fastest step {fastest} after {arrived} ticks"
        );
    }

    #[test]
    fn mixed_definitions_in_one_room_each_close_to_their_own_reach() {
        let (mut game, x, z) = strike_arena();
        place_monster_of(&mut game, 1, BRUTE, x + 700, z + 200);
        place_monster_of(&mut game, 2, SKIRMISHER, x + 700, z - 200);
        let mut started = BTreeMap::new();
        for _ in 0..500 {
            game.advance_tick().unwrap();
            let player = player_position(&game);
            for monster in &game.monsters {
                if monster.action.is_some() {
                    started
                        .entry(monster.id)
                        .or_insert(xz_distance_sq(monster.position, player));
                }
            }
            if started.len() == 2 {
                break;
            }
        }
        let brute_reach = brute().strike.reach;
        let skirmisher_reach = skirmisher().strike.reach;
        assert!(within_range(started[&1], brute_reach), "{started:?}");
        assert!(within_range(started[&2], skirmisher_reach), "{started:?}");
    }

    #[test]
    fn a_save_mid_skirmisher_attack_continues_like_the_uninterrupted_game() {
        let (mut game, id) = skirmisher_room(100);
        for _ in 0..4 {
            game.advance_tick().unwrap();
        }
        assert!(
            game.monsters
                .iter()
                .any(|m| m.id == id && m.action.is_some())
        );

        let mut restored = ArpgGame::from_save_state(game.save_state().unwrap()).unwrap();
        assert_eq!(restored.snapshot().unwrap(), game.snapshot().unwrap());
        for _ in 0..60 {
            game.advance_tick().unwrap();
            restored.advance_tick().unwrap();
            assert_eq!(restored.snapshot().unwrap(), game.snapshot().unwrap());
        }
    }

    #[test]
    fn saved_monsters_are_validated_against_their_own_definition() {
        let (game, id) = skirmisher_room(100);
        let save = game.save_state().unwrap();
        let skirmisher = skirmisher();

        let mut over_health = save.clone();
        let saved = over_health
            .monsters
            .iter_mut()
            .find(|m| m.id == id)
            .unwrap();
        saved.health = skirmisher.health + 1;
        assert!(saved.health <= brute().health);
        assert!(ArpgGame::from_save_state(over_health).is_err());

        let mut long_windup = save;
        let saved = long_windup
            .monsters
            .iter_mut()
            .find(|m| m.id == id)
            .unwrap();
        saved.engagement = Engagement::Engaged {
            target_player_id: 1,
        };
        saved.action = Some(MonsterActionSaveState {
            phase: ActionPhase::Windup,
            ticks_remaining: skirmisher.windup_ticks + 1,
            target_player_id: 1,
        });
        assert!(skirmisher.windup_ticks < brute().windup_ticks);
        assert!(ArpgGame::from_save_state(long_windup).is_err());
    }

    #[test]
    fn a_save_during_arrow_stagger_restores() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        game.add_player(1).unwrap();
        game.monsters[0].stagger_ticks_remaining = ARROW_STAGGER_TICKS;
        assert!(ArpgGame::from_save_state(game.save_state().unwrap()).is_ok());
        game.monsters[0].stagger_ticks_remaining = content().max_stagger_ticks() + 1;
        assert!(ArpgGame::from_save_state(game.save_state().unwrap()).is_err());
    }

    #[test]
    fn games_of_one_bundle_share_its_content_and_the_last_one_frees_it() {
        let mut bundle = base_bundle().clone();
        bundle.rewards[0].gold = 40_000;
        let first = ArpgGame::new_with_content(bundle.clone(), 9).unwrap();
        let restored =
            ArpgGame::from_save_state_with_content(first.save_state().unwrap(), bundle.clone())
                .unwrap();
        assert!(
            Arc::ptr_eq(&first.content, &restored.content),
            "one resolution"
        );
        let weak = Arc::downgrade(&first.content);
        drop(first);
        assert!(weak.upgrade().is_some(), "the restored game still owns it");
        drop(restored);
        assert!(weak.upgrade().is_none(), "freed with the last game");
        assert!(content::registered(&bundle.revision()).is_none());

        // Built-in games all run the one built-in content.
        let built_in = ArpgGame::new_with_seed(9).unwrap();
        let as_custom = ArpgGame::new_with_content(base_bundle().clone(), 9).unwrap();
        assert!(Arc::ptr_eq(&built_in.content, content()));
        assert!(Arc::ptr_eq(&as_custom.content, content()));
    }

    #[test]
    fn failed_restores_keep_no_content() {
        // A valid bundle that never ran a game, and a save that matches its revision but
        // whose room set is broken, so only the restore itself fails.
        let mut bundle = base_bundle().clone();
        bundle.rewards[0].gold = 40_001;
        let mut save = ArpgGame::new_with_seed(9).unwrap().save_state().unwrap();
        save.content_revision = bundle.revision();
        save.rooms.pop();
        assert!(ArpgGame::from_save_state_with_content(save, bundle.clone()).is_err());
        assert!(content::registered(&bundle.revision()).is_none());

        // A save from another bundle is refused before that bundle is resolved.
        let mut game = ArpgGame::new_with_seed(7).unwrap();
        game.add_player(1).unwrap();
        let mut other = base_bundle().clone();
        other.rewards[0].gold = 31_337;
        let revision = other.revision();
        let Err(error) = ArpgGame::from_save_state_with_content(game.save_state().unwrap(), other)
        else {
            panic!("a built-in save does not restore with another bundle");
        };
        assert!(error.message().contains("content revision"), "{error}");
        assert!(content::registered(&revision).is_none());
    }

    #[test]
    fn saves_from_another_content_revision_are_rejected() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        game.add_player(1).unwrap();
        let mut save = game.save_state().unwrap();
        assert_eq!(save.content_revision, content_revision());
        assert_eq!(
            game.snapshot().unwrap().content_revision,
            content_revision()
        );
        save.content_revision = "0000000000000000".into();
        assert!(
            ArpgGame::from_save_state(save)
                .unwrap_err()
                .message()
                .contains("content revision")
        );
    }

    #[test]
    fn generated_rooms_are_large_enough_for_arpg_combat() {
        for seed in 0..64 {
            let dungeon = generate_dungeon(content(), seed);
            assert!(
                dungeon.rooms.iter().all(|room| {
                    room.max_x - room.min_x >= 1_400 && room.max_z - room.min_z >= 1_400
                }),
                "seed {seed} generated a cramped room"
            );
        }
    }

    #[test]
    fn procedural_dungeon_is_seeded_and_deterministic() {
        let first = generate_dungeon(content(), 42);
        let replay = generate_dungeon(content(), 42);
        let different_seed = generate_dungeon(content(), 43);

        assert_eq!(first.rooms, replay.rooms);
        assert_eq!(first.doors, replay.doors);
        assert_eq!(first.static_colliders, replay.static_colliders);
        assert_eq!(first.player_spawns, replay.player_spawns);
        assert_eq!(
            monster_layout(&first.monsters),
            monster_layout(&replay.monsters)
        );
        assert_ne!(first.rooms, different_seed.rooms);
        assert!(
            first.static_colliders.len() > 4,
            "expected generated internal room walls"
        );
    }

    #[test]
    fn generated_room_graph_is_connected_symmetric_and_backed_by_doors() {
        let dungeon = generate_dungeon(content(), 42);
        assert_eq!(dungeon.rooms.len(), 6);
        assert_eq!(dungeon.doors.len(), 7);

        let mut seen = BTreeSet::new();
        let mut pending = vec![dungeon.rooms[0].id];
        while let Some(room_id) = pending.pop() {
            if !seen.insert(room_id) {
                continue;
            }
            let room = dungeon
                .rooms
                .iter()
                .find(|room| room.id == room_id)
                .expect("neighbor must reference an existing room");
            for &neighbor_id in &room.neighbors {
                let neighbor = dungeon
                    .rooms
                    .iter()
                    .find(|candidate| candidate.id == neighbor_id)
                    .expect("neighbor must reference an existing room");
                assert!(
                    neighbor.neighbors.contains(&room.id),
                    "room {} -> {} connection must be symmetric",
                    room.id,
                    neighbor_id
                );
                assert!(dungeon.doors.iter().any(|door| {
                    (door.room_a == room.id && door.room_b == neighbor_id)
                        || (door.room_b == room.id && door.room_a == neighbor_id)
                }));
                pending.push(neighbor_id);
            }
        }
        assert_eq!(seen.len(), dungeon.rooms.len());
        assert!(dungeon.doors.iter().all(|door| !door.locked));
    }

    #[test]
    fn generated_spawns_stay_inside_their_rooms() {
        let dungeon = generate_dungeon(content(), 0xC0FF_EE11);
        let start_room = dungeon
            .rooms
            .iter()
            .find(|room| room.kind == RoomKind::Start)
            .unwrap();
        for spawn in dungeon.player_spawns {
            assert!(start_room.contains_xz(spawn.into()));
        }
        for monster in &dungeon.monsters {
            let room = dungeon
                .rooms
                .iter()
                .find(|room| room.id == monster.room_id)
                .unwrap();
            assert_eq!(room.kind, RoomKind::Combat);
            assert!(room.contains_xz_with_margin(monster.position, 0));
        }
    }

    #[test]
    fn snapshot_carries_run_seed_room_door_and_progression_semantics_for_replay() {
        let mut game = ArpgGame::new_with_seed(0xDEAD_BEEF).unwrap();
        game.add_player(1).unwrap();
        let snapshot = game.snapshot().unwrap();
        let generated = generate_dungeon(content(), snapshot.run_seed);
        assert_eq!(snapshot.schema_version, 23);
        assert_eq!(snapshot.run_seed, 0xDEAD_BEEF);
        assert_eq!(game.run_seed(), snapshot.run_seed);
        assert_eq!(snapshot.rooms, generated.rooms);
        assert_eq!(snapshot.doors, generated.doors);
        assert_eq!(snapshot.static_colliders, generated.static_colliders);
        assert_eq!(snapshot.monsters.len(), generated.monsters.len());
        assert!(snapshot.ground_loot.is_empty());
        assert!(snapshot.monsters.iter().all(|monster| monster.room_id > 1));
        assert_eq!(snapshot.players[0].level, 1);
        assert_eq!(snapshot.players[0].experience, 0);
        assert_eq!(snapshot.players[0].attack_damage, BASE_ATTACK_DAMAGE);
    }

    #[test]
    fn entering_combat_room_locks_connected_doors_until_encounter_is_cleared() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        let room_id = 2;
        let (center_x, center_z) = game
            .rooms
            .iter()
            .find(|room| room.id == room_id)
            .unwrap()
            .center();
        game.player_spawns[0] = Vec3i::new(center_x, PLAYER_Y, center_z);
        game.add_player(1).unwrap();
        game.reconcile_encounters().unwrap();

        assert_eq!(room_state(&game, room_id), RoomEncounterState::Active);
        let connected_door_ids = game
            .doors
            .iter()
            .filter(|door| door.room_a == room_id || door.room_b == room_id)
            .map(|door| door.id)
            .collect::<Vec<_>>();
        assert!(!connected_door_ids.is_empty());
        for door_id in &connected_door_ids {
            let door = game.doors.iter().find(|door| door.id == *door_id).unwrap();
            assert!(door.locked);
            assert!(game.physics.has_fixed(*door_id));
        }

        let monster = game
            .monsters
            .iter_mut()
            .find(|monster| monster.room_id == room_id)
            .unwrap();
        monster.position = Vec3i::new(center_x + 100, PLAYER_Y, center_z).into();
        for sequence in 1..=4 {
            run_action(
                &mut game,
                1,
                sequence,
                ArpgCommand::PrimaryAttack { aim: None },
            );
        }

        assert_eq!(room_state(&game, room_id), RoomEncounterState::Cleared);
        for door_id in connected_door_ids {
            let door = game.doors.iter().find(|door| door.id == door_id).unwrap();
            assert!(!door.locked);
            assert!(!game.physics.has_fixed(door_id));
        }
        assert_eq!(
            game.players.get(&1).unwrap().experience,
            MONSTER_EXPERIENCE_REWARD
        );
        assert!(
            game.snapshot()
                .unwrap()
                .static_colliders
                .iter()
                .all(|collider| collider.kind != StaticColliderKind::Door)
        );
    }

    #[test]
    fn dormant_room_monsters_cannot_be_attacked_before_entry() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        game.add_player(1).unwrap();
        let player_position = Vec3i::rounded(game.players[&1].position);
        let monster = game
            .monsters
            .iter_mut()
            .find(|monster| monster.room_id == 2)
            .unwrap();
        monster.position = Vec3i::new(player_position.x + 100, PLAYER_Y, player_position.z).into();

        for sequence in 1..=4 {
            run_action(
                &mut game,
                1,
                sequence,
                ArpgCommand::PrimaryAttack { aim: None },
            );
        }
        assert_eq!(
            game.monsters
                .iter()
                .find(|monster| monster.room_id == 2)
                .unwrap()
                .health,
            100
        );
        assert_eq!(game.players.get(&1).unwrap().experience, 0);
        assert_eq!(room_state(&game, 2), RoomEncounterState::Dormant);
    }

    #[test]
    fn active_room_doors_are_projected_as_static_door_colliders() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        let (center_x, center_z) = game
            .rooms
            .iter()
            .find(|room| room.id == 2)
            .unwrap()
            .center();
        game.player_spawns[0] = Vec3i::new(center_x, PLAYER_Y, center_z);
        game.add_player(1).unwrap();
        game.reconcile_encounters().unwrap();
        let snapshot = game.snapshot().unwrap();
        let locked_count = snapshot.doors.iter().filter(|door| door.locked).count();
        let projected_count = snapshot
            .static_colliders
            .iter()
            .filter(|collider| collider.kind == StaticColliderKind::Door)
            .count();
        assert_eq!(locked_count, projected_count);
        assert!(locked_count > 0);
    }

    #[test]
    fn procedural_dungeon_keeps_outer_boundary_stable() {
        let first = generate_dungeon(content(), 1).static_colliders;
        let second = generate_dungeon(content(), 2).static_colliders;
        assert_eq!(&first[..4], &second[..4]);
        assert_eq!(first[0].position, [0, PLAYER_Y, -ARENA_HALF_DEPTH]);
        assert_eq!(first[1].position, [0, PLAYER_Y, ARENA_HALF_DEPTH]);
        assert_eq!(first[2].position, [-ARENA_HALF_WIDTH, PLAYER_Y, 0]);
        assert_eq!(first[3].position, [ARENA_HALF_WIDTH, PLAYER_Y, 0]);
    }

    #[test]
    fn wasd_style_movement_is_driven_through_physics() {
        let mut game = ArpgGame::new().unwrap();
        game.add_player(1).unwrap();
        let before = game.snapshot().unwrap().players[0].position;
        game.apply_command(
            PlayerCommand::new(1, 1, ArpgCommand::SetMovement { x: 1, z: 0 }).unwrap(),
        )
        .unwrap();
        for _ in 0..10 {
            game.advance_tick().unwrap();
        }
        let after = game.snapshot().unwrap().players[0].position;
        assert!(after[0] > before[0]);
        assert_eq!(after[2], before[2]);
    }

    #[test]
    fn fixed_dungeon_boundary_stops_the_player() {
        let mut game = ArpgGame::new().unwrap();
        game.add_player(1).unwrap();
        game.apply_command(
            PlayerCommand::new(1, 1, ArpgCommand::SetMovement { x: -1, z: 0 }).unwrap(),
        )
        .unwrap();
        for _ in 0..800 {
            game.advance_tick().unwrap();
        }
        let x = game.snapshot().unwrap().players[0].position[0];
        assert!(x >= -2_945, "player crossed the west wall: {x}");
    }

    #[test]
    fn stale_commands_fail_closed() {
        let mut game = ArpgGame::new().unwrap();
        game.add_player(1).unwrap();
        game.apply_command(
            PlayerCommand::new(1, 5, ArpgCommand::SetMovement { x: 1, z: 0 }).unwrap(),
        )
        .unwrap();
        let error = game
            .apply_command(
                PlayerCommand::new(1, 5, ArpgCommand::SetMovement { x: -1, z: 0 }).unwrap(),
            )
            .unwrap_err();
        assert_eq!(error.message(), "command sequence is stale");
    }

    fn room_state(game: &ArpgGame, room_id: RoomId) -> RoomEncounterState {
        game.rooms
            .iter()
            .find(|room| room.id == room_id)
            .unwrap()
            .encounter_state
    }

    fn monster_layout(monsters: &[MonsterState]) -> Vec<(u32, RoomId, [i32; 3])> {
        monsters
            .iter()
            .map(|monster| {
                (
                    monster.id,
                    monster.room_id,
                    vec_to_array(Vec3i::rounded(monster.position)),
                )
            })
            .collect()
    }
    #[test]
    fn save_state_round_trip_preserves_authoritative_state_and_continuation() {
        let mut game = ArpgGame::new_with_seed(0x51A7_E123).unwrap();
        game.add_player(1).unwrap();
        game.apply_command(
            PlayerCommand::new(1, 1, ArpgCommand::SetMovement { x: 1, z: -1 }).unwrap(),
        )
        .unwrap();
        for _ in 0..17 {
            game.advance_tick().unwrap();
        }
        game.apply_command(
            PlayerCommand::new(1, 2, ArpgCommand::PrimaryAttack { aim: None }).unwrap(),
        )
        .unwrap();
        for _ in 0..3 {
            game.advance_tick().unwrap();
        }

        let save = game.save_state().unwrap();
        let mut restored = ArpgGame::from_save_state(save).unwrap();
        assert_eq!(restored.snapshot().unwrap(), game.snapshot().unwrap());

        let next = PlayerCommand::new(1, 3, ArpgCommand::SetMovement { x: 0, z: 1 }).unwrap();
        game.apply_command(next.clone()).unwrap();
        restored.apply_command(next).unwrap();
        for _ in 0..25 {
            game.advance_tick().unwrap();
            restored.advance_tick().unwrap();
        }
        assert_eq!(restored.snapshot().unwrap(), game.snapshot().unwrap());
    }

    #[test]
    fn save_state_preserves_velocity_while_accelerating() {
        let mut game = ArpgGame::new_with_seed(0x51A7_E123).unwrap();
        game.add_player(1).unwrap();
        game.apply_command(
            PlayerCommand::new(1, 1, ArpgCommand::SetMovement { x: 1, z: 0 }).unwrap(),
        )
        .unwrap();
        game.advance_tick().unwrap();

        let save = game.save_state().unwrap();
        assert_ne!(save.players[0].velocity, [0.0; 3]);
        assert_ne!(save.players[0].velocity, [PLAYER_SPEED as f32, 0.0, 0.0]);
        let mut restored = ArpgGame::from_save_state(save.clone()).unwrap();
        for _ in 0..4 {
            game.advance_tick().unwrap();
            restored.advance_tick().unwrap();
            assert_eq!(restored.snapshot().unwrap(), game.snapshot().unwrap());
        }

        let mut tampered = save;
        tampered.players[0].velocity = [PLAYER_SPEED as f32 + 1.0, 0.0, 0.0];
        assert_eq!(
            ArpgGame::from_save_state(tampered).unwrap_err().message(),
            "saved player velocity is out of range"
        );
        for velocity in [f32::MIN, f32::NAN, f32::INFINITY] {
            let mut minimum = game.save_state().unwrap();
            minimum.players[0].velocity = [velocity, 0.0, 0.0];
            assert_eq!(
                ArpgGame::from_save_state(minimum).unwrap_err().message(),
                "saved player velocity is out of range"
            );
        }
    }

    #[test]
    fn save_state_rejects_positions_outside_the_generated_dungeon() {
        let mut game = ArpgGame::new_with_seed(0x51A7_E123).unwrap();
        game.add_player(1).unwrap();
        let save = game.save_state().unwrap();

        let mut player = save.clone();
        player.players[0].position = [i32::MAX as f32, PLAYER_Y as f32, 0.0];
        assert_eq!(
            ArpgGame::from_save_state(player).unwrap_err().message(),
            "saved player position is outside the dungeon"
        );
        for invalid in [f32::NAN, f32::INFINITY] {
            let mut player = save.clone();
            player.players[0].position[0] = invalid;
            assert_eq!(
                ArpgGame::from_save_state(player).unwrap_err().message(),
                "saved player position is outside the dungeon"
            );
            let mut monster = save.clone();
            monster.monsters[0].position[2] = invalid;
            assert_eq!(
                ArpgGame::from_save_state(monster).unwrap_err().message(),
                "saved monster position is outside its room"
            );
        }

        let mut monster = save.clone();
        monster.monsters[0].position = [i32::MIN as f32, PLAYER_Y as f32, i32::MIN as f32];
        assert_eq!(
            ArpgGame::from_save_state(monster).unwrap_err().message(),
            "saved monster position is outside its room"
        );

        let wall = game
            .static_colliders
            .iter()
            .find(|collider| collider.kind == StaticColliderKind::Wall)
            .unwrap()
            .position;
        let mut inside_wall = save.clone();
        inside_wall.players[0].position = [wall[0] as f32, PLAYER_Y as f32, wall[2] as f32];
        assert_eq!(
            ArpgGame::from_save_state(inside_wall)
                .unwrap_err()
                .message(),
            "saved player position is outside the dungeon"
        );

        let mut exhausted = save.clone();
        exhausted.players[0].last_sequence = u32::MAX;
        assert_eq!(
            ArpgGame::from_save_state(exhausted).unwrap_err().message(),
            "saved command sequence leaves no valid next command"
        );

        let mut cleared = save.clone();
        let monster_room = cleared.monsters[0].room_id;
        cleared
            .rooms
            .iter_mut()
            .find(|room| room.id == monster_room)
            .unwrap()
            .encounter_state = RoomEncounterState::Cleared;
        assert_eq!(
            ArpgGame::from_save_state(cleared).unwrap_err().message(),
            "saved cleared room still contains a living monster"
        );

        let mut loot = save;
        loot.next_ground_loot_id = GROUND_LOOT_ID_BASE + 1;
        loot.ground_loot.push(GroundLootSaveState {
            id: GROUND_LOOT_ID_BASE,
            position: [0.0, f32::MAX, 0.0],
            kind: LootKind::Gold,
            amount: GROUND_LOOT_GOLD_AMOUNT,
        });
        assert_eq!(
            ArpgGame::from_save_state(loot).unwrap_err().message(),
            "save contains invalid ground loot"
        );
    }

    #[test]
    fn save_state_restores_command_sequence_fence() {
        let mut game = ArpgGame::new_with_seed(17).unwrap();
        game.add_player(1).unwrap();
        game.apply_command(
            PlayerCommand::new(1, 9, ArpgCommand::PrimaryAttack { aim: None }).unwrap(),
        )
        .unwrap();

        let mut restored = ArpgGame::from_save_state(game.save_state().unwrap()).unwrap();
        let stale = restored
            .apply_command(PlayerCommand::new(1, 9, ArpgCommand::Interact).unwrap())
            .unwrap_err();
        assert_eq!(stale.message(), "command sequence is stale");
        restored
            .apply_command(PlayerCommand::new(1, 10, ArpgCommand::Interact).unwrap())
            .unwrap();
    }

    #[test]
    fn saves_from_the_integer_physics_schema_are_refused() {
        let mut game = ArpgGame::new_with_seed(91).unwrap();
        game.add_player(1).unwrap();
        // A schema 12 save: integer positions and velocities, as the old physics wrote them.
        let mut json = serde_json::to_value(game.save_state().unwrap()).unwrap();
        json["schemaVersion"] = 12.into();
        json["players"][0]["position"] = serde_json::json!([-2000, 50, -1000]);
        json["players"][0]["velocity"] = serde_json::json!([0, 0, 0]);
        let save: ArpgSaveState = serde_json::from_value(json).unwrap();
        assert_eq!(
            ArpgGame::from_save_state(save).unwrap_err().message(),
            "unsupported ARPG save schema version 12; expected 17"
        );
    }

    #[test]
    fn saves_from_the_whole_unit_rules_schema_are_refused() {
        let mut game = ArpgGame::new_with_seed(91).unwrap();
        game.add_player(1).unwrap();
        // A schema 13 save: whole-unit arrows, ground loot and monster posts.
        let mut json = serde_json::to_value(game.save_state().unwrap()).unwrap();
        json["schemaVersion"] = 13.into();
        json["monsters"][0]["post"] = serde_json::json!([0, 50, 0]);
        let save: ArpgSaveState = serde_json::from_value(json).unwrap();
        assert_eq!(
            ArpgGame::from_save_state(save).unwrap_err().message(),
            "unsupported ARPG save schema version 13; expected 17"
        );
    }

    #[test]
    fn save_state_rejects_unknown_schema_and_tampered_seed_state() {
        let mut game = ArpgGame::new_with_seed(91).unwrap();
        game.add_player(1).unwrap();
        let mut save = game.save_state().unwrap();

        save.schema_version += 1;
        assert!(
            ArpgGame::from_save_state(save)
                .unwrap_err()
                .message()
                .contains("unsupported ARPG save schema version")
        );

        let mut save = game.save_state().unwrap();
        save.rules_version += 1;
        assert!(
            ArpgGame::from_save_state(save)
                .unwrap_err()
                .message()
                .contains("unsupported ARPG save rules version")
        );

        // Saves from before directional strike volumes resolve melee differently.
        let mut save = game.save_state().unwrap();
        save.rules_version = 1;
        assert!(
            ArpgGame::from_save_state(save)
                .unwrap_err()
                .message()
                .contains("unsupported ARPG save rules version")
        );

        let mut save = game.save_state().unwrap();
        save.rooms.pop();
        assert_eq!(
            ArpgGame::from_save_state(save).unwrap_err().message(),
            "save room set does not match generated dungeon"
        );
    }

    fn monster_position(game: &ArpgGame, id: u32) -> Vector {
        game.monsters
            .iter()
            .find(|monster| monster.id == id)
            .unwrap()
            .position
    }

    fn player_position(game: &ArpgGame) -> Vector {
        game.players[&1].position
    }

    fn overlaps_xz(center: Vec3i, half: Vec3i, other: Vec3i, other_half: Vec3i) -> bool {
        (center.x - other.x).abs() < half.x + other_half.x
            && (center.z - other.z).abs() < half.z + other_half.z
    }

    /// Penetration deeper than this counts as overlap; Rapier keeps bodies touching at its
    /// contact offset.
    const OVERLAP_TOLERANCE: f32 = 0.01;

    /// Whether an upright character body of `half_extents` (a capsule, round on the plane)
    /// at `center` penetrates the box footprint centred on `other`.
    fn body_overlaps_box(
        center: Vector,
        half_extents: Vec3i,
        other: Vec3i,
        other_half: Vec3i,
    ) -> bool {
        let gap = |own: f32, at: i32, half: i32| ((own - at as f32).abs() - half as f32).max(0.0);
        let (dx, dz) = (
            gap(center.x, other.x, other_half.x),
            gap(center.z, other.z, other_half.z),
        );
        (dx * dx + dz * dz).sqrt() < half_extents.x as f32 - OVERLAP_TOLERANCE
    }

    /// Whether two upright character bodies penetrate each other on the plane.
    fn bodies_overlap(center: Vector, half: Vec3i, other: Vector, other_half: Vec3i) -> bool {
        let (dx, dz) = (center.x - other.x, center.z - other.z);
        (dx * dx + dz * dz).sqrt() < (half.x + other_half.x) as f32 - OVERLAP_TOLERANCE
    }

    /// Ticks until monster `id` starts a wind-up, or `None` within `limit`.
    fn ticks_until_windup(game: &mut ArpgGame, id: u32, limit: usize) -> Option<usize> {
        (1..=limit).find(|_| {
            game.advance_tick().unwrap();
            game.monsters
                .iter()
                .find(|monster| monster.id == id)
                .unwrap()
                .action
                .is_some_and(|action| action.phase == ActionPhase::Windup)
        })
    }

    #[test]
    fn monster_pursues_across_its_room_and_attacks_on_arrival() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 700, z + 200);
        let start = monster_position(&game, 1);
        let reach = brute().strike.reach;

        let arrived =
            ticks_until_windup(&mut game, 1, 400).expect("the monster reaches its target");

        let position = monster_position(&game, 1);
        assert!(within_range(
            xz_distance_sq(position, player_position(&game)),
            reach
        ));
        let travelled = xz_distance_sq(start, position).sqrt();
        // Never faster than its content speed.
        assert!(travelled <= f64::from(brute().pursuit_speed) * arrived as f64 + 1.0e-3);
        assert!(game.navigation_expansions() > 0);
    }

    #[test]
    fn pursuit_goes_around_a_wall_without_entering_it() {
        let (mut game, x, z) = strike_arena();
        // A wall between player and monster; the gaps at its ends are wider than a body.
        let wall_half = Vec3i::new(20, 50, 220);
        place_blocker(&mut game, 0, x + 300, z, wall_half);
        place_monster(&mut game, 1, x + 520, z);
        let wall = Vec3i::new(x + 300, PLAYER_Y, z);

        let mut rounded_the_wall = false;
        let arrived = (1..=600).find(|_| {
            game.advance_tick().unwrap();
            let position = monster_position(&game, 1);
            assert!(!body_overlaps_box(
                game.monsters[0].position,
                MONSTER_BODY_HALF_EXTENTS,
                wall,
                wall_half
            ));
            rounded_the_wall |= (position.z - z as f32).abs() > wall_half.z as f32;
            game.monsters[0].action.is_some()
        });

        assert!(arrived.is_some(), "the monster finds a way around");
        assert!(rounded_the_wall);
        assert!(
            !game
                .strike_obstructed(monster_position(&game, 1), player_position(&game))
                .unwrap()
        );
    }

    #[test]
    fn a_player_hugging_a_pillar_is_still_reached_and_attacked() {
        let (mut game, x, z) = strike_arena();
        // The player stands against a pillar's face, where no monster body fits.
        place_blocker(&mut game, 0, x - 60, z, Vec3i::new(20, 50, 60));
        place_monster(&mut game, 1, x + 600, z + 250);

        assert!(ticks_until_windup(&mut game, 1, 400).is_some());
        assert!(
            !game
                .strike_obstructed(monster_position(&game, 1), player_position(&game))
                .unwrap()
        );
    }

    #[test]
    fn a_slow_pursuer_still_moves_on_a_diagonal() {
        let grid = RoomGrid::new(
            navigation::Rect {
                min_x: 0,
                max_x: 400,
                min_z: 0,
                max_z: 400,
            },
            &[],
            30,
        );
        let path = [(310, 310)];

        let velocity = pursuit_velocity(&grid, Vec3i::new(100, PLAYER_Y, 120).into(), &path, 1);

        // Exactly toward the point at its full speed of one unit per tick (integer
        // velocities once truncated this to a single axis step).
        let (dx, dz) = (210.0_f64, 190.0_f64);
        let length = planar_length(dx, dz);
        assert!(
            (f64::from(velocity.x) - dx / length).abs() < 1.0e-6,
            "{velocity:?}"
        );
        assert!(
            (f64::from(velocity.z) - dz / length).abs() < 1.0e-6,
            "{velocity:?}"
        );
        assert_eq!(velocity.y, 0.0);
    }

    #[test]
    fn an_attackable_player_is_engaged_before_a_nearer_blocked_one() {
        let (mut game, x, z) = strike_arena();
        // Player 1 is nearest but behind a pillar; player 2 is in reach with a clear line.
        game.player_spawns[1] = Vec3i::new(x + 50, PLAYER_Y, z + 160);
        game.add_player(2).unwrap();
        place_blocker(&mut game, 0, x + 50, z, Vec3i::new(10, 50, 40));
        place_monster(&mut game, 1, x + 120, z);
        let start = monster_position(&game, 1);

        ticks_until_windup(&mut game, 1, 3).expect("the clear target is attacked at once");

        assert_eq!(
            monster_position(&game, 1),
            start,
            "it does not walk off to the blocked one"
        );
        assert_eq!(game.monsters[0].action.unwrap().target_player_id, 2);
    }

    #[test]
    fn staggered_and_attacking_monsters_hold_their_position() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 600, z);
        game.monsters[0].stagger_ticks_remaining = 20;
        let start = monster_position(&game, 1);
        for _ in 0..15 {
            game.advance_tick().unwrap();
        }
        assert_eq!(monster_position(&game, 1), start);

        let mut game = strike_arena().0;
        place_monster(&mut game, 1, x + 100, z);
        ticks_until_windup(&mut game, 1, 5).expect("a monster in reach attacks at once");
        let striking_from = monster_position(&game, 1);
        while game.monsters[0].action.is_some() {
            game.advance_tick().unwrap();
            assert_eq!(monster_position(&game, 1), striking_from);
        }
    }

    #[test]
    fn an_unreachable_target_leaves_the_monster_idle() {
        let (mut game, x, z) = strike_arena();
        // Close the player in; the gaps are narrower than a monster body.
        for (index, (dx, dz, half_x, half_z)) in [
            (0, 120, 140, 20),
            (0, -120, 140, 20),
            (120, 0, 20, 140),
            (-120, 0, 20, 140),
        ]
        .into_iter()
        .enumerate()
        {
            place_blocker(
                &mut game,
                index as u64,
                x + dx,
                z + dz,
                Vec3i::new(half_x, 50, half_z),
            );
        }
        place_monster(&mut game, 1, x + 600, z + 300);
        let start = monster_position(&game, 1);

        for _ in 0..60 {
            game.advance_tick().unwrap();
        }

        assert_eq!(monster_position(&game, 1), start);
        assert!(game.monsters[0].action.is_none());
    }

    #[test]
    fn a_large_player_id_never_aliases_a_monster_body() {
        // Player 59001 shared BodyId(60001) with monster 1 under the former 60_000 base.
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        let (x, z) = game
            .rooms
            .iter()
            .find(|room| room.id == STRIKE_ROOM)
            .unwrap()
            .center();
        game.player_spawns[0] = Vec3i::new(x, PLAYER_Y, z);
        game.add_player(59_001).unwrap();
        game.reconcile_encounters().unwrap();
        game.monsters.clear();
        place_monster(&mut game, 1, x + 600, z);

        game.advance_tick().unwrap();

        assert_ne!(
            ArpgGame::monster_body_id(1),
            ArpgGame::player_body_id(59_001)
        );
        assert!(game.physics.has_character(ArpgGame::monster_body_id(1)));
        let player = Vec3i::rounded(game.players[&59_001].position);
        assert!(
            (player.x - x).abs() < 100,
            "the player was not moved to the monster"
        );
    }

    #[test]
    fn dead_monsters_lose_their_body() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 600, z);
        place_monster(&mut game, 2, x - 600, z);
        game.advance_tick().unwrap();
        assert!(game.physics.has_character(ArpgGame::monster_body_id(1)));

        game.monsters[0].health = 0;
        game.advance_tick().unwrap();

        assert!(!game.physics.has_character(ArpgGame::monster_body_id(1)));
        assert!(game.physics.has_character(ArpgGame::monster_body_id(2)));
    }

    fn monster_state(game: &ArpgGame, id: u32) -> MonsterState {
        *game
            .monsters
            .iter()
            .find(|monster| monster.id == id)
            .unwrap()
    }

    /// The published behaviour and engaged target of monster `id`.
    fn published(game: &ArpgGame, id: u32) -> (MonsterBehavior, Option<PlayerId>) {
        let snapshot = game.snapshot().unwrap();
        let monster = snapshot
            .monsters
            .iter()
            .find(|monster| monster.id == id)
            .unwrap();
        (monster.behavior, monster.target_player_id)
    }

    fn place_player(game: &mut ArpgGame, id: PlayerId, x: i32, z: i32) {
        game.put_player(id, Vec3i::new(x, PLAYER_Y, z));
    }

    fn add_player_at(game: &mut ArpgGame, id: PlayerId, x: i32, z: i32) {
        game.add_player(id).unwrap();
        place_player(game, id, x, z);
    }

    fn next_sequence(game: &ArpgGame, id: PlayerId) -> u32 {
        game.last_sequences[&id] + 1
    }

    fn command_as(game: &mut ArpgGame, id: PlayerId, command: ArpgCommand) {
        let sequence = next_sequence(game, id);
        game.apply_command(PlayerCommand::new(id, sequence, command).unwrap())
            .unwrap();
    }

    #[test]
    fn two_competing_targets_switch_only_beyond_the_hysteresis_margin() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 600, z);
        game.advance_tick().unwrap();
        // It notices its target at the end of a tick and sets out in the next.
        assert_eq!(published(&game, 1), (MonsterBehavior::Holding, Some(1)));
        game.advance_tick().unwrap();
        assert_eq!(published(&game, 1), (MonsterBehavior::Pursuing, Some(1)));

        // A second player arrives nearer than the target, but by less than the margin.
        let margin = brute().target_switch_margin;
        let monster = monster_position(&game, 1);
        let current = xz_distance_sq(monster, player_position(&game)).sqrt();
        let offset = (current - margin as f64 + 30.0) as f32;
        game.add_player(2).unwrap();
        game.put_player(2, monster + Vector::Z * offset);
        for _ in 0..20 {
            game.advance_tick().unwrap();
            assert_eq!(published(&game, 1), (MonsterBehavior::Pursuing, Some(1)));
        }

        // Now clearly nearer: it takes over.
        let monster = monster_position(&game, 1);
        let current = xz_distance_sq(monster, player_position(&game)).sqrt();
        let offset = (current - margin as f64 - 30.0) as f32;
        game.put_player(2, monster + Vector::Z * offset);
        game.advance_tick().unwrap();
        assert_eq!(monster_state(&game, 1).engaged_target(), Some(2));

        // Player 1 coming back to an equal distance does not take it back.
        let monster = monster_position(&game, 1);
        game.put_player(1, monster - Vector::Z * offset);
        for _ in 0..10 {
            game.advance_tick().unwrap();
            assert_eq!(monster_state(&game, 1).engaged_target(), Some(2));
        }

        // A competitor it can strike at once beats a target it cannot.
        let monster = monster_position(&game, 1);
        game.put_player(1, monster - Vector::X * 120.0);
        game.advance_tick().unwrap();
        let state = monster_state(&game, 1);
        assert_eq!(state.engaged_target(), Some(1));
        assert_eq!(state.action.unwrap().target_player_id, 1);
        assert_eq!(published(&game, 1).0, MonsterBehavior::Attacking);
    }

    #[test]
    fn a_crowd_surrounds_its_target_without_overlapping_bodies() {
        let (mut game, x, z) = strike_arena();
        // The target outlasts the test; only the approach matters here.
        game.players.get_mut(&1).unwrap().health = u16::MAX;
        let mut id = 0;
        for column in 0..4 {
            for row in 0..2 {
                id += 1;
                place_monster(&mut game, id, x + 450 + column * 70, z - 35 + row * 70);
            }
        }
        let mut attackers = BTreeSet::new();
        for _ in 0..360 {
            game.advance_tick().unwrap();
            for (index, monster) in game.monsters.iter().enumerate() {
                for other in &game.monsters[index + 1..] {
                    assert!(
                        !bodies_overlap(
                            monster.position,
                            MONSTER_BODY_HALF_EXTENTS,
                            other.position,
                            MONSTER_BODY_HALF_EXTENTS
                        ),
                        "tick {}: monsters {} and {} overlap: {:?} {:?}",
                        game.tick,
                        monster.id,
                        other.id,
                        monster.position,
                        other.position
                    );
                }
                assert!(!bodies_overlap(
                    monster.position,
                    MONSTER_BODY_HALF_EXTENTS,
                    game.players[&1].position,
                    PLAYER_HALF_EXTENTS
                ));
                if monster.action.is_some() {
                    attackers.insert(monster.id);
                }
            }
        }
        // Separation spreads them around the target instead of queueing on one side.
        assert!(attackers.len() >= 6, "attackers {attackers:?}");
    }

    #[test]
    fn a_lost_target_is_searched_for_then_another_is_reacquired() {
        let (mut game, x, z) = strike_arena();
        add_player_at(&mut game, 2, x - 400, z + 300);
        place_monster(&mut game, 1, x + 500, z);
        for _ in 0..3 {
            game.advance_tick().unwrap();
        }
        assert_eq!(published(&game, 1), (MonsterBehavior::Pursuing, Some(1)));

        // The target dies: the monster stops, searches, then engages the other player.
        game.players.get_mut(&1).unwrap().health = 0;
        game.advance_tick().unwrap();
        let held_at = monster_position(&game, 1);
        let reacquire = brute().reacquire_ticks;
        assert_eq!(
            monster_state(&game, 1).engagement,
            Engagement::Searching {
                ticks_remaining: reacquire
            }
        );
        for _ in 1..reacquire {
            game.advance_tick().unwrap();
            assert_eq!(published(&game, 1), (MonsterBehavior::Searching, None));
            assert_eq!(monster_position(&game, 1), held_at);
        }
        game.advance_tick().unwrap();
        assert_eq!(published(&game, 1), (MonsterBehavior::Holding, Some(2)));
        game.advance_tick().unwrap();
        assert_eq!(published(&game, 1), (MonsterBehavior::Pursuing, Some(2)));

        // Its new target disconnects mid-chase and rejoins during the search.
        assert!(game.remove_player(2));
        game.advance_tick().unwrap();
        assert_eq!(published(&game, 1), (MonsterBehavior::Searching, None));
        let monster = monster_position(&game, 1);
        game.add_player(2).unwrap();
        game.put_player(2, monster - Vector::X * 500.0);
        for _ in 1..reacquire {
            game.advance_tick().unwrap();
            assert_eq!(published(&game, 1).0, MonsterBehavior::Searching);
        }
        game.advance_tick().unwrap();
        assert_eq!(published(&game, 1), (MonsterBehavior::Holding, Some(2)));
        assert!(ticks_until_windup(&mut game, 1, 400).is_some());
        assert_eq!(monster_state(&game, 1).action.unwrap().target_player_id, 2);
    }

    #[test]
    fn a_struck_monster_turns_on_its_attacker_unless_already_engaged() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 200, z);
        game.monsters[0].engagement = Engagement::Searching { ticks_remaining: 5 };
        // Facing +x: the light swing (reach 220) hits the monster at 200.
        run_action(&mut game, 1, 1, ArpgCommand::PrimaryAttack { aim: None });
        let state = monster_state(&game, 1);
        assert!(state.health < brute().health);
        assert_eq!(state.engaged_target(), Some(1));

        let (mut game, x, z) = strike_arena();
        // Player 1, the striker, is not nearer than the target by the switch margin.
        add_player_at(&mut game, 2, x + 200, z + 300);
        place_monster(&mut game, 1, x + 200, z);
        game.monsters[0].engagement = Engagement::Engaged {
            target_player_id: 2,
        };
        run_action(&mut game, 1, 1, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(monster_state(&game, 1).engaged_target(), Some(2));
    }

    #[test]
    fn an_interrupted_windup_resumes_pursuit_of_the_same_target() {
        let (mut game, x, z) = strike_arena();
        // Player 2 stands in its own sword reach but outside the claw's, facing the monster.
        add_player_at(&mut game, 2, x + 150, z + 200);
        command_as(&mut game, 2, ArpgCommand::SetMovement { x: 0, z: -1 });
        command_as(&mut game, 2, ArpgCommand::SetMovement { x: 0, z: 0 });
        place_monster(&mut game, 1, x + 150, z);
        game.advance_tick().unwrap();
        let windup = monster_state(&game, 1).action.unwrap();
        assert_eq!(
            (windup.phase, windup.target_player_id),
            (ActionPhase::Windup, 1)
        );

        // Player 1 backs off while player 2 interrupts the wind-up.
        command_as(&mut game, 1, ArpgCommand::SetMovement { x: -1, z: 0 });
        command_as(&mut game, 2, ArpgCommand::PrimaryAttack { aim: None });
        let staggered = (1..=10)
            .find(|_| {
                game.advance_tick().unwrap();
                monster_state(&game, 1).stagger_ticks_remaining > 0
            })
            .expect("the strike lands before the claw");
        assert!(staggered < usize::from(brute().windup_ticks));
        assert_eq!(published(&game, 1), (MonsterBehavior::Staggered, Some(1)));
        let held_at = monster_position(&game, 1);
        while monster_state(&game, 1).stagger_ticks_remaining > 0 {
            assert_eq!(monster_position(&game, 1), held_at);
            game.advance_tick().unwrap();
        }

        // Free again, it chases the same target: no search, no switch to player 2.
        let resumed = (1..=3)
            .find(|_| {
                game.advance_tick().unwrap();
                published(&game, 1) == (MonsterBehavior::Pursuing, Some(1))
            })
            .expect("pursuit resumes");
        assert!(resumed <= 2);
        command_as(&mut game, 1, ArpgCommand::SetMovement { x: 0, z: 0 });
        assert!(ticks_until_windup(&mut game, 1, 400).is_some());
        assert_eq!(monster_state(&game, 1).action.unwrap().target_player_id, 1);
    }

    #[test]
    fn a_leashed_monster_returns_to_its_post_ignoring_players_until_home() {
        let (mut game, x, _) = strike_arena();
        let room = game
            .rooms
            .iter()
            .find(|room| room.id == STRIKE_ROOM)
            .unwrap()
            .clone();
        let post = Vector::from(Vec3i::new(room.min_x + 150, PLAYER_Y, room.min_z + 150));
        let start = Vec3i::new(room.max_x - 150, PLAYER_Y, room.max_z - 150);
        let leash = brute().leash_range;
        assert!(!within_range(xz_distance_sq(post, start.into()), leash));
        place_monster(&mut game, 1, start.x, start.z);
        game.monsters[0].post = Some(post);
        game.monsters[0].engagement = Engagement::Engaged {
            target_player_id: 1,
        };
        // Beyond the leash, it breaks off at the end of its first tick.
        game.advance_tick().unwrap();
        assert_eq!(published(&game, 1), (MonsterBehavior::Returning, None));

        // On the way home it walks past a player in reach without attacking.
        for _ in 0..40 {
            let monster = monster_position(&game, 1);
            game.put_player(1, monster + Vector::new(100.0, 0.0, 100.0));
            let before = xz_distance_sq(monster_position(&game, 1), post);
            game.advance_tick().unwrap();
            assert_eq!(published(&game, 1), (MonsterBehavior::Returning, None));
            assert!(monster_state(&game, 1).action.is_none());
            assert!(xz_distance_sq(monster_position(&game, 1), post) < before);
        }

        // Home again with nobody in aggro range, it rests; then re-engages a player.
        place_player(&mut game, 1, x + 600, room.max_z - 150);
        let aggro = brute().aggro_range;
        assert!(!within_range(
            xz_distance_sq(post, player_position(&game)),
            aggro
        ));
        let home = (1..=600)
            .find(|_| {
                game.advance_tick().unwrap();
                published(&game, 1).0 == MonsterBehavior::Idle
            })
            .expect("it gets home");
        assert!(home > 1);
        assert!(within_range(
            xz_distance_sq(monster_position(&game, 1), post),
            POST_ARRIVAL_DISTANCE
        ));
        game.advance_tick().unwrap();
        assert_eq!(published(&game, 1).0, MonsterBehavior::Idle);
        game.put_player(1, post + Vector::new(400.0, 0.0, 300.0));
        game.advance_tick().unwrap();
        assert_eq!(published(&game, 1), (MonsterBehavior::Holding, Some(1)));
    }

    #[test]
    fn a_monster_that_finds_nobody_after_a_search_returns_to_its_post() {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 500, z);
        for _ in 0..80 {
            game.advance_tick().unwrap();
        }
        let post = monster_state(&game, 1).post.unwrap();
        assert_eq!(post, Vec3i::new(x + 500, PLAYER_Y, z).into());
        assert!(!within_range(
            xz_distance_sq(monster_position(&game, 1), post),
            200
        ));
        assert!(game.remove_player(1));
        for _ in 0..=brute().reacquire_ticks {
            game.advance_tick().unwrap();
        }
        assert_eq!(published(&game, 1), (MonsterBehavior::Returning, None));
        let home = (1..=200).find(|_| {
            game.advance_tick().unwrap();
            published(&game, 1).0 == MonsterBehavior::Idle
        });
        assert!(home.is_some());
    }

    /// A seeded chase in the strike arena: three pursuers and a target walking a square.
    /// With `drop_bodies_at`, every monster body is removed from the world at that tick.
    fn body_chase(drop_bodies_at: Option<u32>) -> Vec<ArpgSnapshot> {
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 600, z);
        place_monster(&mut game, 2, x + 600, z + 120);
        place_monster(&mut game, 3, x - 600, z - 200);
        let mut snapshots = Vec::new();
        for tick in 0..160_u32 {
            if tick % 30 == 0 {
                let (dx, dz) = [(1, 0), (0, 1), (-1, 0), (0, -1)][(tick / 30) as usize % 4];
                command_as(&mut game, 1, ArpgCommand::SetMovement { x: dx, z: dz });
            }
            if Some(tick) == drop_bodies_at {
                for id in 1..=3 {
                    assert!(game.physics.remove_character(ArpgGame::monster_body_id(id)));
                }
            }
            game.advance_tick().unwrap();
            snapshots.push(game.snapshot().unwrap());
        }
        snapshots
    }

    #[test]
    fn monster_bodies_removed_mid_chase_are_rebuilt_without_changing_the_game() {
        let uninterrupted = body_chase(None);
        assert!(uninterrupted[50].monsters.iter().all(|monster| {
            monster.behavior == MonsterBehavior::Pursuing && monster.target_player_id == Some(1)
        }));
        for tick in [40, 50, 90] {
            assert_eq!(body_chase(Some(tick)), uninterrupted, "dropped at {tick}");
        }
    }

    #[test]
    fn a_displaced_pursuer_keeps_its_target_and_resumes_from_where_it_lands() {
        // Knockback is not a mechanic yet (#63); any displacement of the body must already
        // leave the engagement intact and the chase continuing from the new position.
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 600, z);
        for _ in 0..10 {
            game.advance_tick().unwrap();
        }
        assert_eq!(published(&game, 1), (MonsterBehavior::Pursuing, Some(1)));
        let from = monster_position(&game, 1);
        let landed = from + Vector::new(150.0, 0.0, 200.0);
        game.physics
            .place_character(ArpgGame::monster_body_id(1), landed)
            .unwrap();
        game.monsters[0].position = landed;
        let before = xz_distance_sq(landed, player_position(&game));
        game.advance_tick().unwrap();
        assert_eq!(published(&game, 1), (MonsterBehavior::Pursuing, Some(1)));
        assert!(xz_distance_sq(monster_position(&game, 1), player_position(&game)) < before);
        assert!(ticks_until_windup(&mut game, 1, 400).is_some());
    }

    #[test]
    fn engagement_survives_save_and_load_exactly() {
        // The room's generated monster: saves only accept the generated monster set.
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        let room = game
            .rooms
            .iter()
            .find(|room| room.id == STRIKE_ROOM)
            .unwrap()
            .clone();
        let (x, z) = room.center();
        game.player_spawns[0] = Vec3i::new(x, PLAYER_Y, z);
        game.add_player(1).unwrap();
        add_player_at(&mut game, 2, room.min_x + 100, room.max_z - 100);
        let id = game
            .monsters
            .iter()
            .find(|monster| monster.room_id == STRIKE_ROOM)
            .unwrap()
            .id;
        game.monsters
            .iter_mut()
            .find(|monster| monster.id == id)
            .unwrap()
            .position = Vec3i::new(room.max_x - 120, PLAYER_Y, z).into();
        game.reconcile_encounters().unwrap();
        // The target dies at tick 60; player 2 is out of aggro range, so the monster
        // searches, then returns to its post.
        let step = |game: &mut ArpgGame, tick: u32| {
            if tick == 60 {
                game.players.get_mut(&1).unwrap().health = 0;
            }
            game.advance_tick().unwrap();
        };
        let mut saves = Vec::new();
        let mut reference = Vec::new();
        for tick in 0..260_u32 {
            step(&mut game, tick);
            if [30, 65, 90, 120].contains(&tick) {
                saves.push((reference.len(), game.save_state().unwrap()));
            }
            reference.push(game.snapshot().unwrap());
        }
        let behaviours = saves
            .iter()
            .map(|&(index, _)| reference[index].monsters[usize::try_from(id - 1).unwrap()].behavior)
            .collect::<Vec<_>>();
        assert!(
            behaviours.contains(&MonsterBehavior::Pursuing),
            "{behaviours:?}"
        );
        assert!(
            behaviours.contains(&MonsterBehavior::Searching),
            "{behaviours:?}"
        );
        assert!(
            behaviours.contains(&MonsterBehavior::Returning),
            "{behaviours:?}"
        );
        for (index, save) in saves {
            let json = serde_json::to_string(&save).unwrap();
            let mut restored =
                ArpgGame::from_save_state(serde_json::from_str(&json).unwrap()).unwrap();
            let mut expected = reference[index].clone();
            // Strike and interaction events are transient (not saved).
            expected.strike_events.clear();
            expected.interaction_events.clear();
            assert_eq!(restored.snapshot().unwrap(), expected, "load at {index}");
            for (tick, expected) in reference.iter().enumerate().skip(index + 1) {
                step(&mut restored, u32::try_from(tick).unwrap());
                assert_eq!(&restored.snapshot().unwrap(), expected);
            }
        }
    }

    #[test]
    fn saves_reject_invalid_engagement() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        game.add_player(1).unwrap();
        let save = game.save_state().unwrap();
        let tamper = |change: &dyn Fn(&mut MonsterSaveState)| {
            let mut tampered = save.clone();
            change(&mut tampered.monsters[0]);
            ArpgGame::from_save_state(tampered)
        };
        assert!(tamper(&|monster| monster.engagement = Engagement::Returning).is_ok());
        assert!(
            tamper(&|monster| monster.engagement = Engagement::Engaged {
                target_player_id: 0
            })
            .is_err()
        );
        assert!(
            tamper(&|monster| monster.engagement = Engagement::Searching { ticks_remaining: 0 })
                .is_err()
        );
        assert!(
            tamper(&|monster| monster.engagement = Engagement::Searching {
                ticks_remaining: brute().reacquire_ticks + 1
            })
            .is_err()
        );
        assert!(tamper(&|monster| monster.post = Some([0.0, PLAYER_Y as f32, 99_999.0])).is_err());
    }

    #[test]
    fn saves_reject_a_guard_the_monster_cannot_hold() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        game.add_player(1).unwrap();
        let save = game.save_state().unwrap();
        let defender = save
            .monsters
            .iter()
            .position(|monster| monster.room_id == 6)
            .unwrap();
        assert_eq!(
            content().id(game.monsters[defender].definition().id),
            DEFENDER
        );
        let ready = save.monsters[defender]
            .guard
            .expect("a shield monster saves its guard");
        assert_eq!(ready, GuardState::ready(MAX_GUARD_POINTS));
        let tamper = |index: usize, change: &dyn Fn(&mut MonsterSaveState)| {
            let mut tampered = save.clone();
            change(&mut tampered.monsters[index]);
            ArpgGame::from_save_state(tampered)
        };
        // The defender's encounter running, as when the runtime holds its guard.
        let mut active_save = save.clone();
        for room in &mut active_save.rooms {
            if room.id == 6 {
                room.encounter_state = RoomEncounterState::Active;
            }
        }
        let tamper_active = |change: &dyn Fn(&mut MonsterSaveState)| {
            let mut tampered = active_save.clone();
            change(&mut tampered.monsters[defender]);
            ArpgGame::from_save_state(tampered)
        };
        let raised = Some(GuardStance {
            phase: GuardPhase::Raised,
            ticks_remaining: 0,
        });
        let engaged = Engagement::Engaged {
            target_player_id: 1,
        };
        assert!(
            tamper_active(&|monster| {
                monster.engagement = engaged;
                monster.guard = Some(GuardState {
                    held: true,
                    stance: raised,
                    ..ready
                });
            })
            .is_ok()
        );
        // Held in a dormant encounter, which the runtime never produces.
        assert!(
            tamper(defender, &|monster| {
                monster.engagement = engaged;
                monster.guard = Some(GuardState {
                    held: true,
                    stance: raised,
                    ..ready
                });
            })
            .is_err()
        );
        // Without a shield, or a shield monster without its guard.
        assert!(tamper(0, &|monster| monster.guard = Some(ready)).is_err());
        assert!(tamper(defender, &|monster| monster.guard = None).is_err());
        for invalid in [
            // Held only by an engaged monster.
            GuardState {
                held: true,
                ..ready
            },
            // A stance only while held.
            GuardState {
                stance: raised,
                ..ready
            },
            GuardState {
                points: MAX_GUARD_POINTS + 1,
                ..ready
            },
        ] {
            assert!(tamper(defender, &|monster| monster.guard = Some(invalid)).is_err());
        }
        // No stance while staggered.
        assert!(
            tamper_active(&|monster| {
                monster.engagement = engaged;
                monster.stagger_ticks_remaining = 1;
                monster.guard = Some(GuardState {
                    held: true,
                    stance: raised,
                    ..ready
                });
            })
            .is_err()
        );
        // A stagger keeps the guard held without a stance, as the runtime saves it.
        assert!(
            tamper_active(&|monster| {
                monster.engagement = engaged;
                monster.stagger_ticks_remaining = 1;
                monster.guard = Some(GuardState {
                    held: true,
                    ..ready
                });
            })
            .is_ok()
        );
        // Committing to an attack lets go of the guard: never held while acting.
        let acting = |held: bool| {
            tamper_active(&|monster| {
                monster.engagement = engaged;
                monster.action = Some(MonsterActionSaveState {
                    phase: ActionPhase::Windup,
                    ticks_remaining: 1,
                    target_player_id: 1,
                });
                monster.guard = Some(GuardState { held, ..ready });
            })
        };
        assert!(acting(false).is_ok());
        assert!(acting(true).is_err());
    }

    /// Deterministic xorshift for randomized pursuit scenarios.
    struct ScenarioRng(u64);

    impl ScenarioRng {
        fn between(&mut self, low: i32, high: i32) -> i32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            low + i32::try_from((self.0 >> 33) % u64::from((high - low + 1).unsigned_abs()))
                .unwrap()
        }
    }

    /// A seeded chase in the strike arena: random pillars and pursuers, a target walking in
    /// random directions, and a pillar that appears at tick 40 and vanishes at tick 80.
    /// Returns the initial and every tick's snapshot, and the navigation work.
    fn random_chase(seed: u64, mode: NavigationMode) -> (Vec<ArpgSnapshot>, NavigationWork) {
        let mut rng = ScenarioRng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let (mut game, x, z) = strike_arena();
        game.navigation_mode = mode;
        let mut bodies = vec![(Vec3i::new(x, PLAYER_Y, z), Vec3i::new(80, 50, 80))];
        let mut free_spot = |rng: &mut ScenarioRng, half: Vec3i, near: i32| loop {
            let spot = Vec3i::new(
                x + rng.between(-near, near),
                PLAYER_Y,
                z + rng.between(-near / 2, near / 2),
            );
            if bodies
                .iter()
                .all(|&(other, other_half)| !overlaps_xz(spot, half, other, other_half))
            {
                bodies.push((spot, half));
                return spot;
            }
        };
        for index in 0..rng.between(0, 4) {
            let half = Vec3i::new(rng.between(10, 60), 50, rng.between(10, 60));
            let spot = free_spot(&mut rng, half, 450);
            place_blocker(&mut game, index as u64, spot.x, spot.z, half);
        }
        for id in 1..=rng.between(1, 3) {
            let spot = free_spot(&mut rng, MONSTER_BODY_HALF_EXTENTS, 700);
            // Melee, ranged and heavy pursuers share the planner (#108).
            let definition = [BRUTE, ARCHER, BRUISER][(id as usize - 1) % 3];
            place_monster_of(&mut game, id as u32, definition, spot.x, spot.z);
        }
        let late_pillar = free_spot(&mut rng, Vec3i::new(40, 50, 40), 450);
        let mut snapshots = vec![game.snapshot().unwrap()];
        for tick in 0..120_u32 {
            if tick % 20 == 0 {
                let (dx, dz) = (rng.between(-1, 1), rng.between(-1, 1));
                game.apply_command(
                    PlayerCommand::new(
                        1,
                        tick + 1,
                        ArpgCommand::SetMovement {
                            x: dx as i8,
                            z: dz as i8,
                        },
                    )
                    .unwrap(),
                )
                .unwrap();
            }
            if tick == 40 {
                place_blocker(
                    &mut game,
                    50,
                    late_pillar.x,
                    late_pillar.z,
                    Vec3i::new(40, 50, 40),
                );
            }
            if tick == 80 {
                game.physics.remove_fixed(STATIC_BODY_BASE + 950);
            }
            game.advance_tick().unwrap();
            snapshots.push(game.snapshot().unwrap());
        }
        (snapshots, game.navigation_work())
    }

    #[test]
    fn retained_navigation_never_changes_the_game() {
        let mut total = NavigationWork::default();
        for seed in 0..16 {
            let (retained, work) = random_chase(seed, NavigationMode::Retained);
            let (uncached, _) = random_chase(seed, NavigationMode::RetainedWithoutCache);
            assert_eq!(retained, uncached, "seed {seed}");
            total.add(work);
        }
        // The pillar that comes and goes rebuilds grids while pursuers still need them.
        assert!(total.grid_builds > 16, "{total:?}");
        assert!(total.repaths() < total.pursuit_ticks, "{total:?}");
    }

    #[test]
    fn retained_and_per_tick_pursuers_set_out_alike() {
        // Whether a pursuer moves on its first tick depends only on reachability, which the
        // retained planner shares with the per-tick reference. A resting monster engages at
        // the end of its first tick and sets out in the second.
        let moved = |snapshots: &[ArpgSnapshot]| {
            snapshots[1]
                .monsters
                .iter()
                .zip(&snapshots[2].monsters)
                .map(|(before, after)| before.position != after.position)
                .collect::<Vec<_>>()
        };
        let mut moving = 0;
        for seed in 100..160 {
            let (retained, _) = random_chase(seed, NavigationMode::Retained);
            let (reference, _) = random_chase(seed, NavigationMode::PerTickReference);
            assert_eq!(retained[..2], reference[..2]);
            assert_eq!(moved(&retained), moved(&reference), "seed {seed}");
            moving += moved(&retained).iter().filter(|&&moved| moved).count();
        }
        assert!(moving > 30);
    }

    #[test]
    fn strict_goal_cells_have_a_clear_physics_strike_line_to_their_whole_target_cell() {
        let reach = brute().strike.reach;
        for seed in 0..12 {
            let mut rng = ScenarioRng(seed | 1);
            let (mut game, x, z) = strike_arena();
            for index in 0..4 {
                let half = Vec3i::new(rng.between(10, 60), 50, rng.between(10, 60));
                place_blocker(
                    &mut game,
                    index,
                    x + rng.between(-400, 400),
                    z + rng.between(-250, 250),
                    half,
                );
            }
            let obstacles = game.fixed_footprints();
            let grid = game.room_grid(STRIKE_ROOM, &obstacles);
            for _ in 0..8 {
                let target = (x + rng.between(-450, 450), z + rng.between(-300, 300));
                let Some(cell) = grid.target_cell(target) else {
                    continue;
                };
                let field = grid.target_field(cell, reach - i64::from(NAV_CELL_SIZE), &obstacles);
                let (goals, area) = field.strict_goal_centres(&grid);
                let samples = [
                    (area.min_x, area.min_z),
                    (area.max_x, area.max_z),
                    (area.min_x, area.max_z),
                    (area.max_x, area.min_z),
                    ((area.min_x + area.max_x) / 2, (area.min_z + area.max_z) / 2),
                    (target.0, target.1),
                ];
                for (gx, gz) in goals {
                    for (tx, tz) in samples {
                        let from = Vec3i::new(gx, PLAYER_Y, gz).into();
                        let to = Vec3i::new(tx, PLAYER_Y, tz).into();
                        assert!(within_range(
                            xz_distance_sq(from, to),
                            reach - i64::from(NAV_CELL_SIZE)
                        ));
                        assert!(
                            !game.strike_obstructed(from, to).unwrap(),
                            "seed {seed}: goal ({gx}, {gz}) to ({tx}, {tz})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_chase_continues_identically_after_a_save_that_drops_the_navigation_cache() {
        use seeded_workloads::{Case, commands};
        // The room's generated monsters, moved to its west edge: saves only accept the
        // generated monster set in its own rooms.
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        let room = game
            .rooms
            .iter()
            .find(|room| room.id == STRIKE_ROOM)
            .unwrap()
            .clone();
        let (x, z) = room.center();
        game.player_spawns[0] = Vec3i::new(x, PLAYER_Y, z);
        game.add_player(1).unwrap();
        for (index, monster) in game
            .monsters
            .iter_mut()
            .filter(|monster| monster.room_id == STRIKE_ROOM)
            .enumerate()
        {
            monster.position = Vec3i::new(
                room.min_x + 120,
                PLAYER_Y,
                z + i32::try_from(index).unwrap() * 90,
            )
            .into();
        }
        game.reconcile_encounters().unwrap();
        for tick in 0..45 {
            commands(&mut game, Case::Chase, tick);
            game.advance_tick().unwrap();
        }
        assert!(
            !game.navigation.fields.is_empty(),
            "the uninterrupted game holds fields"
        );
        let mut restored = ArpgGame::from_save_state(game.save_state().unwrap()).unwrap();
        assert!(restored.navigation.fields.is_empty());
        for tick in 45..140 {
            commands(&mut game, Case::Chase, tick);
            commands(&mut restored, Case::Chase, tick);
            game.advance_tick().unwrap();
            restored.advance_tick().unwrap();
            assert_eq!(
                restored.snapshot().unwrap(),
                game.snapshot().unwrap(),
                "tick {tick}"
            );
        }
        assert!(game.navigation_work().pursuit_ticks > 0);
    }

    #[test]
    fn pursuit_replays_and_continues_identically_after_a_save() {
        // The room's generated monster: saves only accept the generated monster set.
        let run = || {
            let mut game = ArpgGame::new_with_seed(42).unwrap();
            let (x, z) = game
                .rooms
                .iter()
                .find(|room| room.id == STRIKE_ROOM)
                .unwrap()
                .center();
            game.player_spawns[0] = Vec3i::new(x, PLAYER_Y, z);
            game.add_player(1).unwrap();
            let chaser = *game
                .monsters
                .iter()
                .find(|monster| monster.room_id == STRIKE_ROOM)
                .unwrap();
            game.apply_command(
                PlayerCommand::new(1, 1, ArpgCommand::SetMovement { x: 1, z: 1 }).unwrap(),
            )
            .unwrap();
            for _ in 0..30 {
                game.advance_tick().unwrap();
            }
            assert_ne!(
                monster_position(&game, chaser.id),
                chaser.position,
                "mid-chase"
            );
            game
        };
        let mut game = run();
        assert_eq!(run().snapshot().unwrap(), game.snapshot().unwrap());

        let mut restored = ArpgGame::from_save_state(game.save_state().unwrap()).unwrap();
        for _ in 0..40 {
            game.advance_tick().unwrap();
            restored.advance_tick().unwrap();
            assert_eq!(restored.snapshot().unwrap(), game.snapshot().unwrap());
        }
    }

    // Enemy roles (#108): ranged spacing and a heavy telegraphed attack, selected by
    // content on the shared locomotion, engagement and strike paths.

    fn archer() -> &'static MonsterDefinition {
        content().monster(definition_index(ARCHER))
    }

    fn bruiser() -> &'static MonsterDefinition {
        content().monster(definition_index(BRUISER))
    }

    /// Asserts that monster `id` stands inside its room and touches no fixed body.
    fn assert_inside_room_and_clear_of_fixed_bodies(game: &ArpgGame, id: u32, tick: u32) {
        let monster = game.monsters.iter().find(|m| m.id == id).unwrap();
        let room = game.rooms.iter().find(|r| r.id == monster.room_id).unwrap();
        assert!(
            room.contains_xz_with_margin(monster.position, MONSTER_BODY_HALF_EXTENTS.x),
            "tick {tick}: {:?} left room {}",
            monster.position,
            room.id
        );
        for (center, half) in game.physics.fixed_boxes() {
            assert!(
                !body_overlaps_box(monster.position, MONSTER_BODY_HALF_EXTENTS, center, half),
                "tick {tick}: {:?} overlaps fixed body at {center:?}",
                monster.position,
            );
        }
    }

    fn monster_shots(snapshot: &ArpgSnapshot, id: u32) -> Vec<StrikeEventSnapshot> {
        snapshot
            .strike_events
            .iter()
            .filter(|event| event.source == StrikeSource::Monster(id))
            .cloned()
            .collect()
    }

    #[test]
    fn the_default_dungeon_places_melee_ranged_and_heavy_roles_that_differ_in_play() {
        // Each role starts 500 units from the same player in the same room.
        let mut first_windups = BTreeMap::new();
        for definition in [BRUTE, ARCHER, BRUISER] {
            let (mut game, x, z) = strike_arena();
            place_monster_of(&mut game, 1, definition, x + 500, z);
            let start = (1..=400)
                .find(|_| {
                    game.advance_tick().unwrap();
                    game.monsters[0].action.is_some()
                })
                .expect("every role attacks");
            let snapshot = game.snapshot().unwrap();
            let action = snapshot.monsters[0].action.unwrap();
            let distance = xz_distance_sq(game.monsters[0].position, player_position(&game)).sqrt();
            first_windups.insert(definition, (start, distance, action));
        }
        let (brute_start, brute_distance, brute_action) = first_windups[BRUTE];
        let (archer_start, archer_distance, archer_action) = first_windups[ARCHER];
        let (bruiser_start, bruiser_distance, bruiser_action) = first_windups[BRUISER];
        // Melee closes in and attacks at claw reach.
        assert!(brute_distance <= brute().strike.reach as f64);
        assert_eq!(brute_action.delivery, AttackDelivery::Strike);
        // Ranged attacks at once from its band, far outside any melee reach.
        assert_eq!(archer_start, 1);
        assert!(archer_distance >= archer().retreat_range.unwrap() as f64);
        assert!(archer_distance > (2 * brute().strike.reach) as f64);
        assert_eq!(archer_action.delivery, AttackDelivery::Projectile);
        // Heavy arrives last and telegraphs far longer than melee.
        assert!(bruiser_start > brute_start, "{first_windups:?}");
        assert!(bruiser_distance <= bruiser().strike.reach as f64);
        assert_eq!(bruiser_action.delivery, AttackDelivery::Strike);
        assert_eq!(bruiser_action.ticks_remaining, bruiser().windup_ticks);
        assert!(bruiser().windup_ticks >= 2 * brute().windup_ticks);
        assert!(bruiser().damage >= 2 * brute().damage);
        assert!(bruiser().strike.guard_cost >= content().guard.max_points);
    }

    #[test]
    fn an_archer_backs_off_around_a_wall_into_its_band_and_never_enters_fixed_bodies() {
        let (mut game, x, z) = strike_arena();
        place_monster_of(&mut game, 1, ARCHER, x + 420, z);
        // A wall right behind the archer: it backs off sideways, never into the wall.
        place_blocker(&mut game, 0, x + 520, z, Vec3i::new(20, 50, 220));
        let (retreat, reach) = (archer().retreat_range.unwrap(), archer().strike.reach);
        let mut retreats = 0;
        let mut shots = 0;
        let mut attacked_inside_range = false;
        for tick in 0..360 {
            match tick {
                // The player walks up to the archer once its first shot is in recovery.
                60 => command_as(&mut game, 1, ArpgCommand::SetMovement { x: 1, z: 0 }),
                90 => command_as(&mut game, 1, ArpgCommand::SetMovement { x: 0, z: 0 }),
                _ => {}
            }
            let was_free = game.monsters[0].action.is_none();
            game.advance_tick().unwrap();
            assert_inside_room_and_clear_of_fixed_bodies(&game, 1, tick);
            let snapshot = game.snapshot().unwrap();
            let monster = &snapshot.monsters[0];
            if monster.behavior == MonsterBehavior::Retreating {
                retreats += 1;
                assert_eq!(monster.target_player_id, Some(1));
            }
            if was_free && game.monsters[0].action.is_some() {
                let distance_sq = xz_distance_sq(game.monsters[0].position, player_position(&game));
                attacked_inside_range |= distance_sq < range_sq(retreat);
            }
            shots += monster_shots(&snapshot, 1).len();
        }
        assert!(retreats > 5, "{retreats}");
        assert!(
            !attacked_inside_range,
            "an archer with room to back off never fires up close"
        );
        assert!(shots >= 3, "{shots}");
        let distance = xz_distance_sq(game.monsters[0].position, player_position(&game)).sqrt();
        assert!(
            (retreat as f64..=reach as f64).contains(&distance),
            "ends in its band: {distance}"
        );
    }

    #[test]
    fn a_cornered_archer_shoots_from_where_it_stands() {
        let (mut game, _, _) = strike_arena();
        let room = game
            .rooms
            .iter()
            .find(|r| r.id == STRIKE_ROOM)
            .unwrap()
            .clone();
        let corner = Vec3i::new(room.max_x - 90, PLAYER_Y, room.max_z - 90);
        place_monster_of(&mut game, 1, ARCHER, corner.x, corner.z);
        place_player(&mut game, 1, corner.x - 180, corner.z - 180);
        let retreat = archer().retreat_range.unwrap();
        let fired_up_close = (0..120).any(|tick| {
            game.advance_tick().unwrap();
            assert_inside_room_and_clear_of_fixed_bodies(&game, 1, tick);
            game.arrows
                .iter()
                .any(|arrow| arrow.source == StrikeSource::Monster(1))
                && xz_distance_sq(game.monsters[0].position, player_position(&game))
                    < range_sq(retreat)
        });
        assert!(fired_up_close);
    }

    fn ranged_scenario() -> ArpgGame {
        let mut game = ArpgGame::new_scenario(ScenarioId::Ranged, 42).unwrap();
        game.add_player(1).unwrap();
        game
    }

    /// Advances until a monster shot is in flight; returns it.
    fn advance_until_shot(game: &mut ArpgGame) -> ArrowState {
        (0..120)
            .find_map(|_| {
                game.advance_tick().unwrap();
                game.arrows
                    .iter()
                    .find(|arrow| matches!(arrow.source, StrikeSource::Monster(_)))
                    .copied()
            })
            .expect("the archer shoots")
    }

    #[test]
    fn archer_shots_fly_the_shared_arrow_path_and_hit_through_the_monster_strike() {
        let mut game = ranged_scenario();
        let id = game
            .monsters
            .iter()
            .find(|m| *content().id(m.definition().id) == *ARCHER)
            .unwrap()
            .id;
        let shot = advance_until_shot(&mut game);
        let definition = archer();
        let projectile = definition.projectile.unwrap();
        assert_eq!(shot.source, StrikeSource::Monster(id));
        // Engaged and winding up at the end of tick 0, it launches as the active phase
        // opens, from the archer toward the player (-x).
        assert_eq!(shot.launched_at_tick, u64::from(definition.windup_ticks));
        assert_eq!(
            shot.velocity,
            Vector::new(-projectile.speed as f32, 0.0, 0.0)
        );
        assert_eq!(shot.damage, definition.damage);
        assert_eq!(shot.ticks_remaining, projectile.lifetime_ticks);
        let hit = (0..40)
            .find_map(|_| {
                game.advance_tick().unwrap();
                monster_shots(&game.snapshot().unwrap(), id).pop()
            })
            .expect("the shot lands");
        assert_eq!(hit.definition, "monster.bolt");
        assert_eq!(hit.strike_tick, shot.launched_at_tick);
        assert_eq!(hit.target, StrikeTarget::Player(1));
        assert_eq!(
            hit.result,
            StrikeResult::Hit {
                damage: definition.damage,
                defeated: false
            }
        );
        let snapshot = game.snapshot().unwrap();
        assert_eq!(
            snapshot.players[0].health,
            BASE_MAX_HEALTH - definition.damage
        );
        assert!(snapshot.arrows.is_empty());
    }

    #[test]
    fn a_raised_shield_blocks_an_archer_shot_and_walls_stop_it() {
        let mut blocked = ranged_scenario();
        let id = blocked
            .monsters
            .iter()
            .find(|m| *content().id(m.definition().id) == *ARCHER)
            .unwrap()
            .id;
        command_as(&mut blocked, 1, ArpgCommand::SetGuard { raised: true });
        advance_until_shot(&mut blocked);
        let block = (0..40)
            .find_map(|_| {
                blocked.advance_tick().unwrap();
                monster_shots(&blocked.snapshot().unwrap(), id).pop()
            })
            .unwrap();
        let guard_cost = archer().strike.guard_cost;
        assert_eq!(
            block.result,
            StrikeResult::Blocked {
                guard_damage: guard_cost
            }
        );
        let player = &blocked.snapshot().unwrap().players[0];
        assert_eq!(player.health, BASE_MAX_HEALTH);
        assert_eq!(player.guard_points, MAX_GUARD_POINTS - guard_cost);

        // A pillar raised into the shot's path after release stops it.
        let mut walled = ranged_scenario();
        let shot = advance_until_shot(&mut walled);
        let player = player_position(&walled);
        place_blocker(
            &mut walled,
            0,
            ((shot.position.x + player.x) / 2.0) as i32,
            player.z as i32,
            Vec3i::new(20, 50, 80),
        );
        for _ in 0..40 {
            walled.advance_tick().unwrap();
            assert!(monster_shots(&walled.snapshot().unwrap(), id).is_empty());
        }
        assert!(walled.arrows.is_empty());
        assert_eq!(
            walled.snapshot().unwrap().players[0].health,
            BASE_MAX_HEALTH
        );
    }

    fn ranged_archer_id(game: &ArpgGame) -> u32 {
        game.monsters
            .iter()
            .find(|m| *content().id(m.definition().id) == *ARCHER)
            .unwrap()
            .id
    }

    /// Advances until the archer's shot `shot` resolves against a player; returns that event.
    fn land_shot(game: &mut ArpgGame, id: u32, shot: ArrowState) -> StrikeEventSnapshot {
        let event = (0..40)
            .find_map(|_| {
                game.advance_tick().unwrap();
                monster_shots(&game.snapshot().unwrap(), id).pop()
            })
            .expect("the shot lands");
        assert_eq!(event.strike_tick, shot.launched_at_tick);
        assert_eq!(event.target, StrikeTarget::Player(1));
        event
    }

    /// Faces player 1 along x without moving it.
    fn face_x(game: &mut ArpgGame, facing_x: i8) {
        let state = game.players.get_mut(&1).unwrap();
        state.facing_x = facing_x;
        state.facing_z = 0;
    }

    #[test]
    fn archer_shots_break_a_guard_whose_points_reach_the_shot_cost() {
        let guard_cost = archer().strike.guard_cost;
        assert!(guard_cost > 0, "a free shot could never break a guard");

        // One point above the cost: the shot is blocked and leaves one point.
        let mut game = ranged_scenario();
        let id = ranged_archer_id(&game);
        command_as(&mut game, 1, ArpgCommand::SetGuard { raised: true });
        let shot = advance_until_shot(&mut game);
        game.players.get_mut(&1).unwrap().guard.points = guard_cost + 1;
        assert_eq!(
            land_shot(&mut game, id, shot).result,
            StrikeResult::Blocked {
                guard_damage: guard_cost
            }
        );
        let player = game.players[&1];
        assert_eq!(player.guard.points, 1);
        assert_eq!(player.health, BASE_MAX_HEALTH);

        // Exactly the cost: the shot breaks the guard instead of being blocked.
        let mut game = ranged_scenario();
        let id = ranged_archer_id(&game);
        command_as(&mut game, 1, ArpgCommand::SetGuard { raised: true });
        let shot = advance_until_shot(&mut game);
        let state = game.players.get_mut(&1).unwrap();
        state.guard.points = guard_cost;
        // A standing counter opportunity from an earlier block is lost to the break.
        state.counter = Some(CounterOpportunity::grant(1_000, id, shot.launched_at_tick));
        let event = land_shot(&mut game, id, shot);
        assert_eq!(event.definition, "monster.bolt");
        assert_eq!(event.result, StrikeResult::GuardBroken);
        let player = game.players[&1];
        assert_eq!(
            player.health, BASE_MAX_HEALTH,
            "a guard break absorbs that shot"
        );
        assert_eq!(player.guard.points, 0);
        assert_eq!(player.guard.stance, None);
        assert_eq!(player.counter, None);
        assert!(player.guard.broken_ticks_remaining > 0);
        assert!(player.guard.broken_ticks_remaining <= game.tuning.guard.break_ticks);
        assert!(game.arrows.is_empty());
    }

    #[test]
    fn an_archer_shot_from_behind_a_raised_shield_hits() {
        let mut game = ranged_scenario();
        let id = ranged_archer_id(&game);
        command_as(&mut game, 1, ArpgCommand::SetGuard { raised: true });
        let shot = advance_until_shot(&mut game);
        // The shot flies -x from the archer; the shield faces -x, away from it.
        assert!(shot.velocity.x < 0.0 && shot.velocity.z == 0.0);
        face_x(&mut game, -1);
        assert!(game.players[&1].guard.is_raised());
        let event = land_shot(&mut game, id, shot);
        assert_eq!(
            event.result,
            StrikeResult::Hit {
                damage: archer().damage,
                defeated: false
            }
        );
        let player = game.players[&1];
        assert_eq!(player.health, BASE_MAX_HEALTH - archer().damage);
        assert_eq!(
            player.guard.points, MAX_GUARD_POINTS,
            "a rear hit costs no guard"
        );
        assert_eq!(player.counter, None, "a rear hit grants no counter");
        assert_eq!(
            player.guard.stance, None,
            "an unblocked hit lowers the shield"
        );
    }

    #[test]
    fn an_archer_shot_is_judged_by_its_incoming_side_after_the_archer_moves() {
        // The archer relocates to the far side of the player after release. The shield must
        // follow where the shot comes from (+x), not where the archer now stands (-x).
        for (facing_x, blocked) in [(1, true), (-1, false)] {
            let mut game = ranged_scenario();
            let id = ranged_archer_id(&game);
            command_as(&mut game, 1, ArpgCommand::SetGuard { raised: true });
            let shot = advance_until_shot(&mut game);
            let player = player_position(&game);
            assert!(shot.position.x > player.x, "the shot starts on the +x side");
            assert!(shot.velocity.x < 0.0 && shot.velocity.z == 0.0);
            let moved = player - Vector::X * 300.0;
            game.physics
                .place_character(ArpgGame::monster_body_id(id), moved)
                .unwrap();
            game.monsters
                .iter_mut()
                .find(|m| m.id == id)
                .unwrap()
                .position = moved;
            face_x(&mut game, facing_x);
            let event = land_shot(&mut game, id, shot);
            // The archer really stands on the other side when the shot lands.
            let archer_at = game.monsters.iter().find(|m| m.id == id).unwrap().position;
            assert!(
                archer_at.x < player_position(&game).x,
                "facing {facing_x}: archer at {archer_at:?}"
            );
            let player = game.players[&1];
            if blocked {
                assert_eq!(
                    event.result,
                    StrikeResult::Blocked {
                        guard_damage: archer().strike.guard_cost
                    },
                    "facing the shot blocks it although the archer is behind"
                );
                assert_eq!(player.health, BASE_MAX_HEALTH);
                assert!(player.counter.is_some());
            } else {
                assert_eq!(
                    event.result,
                    StrikeResult::Hit {
                        damage: archer().damage,
                        defeated: false
                    },
                    "facing the archer does not block a shot arriving from behind"
                );
                assert_eq!(player.guard.points, MAX_GUARD_POINTS);
                assert_eq!(player.counter, None);
            }
        }
    }

    fn heavy_scenario() -> (ArpgGame, u32) {
        let mut game = ArpgGame::new_scenario(ScenarioId::Heavy, 42).unwrap();
        game.add_player(1).unwrap();
        let id = game
            .monsters
            .iter()
            .find(|m| *content().id(m.definition().id) == *BRUISER)
            .unwrap()
            .id;
        (game, id)
    }

    fn monster_by_id(snapshot: &ArpgSnapshot, id: u32) -> &MonsterSnapshot {
        snapshot.monsters.iter().find(|m| m.id == id).unwrap()
    }

    #[test]
    fn a_bruiser_commits_to_its_long_telegraph_and_breaks_a_raised_guard() {
        let definition = bruiser();
        let (mut game, id) = heavy_scenario();
        command_as(&mut game, 1, ArpgCommand::SetGuard { raised: true });
        game.advance_tick().unwrap();
        let telegraph = game.snapshot().unwrap();
        let start = monster_by_id(&telegraph, id).clone();
        let action = start.action.unwrap();
        assert_eq!(action.phase, ActionPhase::Windup);
        assert_eq!(action.ticks_remaining, definition.windup_ticks);
        assert_eq!(i64::from(action.range), definition.strike.reach);
        assert_eq!(action.delivery, AttackDelivery::Strike);
        // The whole wind-up holds still and lands nothing.
        for _ in 1..definition.windup_ticks {
            game.advance_tick().unwrap();
            let snapshot = game.snapshot().unwrap();
            let monster = monster_by_id(&snapshot, id);
            assert_eq!(monster.position, start.position);
            assert_eq!(monster.behavior, MonsterBehavior::Attacking);
            assert!(snapshot.strike_events.is_empty());
        }
        game.advance_tick().unwrap();
        let impact = game.snapshot().unwrap();
        assert_eq!(
            monster_shots(&impact, id)[0].result,
            StrikeResult::GuardBroken
        );
        assert_eq!(impact.players[0].health, BASE_MAX_HEALTH);
        assert_eq!(impact.players[0].guard_points, 0);
    }

    #[test]
    fn stepping_out_of_a_bruiser_telegraph_makes_the_slam_miss() {
        let definition = bruiser();
        let (mut game, id) = heavy_scenario();
        game.advance_tick().unwrap();
        command_as(&mut game, 1, ArpgCommand::SetMovement { x: -1, z: 0 });
        let mut events = Vec::new();
        for _ in 0..definition.windup_ticks + 2 {
            game.advance_tick().unwrap();
            events.extend(monster_shots(&game.snapshot().unwrap(), id));
        }
        assert!(events.is_empty(), "{events:?}");
        assert_eq!(game.snapshot().unwrap().players[0].health, BASE_MAX_HEALTH);
    }

    #[test]
    fn interrupting_a_bruiser_windup_cancels_the_slam_and_it_winds_up_anew() {
        let definition = bruiser();
        let (mut game, id) = heavy_scenario();
        game.advance_tick().unwrap();
        let slam_tick = game.tick + u64::from(definition.windup_ticks);
        for _ in 0..10 {
            game.advance_tick().unwrap();
        }
        command_as(&mut game, 1, ArpgCommand::PrimaryAttack { aim: None });
        let mut interrupted = false;
        let mut slams = Vec::new();
        let mut rewound = None;
        while game.tick < slam_tick + 40 {
            game.advance_tick().unwrap();
            let snapshot = game.snapshot().unwrap();
            let monster = monster_by_id(&snapshot, id);
            interrupted |= monster.behavior == MonsterBehavior::Staggered;
            slams.extend(
                monster_shots(&snapshot, id)
                    .into_iter()
                    .map(|e| e.strike_tick),
            );
            if interrupted
                && rewound.is_none()
                && let Some(action) = monster.action
            {
                rewound = Some(action);
            }
        }
        assert!(interrupted);
        assert!(
            !slams.contains(&slam_tick),
            "the cancelled slam never lands: {slams:?}"
        );
        let rewound = rewound.expect("it winds up again");
        assert_eq!(rewound.phase, ActionPhase::Windup);
        assert_eq!(rewound.ticks_remaining, definition.windup_ticks);
        assert_eq!(rewound.target_player_id, 1);
    }

    /// Runs `game` and a JSON round trip of its save side by side for `ticks` ticks.
    fn assert_save_continues_identically(game: &mut ArpgGame, ticks: u32) {
        let json = serde_json::to_string(&game.save_state().unwrap()).unwrap();
        let mut restored = ArpgGame::from_save_state(serde_json::from_str(&json).unwrap()).unwrap();
        assert_eq!(restored.snapshot().unwrap(), game.snapshot().unwrap());
        for tick in 0..ticks {
            game.advance_tick().unwrap();
            restored.advance_tick().unwrap();
            assert_eq!(
                restored.snapshot().unwrap(),
                game.snapshot().unwrap(),
                "tick {tick}"
            );
        }
    }

    #[test]
    fn saves_mid_shot_mid_retreat_and_mid_telegraph_continue_like_the_uninterrupted_game() {
        let mut flying = ranged_scenario();
        advance_until_shot(&mut flying);
        flying.advance_tick().unwrap();
        assert_save_continues_identically(&mut flying, 150);

        let mut retreating = ranged_scenario();
        command_as(&mut retreating, 1, ArpgCommand::SetMovement { x: 1, z: 0 });
        let found = (0..200).any(|_| {
            retreating.advance_tick().unwrap();
            retreating.monsters.iter().any(|m| m.retreating)
        });
        assert!(found, "the archer backs off from the advancing player");
        assert_save_continues_identically(&mut retreating, 150);

        let (mut telegraph, _) = heavy_scenario();
        for _ in 0..20 {
            telegraph.advance_tick().unwrap();
        }
        assert_save_continues_identically(&mut telegraph, 150);
    }

    #[test]
    fn saved_shots_and_retreats_are_validated_against_the_monster_definition() {
        let mut game = ranged_scenario();
        advance_until_shot(&mut game);
        game.advance_tick().unwrap();
        let save = game.save_state().unwrap();
        assert!(ArpgGame::from_save_state(save.clone()).is_ok());
        let invalid = |mutate: &dyn Fn(&mut ArpgSaveState)| {
            let mut save = save.clone();
            mutate(&mut save);
            ArpgGame::from_save_state(save).is_err()
        };
        assert!(invalid(&|save| save.arrows[0].damage += 1));
        assert!(invalid(&|save| save.arrows[0].velocity[0] *= 2.0));
        assert!(invalid(&|save| save.arrows[0].ticks_remaining -= 1));
        assert!(invalid(
            &|save| save.arrows[0].source = StrikeSource::Monster(1)
        ));
        assert!(invalid(
            &|save| save.arrows[0].source = StrikeSource::Monster(99)
        ));
        assert!(invalid(&|save| {
            let monster = save.monsters.iter_mut().find(|m| m.room_id == 3).unwrap();
            monster.retreating = true;
            monster.steered = false;
        }));
        assert!(invalid(&|save| {
            let brute = save.monsters.iter_mut().find(|m| m.room_id == 2).unwrap();
            brute.engagement = Engagement::Engaged {
                target_player_id: 1,
            };
            brute.steered = true;
            brute.retreating = true;
        }));
    }

    #[test]
    fn role_scenarios_replay_identically_and_show_their_role() {
        let reproduction = |scenario, commands: Vec<(u64, ArpgCommand)>| Reproduction {
            scenario,
            seed: 42,
            players: vec![1],
            ticks: 240,
            commands: commands
                .into_iter()
                .enumerate()
                .map(|(index, (tick, command))| ReproductionCommand {
                    tick,
                    player_id: 1,
                    sequence: u32::try_from(index + 1).unwrap(),
                    command,
                })
                .collect(),
            operations: Vec::new(),
        };
        let ranged = reproduction(
            ScenarioId::Ranged,
            vec![
                (60, ArpgCommand::SetMovement { x: 1, z: 0 }),
                (100, ArpgCommand::SetMovement { x: 0, z: 0 }),
            ],
        );
        let heavy = reproduction(
            ScenarioId::Heavy,
            vec![(20, ArpgCommand::PrimaryAttack { aim: None })],
        );
        for (case, expected) in [
            (
                &ranged,
                [MonsterBehavior::Retreating, MonsterBehavior::Attacking],
            ),
            (
                &heavy,
                [MonsterBehavior::Staggered, MonsterBehavior::Attacking],
            ),
        ] {
            let first = replay_reproduction(case).unwrap();
            assert_eq!(first, replay_reproduction(case).unwrap());
            let behaviours = first
                .iter()
                .flat_map(|snapshot| &snapshot.monsters)
                .map(|monster| monster.behavior)
                .collect::<Vec<_>>();
            for behaviour in expected {
                assert!(
                    behaviours.contains(&behaviour),
                    "{:?}: {behaviour:?}",
                    case.scenario
                );
            }
        }
        let bolts = replay_reproduction(&ranged)
            .unwrap()
            .iter()
            .flat_map(|snapshot| snapshot.strike_events.clone())
            .filter(|event| event.definition == "monster.bolt")
            .count();
        assert!(bolts >= 2, "{bolts}");
    }

    // -----------------------------------------------------------------------------------
    // Jumping and support (#59)
    // -----------------------------------------------------------------------------------

    /// Sends a jump press and advances `ticks` ticks.
    fn jump_for(game: &mut ArpgGame, ticks: u32) {
        command(game, ArpgCommand::Jump);
        for _ in 0..ticks {
            game.advance_tick().unwrap();
        }
    }

    fn airborne(game: &ArpgGame) -> bool {
        !game.players[&1].grounded
    }

    #[test]
    fn countering_needs_the_ground_and_an_airborne_press_keeps_the_opportunity() {
        let (mut game, _, _) = blocked_once(120);
        game.players.get_mut(&1).unwrap().guard.held = false;
        jump_for(&mut game, 2);
        assert!(airborne(&game));
        let opportunity = game.players[&1].counter;
        assert!(opportunity.is_some_and(|counter| counter.usable_at(game.tick)));
        command(&mut game, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(action_kind(&game), Some(ActionKind::PrimaryAttack));
        assert_eq!(game.players[&1].counter, opportunity);
    }

    #[test]
    fn an_airborne_player_is_struck_and_strikes_as_on_the_ground() {
        // A monster's claw lands on a player in the air: no invulnerability.
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 120, z);
        jump_for(&mut game, 12);
        assert!(game.players[&1].position.y > PLAYER_Y as f32 + 60.0);
        assert_eq!(claw(&mut game, 1), CLAW_HIT);

        // And a light swing from the air hits the monster in front at the same reach.
        let (mut game, x, z) = strike_arena();
        place_monster(&mut game, 1, x + 150, z);
        jump_for(&mut game, 6);
        assert!(airborne(&game));
        let outcomes = strike(&mut game, 1_000, ArpgCommand::PrimaryAttack { aim: None });
        assert_eq!(targets(&outcomes), [(StrikeTarget::Monster(1), LIGHT_HIT)]);
    }

    #[test]
    fn shots_meet_bodies_at_any_height() {
        // An arrow loosed at the top of a jump flies at that height and still hits.
        let (mut game, x, z) = bow_arena();
        place_monster(&mut game, 1, x + 600, z);
        draw_for(&mut game, BOW_FULL_DRAW_TICKS);
        jump_for(&mut game, 14);
        assert!(airborne(&game));
        command(&mut game, ArpgCommand::ReleaseBow { aim: None });
        game.advance_tick().unwrap();
        let arrow = game.arrows[0];
        assert!(arrow.position.y > PLAYER_Y as f32 + 60.0, "{arrow:?}");
        let mut hit = Vec::new();
        for _ in 0..20 {
            game.advance_tick().unwrap();
            hit.extend(
                game.strike_outcomes()
                    .iter()
                    .filter(|outcome| outcome.definition == "bow.arrow")
                    .map(|outcome| outcome.target),
            );
        }
        assert_eq!(hit, [StrikeTarget::Monster(1)]);

        // An archer's bolt meets a player in the air.
        let mut game = ranged_scenario();
        advance_until_shot(&mut game);
        jump_for(&mut game, 0);
        let mut landed = None;
        for _ in 0..40 {
            game.advance_tick().unwrap();
            if let Some(event) = game
                .strike_outcomes()
                .iter()
                .find(|outcome| outcome.target == StrikeTarget::Player(1))
            {
                landed = Some((event.result, game.players[&1].position.y));
                break;
            }
        }
        let (result, height) = landed.expect("the bolt lands");
        assert!(matches!(result, StrikeResult::Hit { .. }), "{result:?}");
        assert!(height > PLAYER_Y as f32 + 20.0, "hit at {height}");

        // A save with an arrow in flight at jump height restores and continues.
        let mut game = ArpgGame::new_with_seed(77).unwrap();
        game.add_player(1).unwrap();
        command(
            &mut game,
            ArpgCommand::EquipWeapon {
                weapon: Weapon::Bow,
            },
        );
        draw_for(&mut game, BOW_FULL_DRAW_TICKS);
        jump_for(&mut game, 14);
        command(&mut game, ArpgCommand::ReleaseBow { aim: None });
        game.advance_tick().unwrap();
        game.advance_tick().unwrap();
        assert!(game.arrows[0].position.y > PLAYER_Y as f32 + 60.0);
        let mut restored = ArpgGame::from_save_state(game.save_state().unwrap()).unwrap();
        for _ in 0..10 {
            game.advance_tick().unwrap();
            restored.advance_tick().unwrap();
        }
        assert_eq!(restored.save_state().unwrap(), game.save_state().unwrap());
        let mut tampered = game.save_state().unwrap();
        tampered.arrows[0].position[1] = PLAYER_Y as f32 + 300.0;
        assert!(ArpgGame::from_save_state(tampered).is_err());
    }

    #[test]
    fn a_player_defeated_in_the_air_falls_lands_and_cannot_jump() {
        let mut game = ArpgGame::new_with_seed(23).unwrap();
        game.add_player(1).unwrap();
        jump_for(&mut game, 10);
        let height = game.players[&1].position.y;
        assert!(height > PLAYER_Y as f32 + 50.0);
        game.players.get_mut(&1).unwrap().health = 0;
        command(&mut game, ArpgCommand::Jump);
        assert_eq!(game.players[&1].jump_buffer_ticks_remaining, 0);
        let mut previous = height;
        let mut landed = false;
        for _ in 0..60 {
            game.advance_tick().unwrap();
            let state = game.players[&1];
            if state.grounded {
                landed = true;
                assert!(state.position.y - (PLAYER_Y as f32) < 6.0);
            } else {
                // Past the defeat the body only keeps its jump's arc.
                assert!(state.velocity.x == 0.0 && state.velocity.z == 0.0);
            }
            assert!(state.position.y <= previous + 10.0);
            previous = state.position.y;
        }
        assert!(landed);
        assert!((game.players[&1].position.y - PLAYER_Y as f32).abs() < 1.0e-3);
        assert!(!game.snapshot().unwrap().players[0].alive);
        // The dead body never takes off again.
        command(&mut game, ArpgCommand::Jump);
        game.advance_tick().unwrap();
        assert!(game.players[&1].grounded);
        // A save of the dead, landed body restores.
        ArpgGame::from_save_state(game.save_state().unwrap()).unwrap();
    }

    /// A player standing on a 40-unit block (a test ledge; ledges are content of a later
    /// slice) in the start room, walking off its +x edge.
    fn on_a_ledge() -> ArpgGame {
        let mut game = ArpgGame::new_with_seed(31).unwrap();
        game.add_player(1).unwrap();
        let spawn = game.players[&1].position;
        game.physics
            .add_fixed(
                STATIC_BODY_BASE + 990,
                Vec3i::new(spawn.x as i32, 0, spawn.z as i32),
                Vec3i::new(60, 40, 200),
            )
            .unwrap();
        game.put_player(1, spawn + Vector::new(0.0, 41.0, 0.0));
        let state = game.players.get_mut(&1).unwrap();
        state.grounded = false;
        // Settle onto the block.
        for _ in 0..10 {
            game.advance_tick().unwrap();
        }
        assert!(game.players[&1].grounded, "{:?}", game.players[&1]);
        command(&mut game, ArpgCommand::SetMovement { x: 1, z: 0 });
        game
    }

    #[test]
    fn walking_off_the_ground_opens_the_coyote_window() {
        // Walk until the body leaves the block's top without jumping.
        let mut game = on_a_ledge();
        let left = (0..120)
            .find(|_| {
                game.advance_tick().unwrap();
                airborne(&game)
            })
            .expect("the body walks off the block");
        assert!(left > 0);
        assert_eq!(game.players[&1].coyote_ticks_remaining, COYOTE_TICKS);
        assert!(game.players[&1].velocity.y <= 0.0);
        let fork = game.save_state().unwrap();

        // A press on each tick of the window still takes off; one after it does not.
        for late in 0..=COYOTE_TICKS {
            let mut game = ArpgGame::from_save_state(fork.clone()).unwrap();
            for _ in 0..late {
                game.advance_tick().unwrap();
            }
            jump_for(&mut game, 1);
            let rising = game.players[&1].velocity.y > 0.0;
            assert_eq!(rising, late < COYOTE_TICKS, "pressed {late} ticks late");
        }
    }

    /// A strike arena whose topology adds an upper region 2 above the floor and a drop (3)
    /// from it, with player 1 in `membership` (#207).
    fn two_region_arena(membership: RegionMembership) -> (ArpgGame, i32, i32) {
        let (mut game, x, z) = strike_arena();
        game.topology = Topology::new(
            vec![
                WalkableRegion {
                    id: DUNGEON_FLOOR_REGION,
                    foot_y: FLOOR_FOOT_Y,
                    area: None,
                },
                WalkableRegion {
                    id: 2,
                    foot_y: 300,
                    area: None,
                },
            ],
            vec![TraversalConnection {
                id: 3,
                kind: ConnectionKind::Drop,
                from: 2,
                to: DUNGEON_FLOOR_REGION,
            }],
        )
        .unwrap();
        // Player 1 stands on a platform whose top is region 2's floor.
        game.physics
            .add_fixed(
                STATIC_BODY_BASE + 999,
                Vec3i::new(x, 150, z),
                Vec3i::new(40, 150, 40),
            )
            .unwrap();
        game.put_player(1, Vec3i::new(x, 300 + PLAYER_HALF_EXTENTS.y, z));
        game.players.get_mut(&1).unwrap().region = membership;
        (game, x, z)
    }

    #[test]
    fn the_generated_dungeon_is_one_floor_region_every_player_belongs_to() {
        let mut game = ArpgGame::new_with_seed(42).unwrap();
        game.add_player(1).unwrap();
        assert_eq!(game.topology, Topology::single_floor(FLOOR_FOOT_Y));
        let floor = RegionMembership::Region {
            id: DUNGEON_FLOOR_REGION,
        };
        assert_eq!(game.snapshot().unwrap().players[0].region, floor);
        // Jumping keeps the membership in the air and on landing.
        game.apply_command(PlayerCommand::new(1, 1, ArpgCommand::Jump).unwrap())
            .unwrap();
        for _ in 0..60 {
            game.advance_tick().unwrap();
            assert_eq!(game.players[&1].region, floor);
        }
        assert!(game.players[&1].grounded);
    }

    #[test]
    fn ordinary_combat_targeting_and_interaction_are_region_local() {
        let upper = RegionMembership::Region { id: 2 };
        let (mut game, x, z) = two_region_arena(upper);
        place_monster(&mut game, 1, x + 100, z);
        // The monster stands in reach on the plane, but in another region.
        let outcomes = strike(&mut game, 1, ArpgCommand::PrimaryAttack { aim: None });
        assert!(targets(&outcomes).is_empty(), "{outcomes:?}");
        assert_eq!(monster_health(&game, 1), 100);
        let monster = game.monsters[0];
        let origin = game.players[&1].position;
        assert!(!game.monster_lockable(&monster, upper, origin, content().targeting.lock_range));
        assert!(game.monster_targets().is_empty());
        let contacts = game
            .strike_contacts(
                monster_claw(),
                StrikeTarget::Monster(1),
                monster.position,
                (0.0, 0.0),
                [(StrikeTarget::Player(1), origin)],
            )
            .unwrap();
        assert!(contacts.is_empty());
        assert!(game.interaction_candidates(upper, origin).is_empty());
    }

    #[test]
    fn an_actor_in_transit_stays_reachable_from_both_ends() {
        let transit = RegionMembership::Transit { connection: 3 };
        let (mut game, x, z) = two_region_arena(transit);
        place_monster(&mut game, 1, x + 100, z);
        let origin = game.players[&1].position;
        let monster = game.monsters[0];
        let contacts = game
            .strike_contacts(
                monster_claw(),
                StrikeTarget::Monster(1),
                monster.position,
                (0.0, 0.0),
                [(StrikeTarget::Player(1), origin)],
            )
            .unwrap();
        // Eligible from the floor end; the platform itself stands in the strike's way.
        assert_eq!(contacts, [(StrikeTarget::Player(1), true)]);
        assert_eq!(game.monster_targets().len(), 1);
        assert!(game.monster_lockable(&monster, transit, origin, content().targeting.lock_range));
        // Landing on the platform ends the transit in region 2.
        game.advance_tick().unwrap();
        assert_eq!(game.players[&1].region, RegionMembership::Region { id: 2 });
    }

    #[test]
    fn saves_keep_the_region_and_reject_an_unknown_or_unsupported_one() {
        let mut game = ArpgGame::new_with_seed(91).unwrap();
        game.add_player(1).unwrap();
        let save = game.save_state().unwrap();
        assert_eq!(
            save.players[0].region,
            RegionMembership::Region {
                id: DUNGEON_FLOOR_REGION
            }
        );
        let restored = ArpgGame::from_save_state(save.clone()).unwrap();
        assert_eq!(restored.save_state().unwrap(), save);
        for region in [
            RegionMembership::Region { id: 2 },
            RegionMembership::Transit { connection: 1 },
        ] {
            let mut tampered = save.clone();
            tampered.players[0].region = region;
            assert_eq!(
                ArpgGame::from_save_state(tampered).unwrap_err().message(),
                "saved player region is invalid"
            );
        }
    }

    #[test]
    fn monsters_walk_under_a_low_ceiling_but_around_a_platform() {
        let game = ArpgGame::arranged_scenario(ScenarioId::Traversal, 42).unwrap();
        let footprint = |kind| {
            let collider = game
                .static_colliders
                .iter()
                .find(|collider| collider.kind == kind)
                .unwrap();
            Rect::centered(
                collider.position[0],
                collider.position[2],
                collider.half_extents[0],
                collider.half_extents[2],
            )
        };
        let obstacles = game.fixed_footprints();
        assert!(obstacles.contains(&footprint(StaticColliderKind::Platform)));
        assert!(!obstacles.contains(&footprint(StaticColliderKind::Ceiling)));
    }

    #[test]
    fn a_player_on_the_bridge_and_a_monster_on_the_path_below_never_meet_in_melee() {
        let mut game = ArpgGame::arranged_scenario(ScenarioId::Traversal, 42).unwrap();
        let [x, _, z] = game
            .static_colliders
            .iter()
            .filter(|collider| collider.kind == StaticColliderKind::Platform)
            .max_by_key(|collider| collider.position[1])
            .unwrap()
            .position;
        game.add_player(1).unwrap();
        game.put_player(
            1,
            Vec3i::new(x, TRAVERSAL_BRIDGE_TOP + PLAYER_HALF_EXTENTS.y, z),
        );
        game.advance_tick().unwrap();
        assert_eq!(
            game.players[&1].region,
            RegionMembership::Region {
                id: TRAVERSAL_BRIDGE_REGION
            }
        );
        game.monsters.clear();
        // Right below, on the floor path: in reach on the plane, in another region.
        place_monster(&mut game, 1, x + 60, z);
        let outcomes = strike(&mut game, 1, ArpgCommand::PrimaryAttack { aim: None });
        assert!(targets(&outcomes).is_empty(), "{outcomes:?}");
        assert_eq!(monster_health(&game, 1), 100);
        assert!(game.monster_targets().is_empty());
    }

    #[test]
    fn the_traversal_layout_rejects_an_undeclared_crossing() {
        let mut game = ArpgGame::arranged_scenario(ScenarioId::Traversal, 42).unwrap();
        assert!(game.validate_topology().is_ok());
        // Without the jump between platform and bridge the geometry permits an undeclared
        // crossing.
        let connections = game
            .topology
            .connections()
            .iter()
            .copied()
            .filter(|connection| connection.id != 3)
            .collect();
        game.topology = Topology::new(game.topology.regions().to_vec(), connections).unwrap();
        let error = game.validate_topology().unwrap_err();
        assert!(
            error.message().contains("region 2 can reach region 3"),
            "{error:?}"
        );
    }

    /// Runs `test` on Rapier and, with `physics-ab`, on physics-engine as well.
    fn on_each_backend(test: impl Fn()) {
        #[cfg(feature = "physics-ab")]
        for backend in physics::Backend::ALL {
            physics::with_backend(backend, &test);
        }
        #[cfg(not(feature = "physics-ab"))]
        test();
    }

    #[test]
    fn a_player_out_of_the_world_returns_to_its_last_supported_position_unharmed() {
        on_each_backend(|| {
            let mut game = ArpgGame::new_with_seed(42).unwrap();
            game.add_player(1).unwrap();
            game.apply_command(
                PlayerCommand::new(1, 1, ArpgCommand::SetMovement { x: 1, z: 0 }).unwrap(),
            )
            .unwrap();
            for _ in 0..10 {
                game.advance_tick().unwrap();
            }
            game.apply_command(PlayerCommand::new(1, 2, ArpgCommand::Jump).unwrap())
                .unwrap();
            game.advance_tick().unwrap();
            game.advance_tick().unwrap();
            let state = game.players[&1];
            assert!(!state.grounded);
            let takeoff = state.last_supported;
            assert_eq!(takeoff.y, PLAYER_Y as f32);
            let health = state.health;
            // Mid-jump, the body ends up far outside the dungeon (as a tunnelling body
            // would): the next tick brings it back.
            let outside = Vector::new(100_000.0, state.position.y, state.position.z);
            game.put_player(1, outside);
            game.advance_tick().unwrap();
            let state = game.players[&1];
            assert_eq!(state.position, takeoff);
            assert_eq!(
                game.physics.character_position(ArpgGame::player_body_id(1)),
                Some(takeoff)
            );
            assert!(state.grounded);
            assert_eq!(state.velocity, Vector::ZERO);
            assert_eq!(state.health, health);
            assert_eq!(
                state.region,
                RegionMembership::Region {
                    id: DUNGEON_FLOOR_REGION
                }
            );
            // It plays on from there, and a save taken now continues exactly.
            let save = game.save_state().unwrap();
            let mut restored = ArpgGame::from_save_state(save).unwrap();
            for _ in 0..30 {
                game.advance_tick().unwrap();
                restored.advance_tick().unwrap();
                assert_eq!(restored.save_state().unwrap(), game.save_state().unwrap());
            }
            assert!(!game.out_of_world(game.players[&1].position));
        });
    }

    #[test]
    fn a_body_far_below_the_floor_is_out_of_the_world_and_one_on_it_is_not() {
        let game = ArpgGame::new_with_seed(42).unwrap();
        let (x, z) = game.rooms[0].center();
        let at = |y: f32| Vector::new(x as f32, y, z as f32);
        assert!(!game.out_of_world(at(PLAYER_Y as f32)));
        assert!(!game.out_of_world(at(PLAYER_Y as f32 - 99.0)));
        assert!(game.out_of_world(at(PLAYER_Y as f32 - 101.0)));
        assert!(game.out_of_world(at(f32::NAN)));
    }

    #[test]
    fn saves_reject_a_last_supported_position_out_of_the_world() {
        let mut game = ArpgGame::new_with_seed(91).unwrap();
        game.add_player(1).unwrap();
        let mut save = game.save_state().unwrap();
        save.players[0].last_supported = [100_000.0, PLAYER_Y as f32, 0.0];
        assert_eq!(
            ArpgGame::from_save_state(save).unwrap_err().message(),
            "saved player last supported position is invalid"
        );
    }
}
