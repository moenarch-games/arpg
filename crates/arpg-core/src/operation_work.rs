//! Deterministic operation work counters for `.performance/contract.json` operations (#97).
//!
//! Gameplay code reports work through [`count`] and [`snapshot_requested`]. Without the
//! `operation-work` feature both are empty inline functions, so product builds carry no
//! counter state and no counting cost. With it, counts accumulate per thread until
//! [`OperationWork::take`]. Physics passes are counted by [`ArpgGame::physics_steps`]; their
//! cost is part of the complete tick the performance journey measures.

#[cfg(feature = "operation-work")]
pub mod contract;

#[cfg(feature = "operation-work")]
use super::{
    ArpgGame, AuthoritativeGame, DungeonRng, GameError, MAX_ENCOUNTER_SPAWNS, MAX_PLAYERS,
    PLAYER_Y, RoomKind, RoomSnapshot, RunSeed, Vec3i, spawn_position,
};

/// Work kinds counted by gameplay code.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Counter {
    /// One engagement-policy decision of a free monster in an active encounter.
    MonsterDecision,
    /// One target candidate scored by a decision.
    MonsterTargetEvaluation,
    /// One command validation or lane watermark lookup or update.
    CommandBookkeeping,
}

#[cfg(not(feature = "operation-work"))]
#[inline(always)]
pub(crate) fn count(_counter: Counter, _amount: u64) {}

#[cfg(not(feature = "operation-work"))]
#[inline(always)]
pub(crate) fn snapshot_requested(_game: &super::ArpgGame) {}

/// Deterministic work counted on the current thread since the last [`OperationWork::take`].
#[cfg(feature = "operation-work")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OperationWork {
    /// Authoritative snapshots built (`AuthoritativeGame::snapshot`).
    pub snapshot_materializations: u64,
    /// Entity records written into those snapshots.
    pub snapshot_entities_materialized: u64,
    /// Engagement-policy decisions of free monsters in active encounters.
    pub monster_decisions: u64,
    /// Target candidates those decisions scored.
    pub monster_target_evaluations: u64,
    /// Player validation, lane sequence checks and lane watermark records.
    pub command_bookkeeping_steps: u64,
}

#[cfg(feature = "operation-work")]
thread_local! {
    static WORK: std::cell::Cell<OperationWork> = const {
        std::cell::Cell::new(OperationWork {
            snapshot_materializations: 0,
            snapshot_entities_materialized: 0,
            monster_decisions: 0,
            monster_target_evaluations: 0,
            command_bookkeeping_steps: 0,
        })
    };
}

#[cfg(feature = "operation-work")]
impl OperationWork {
    /// Returns the counters of this thread and resets them.
    pub fn take() -> Self {
        WORK.take()
    }

    fn add(update: impl FnOnce(&mut Self)) {
        WORK.with(|cell| {
            let mut work = cell.get();
            update(&mut work);
            cell.set(work);
        });
    }
}

#[cfg(feature = "operation-work")]
pub(crate) fn count(counter: Counter, amount: u64) {
    OperationWork::add(|work| {
        let total = match counter {
            Counter::MonsterDecision => &mut work.monster_decisions,
            Counter::MonsterTargetEvaluation => &mut work.monster_target_evaluations,
            Counter::CommandBookkeeping => &mut work.command_bookkeeping_steps,
        };
        *total = total.saturating_add(amount);
    });
}

/// Counts one snapshot of `game` and the entity records it exports, collection by
/// collection as `AuthoritativeGame::snapshot` writes them.
#[cfg(feature = "operation-work")]
pub(crate) fn snapshot_requested(game: &ArpgGame) {
    let locked_doors = game.doors.iter().filter(|door| door.locked).count();
    let entities = game.rooms.len()
        + game.doors.len()
        + game.players.len()
        + game.monsters.len()
        + game.ground_loot.len()
        + game.static_colliders.len()
        + locked_doors
        + game.arrows.len()
        + game.chests.len()
        + game.strike_outcomes.len()
        + game.interaction_events.len();
    OperationWork::add(|work| {
        work.snapshot_materializations = work.snapshot_materializations.saturating_add(1);
        work.snapshot_entities_materialized = work
            .snapshot_entities_materialized
            .saturating_add(u64::try_from(entities).unwrap_or(u64::MAX));
    });
}

/// A seeded dungeon arranged for operation work measurements: `players` stand in the first
/// combat room with `engaged_monsters` in front of them, inside aggro range, and
/// `unrelated_monsters` wait in the other, dormant combat rooms. Monster ids are
/// `1..=engaged_monsters`, then the unrelated ones.
#[cfg(feature = "operation-work")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkFixture {
    pub players: u32,
    pub engaged_monsters: u32,
    pub unrelated_monsters: u32,
}

#[cfg(feature = "operation-work")]
impl WorkFixture {
    /// Engaged monsters stand in one row this far apart.
    const ENGAGED_SPACING: i32 = 100;
    /// Every room holds at most one encounter's worth of monsters, so each fixture world is
    /// one the content could produce: the engaged group is one encounter, and the unrelated
    /// population fills the dormant rooms up to this many each.
    pub const MAX_MONSTERS_PER_ROOM: u32 = MAX_ENCOUNTER_SPAWNS as u32;

    pub fn build(&self, seed: RunSeed) -> Result<ArpgGame, GameError> {
        let players = usize::try_from(self.players).unwrap_or(usize::MAX);
        if players == 0 || players > MAX_PLAYERS {
            return Err(GameError::new(format!(
                "a work fixture holds 1 to {MAX_PLAYERS} players"
            )));
        }
        if self.engaged_monsters == 0 || self.engaged_monsters > Self::MAX_MONSTERS_PER_ROOM {
            return Err(GameError::new(format!(
                "a work fixture engages 1 to {} monsters",
                Self::MAX_MONSTERS_PER_ROOM
            )));
        }
        let mut game = ArpgGame::new_with_seed(seed)?;
        let combat_rooms = game
            .rooms
            .iter()
            .filter(|room| room.kind == RoomKind::Combat)
            .cloned()
            .collect::<Vec<RoomSnapshot>>();
        let [arena, dormant @ ..] = combat_rooms.as_slice() else {
            return Err(GameError::new("the dungeon has no combat room"));
        };
        let rooms = u32::try_from(dormant.len()).unwrap_or(u32::MAX);
        if self.unrelated_monsters > rooms.saturating_mul(Self::MAX_MONSTERS_PER_ROOM) {
            return Err(GameError::new(format!(
                "{rooms} dormant rooms hold at most {} unrelated monsters",
                rooms.saturating_mul(Self::MAX_MONSTERS_PER_ROOM)
            )));
        }
        let template = *game
            .monsters
            .iter()
            .find(|monster| monster.room_id == arena.id)
            .ok_or_else(|| GameError::new("the arena has no generated monster"))?;
        let (center_x, center_z) = arena.center();
        for (index, spawn) in (0_i32..).zip(game.player_spawns.iter_mut()) {
            *spawn = Vec3i::new(center_x - 120 + index * 80, PLAYER_Y, center_z - 150);
        }

        let engaged = i32::try_from(self.engaged_monsters).unwrap_or(i32::MAX);
        let mut monsters = Vec::new();
        for (id, index) in (1_u32..).zip(0..engaged) {
            let offset = (2 * index - (engaged - 1)) * Self::ENGAGED_SPACING / 2;
            monsters.push(super::MonsterState {
                id,
                room_id: arena.id,
                position: Vec3i::new(center_x + offset, PLAYER_Y, center_z + 250).into(),
                ..template
            });
        }
        // Unrelated monsters go round-robin over the dormant rooms, placed like a generated
        // spawn group: apart from each other and from fixed colliders.
        let mut rng = DungeonRng::new(u64::from(seed));
        let mut placed = vec![Vec::new(); dormant.len()];
        for index in 0..self.unrelated_monsters {
            let slot = usize::try_from(index % rooms).unwrap_or_default();
            let room = &dormant[slot];
            let position = spawn_position(room, &placed[slot], &game.static_colliders, &mut rng);
            placed[slot].push(position);
            monsters.push(super::MonsterState {
                id: self.engaged_monsters + 1 + index,
                room_id: room.id,
                position: position.into(),
                ..template
            });
        }
        game.monsters = monsters;
        for player_id in 1..=self.players {
            game.add_player(player_id)?;
        }
        Ok(game)
    }
}
