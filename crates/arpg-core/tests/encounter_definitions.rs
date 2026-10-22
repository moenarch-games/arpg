//! Acceptance for room encounter definitions (#66, first slice: "Make encounter
//! definitions describe a bounded spawn group, activation condition, completion condition
//! and reward reference while preserving the current dormant/active/cleared semantics").
//!
//! # Contract
//!
//! ## Content schema (`formatVersion` 8)
//!
//! The bundle replaces `roomMonsters` with encounter definitions referenced by the combat
//! rooms, and moves the chest's gold out of `loot` into a reward definition:
//!
//! ```json
//! "encounters": [
//!   { "id": "encounter.brute",
//!     "spawnGroup": [ { "monster": "monster.brute", "count": 1 } ],
//!     "activation": "playerEntersRoom",
//!     "completion": "spawnGroupDefeated",
//!     "reward": "reward.roomChest" }
//! ],
//! "rewards": [ { "id": "reward.roomChest", "kind": "chest", "gold": 25 } ],
//! "roomEncounters": ["encounter.brute", "..."],
//! "loot": { "monsterGold": 10 }
//! ```
//!
//! - `encounters` and `rewards` are canonicalised by id (source order is irrelevant);
//!   `roomEncounters` (combat rooms in generation order, repeating from the start when rooms
//!   outnumber entries, exactly like `roomMonsters` did) and each `spawnGroup` are authored
//!   order and part of the revision.
//! - `activation` is the closed vocabulary `"playerEntersRoom"` (the existing rule: a dormant
//!   combat room becomes active when a player's body is in it). `completion` is the closed
//!   vocabulary `"spawnGroupDefeated"` (the existing rule: an active room is cleared when no
//!   member of its group lives). `kind` is the closed vocabulary `"chest"` (the existing
//!   room chest, locked until the room is cleared and opened once, paying `gold`). Unknown
//!   variants, unknown fields and the removed `roomMonsters`/`loot.chestGold` fail to parse.
//! - Spawning: the combat room's encounter spawns every member of its group, entries in
//!   authored order, `count` times each. Monster ids are assigned 1, 2, … over combat rooms
//!   in generation order and members in group order; each member draws its position from
//!   the dungeon RNG as the single room monster does today, so the built-in single-member
//!   groups generate exactly the previous dungeon. Members never overlap each other or a
//!   fixed collider.
//! - Validation fails closed and reports every violation as `"<path>: <message>"` with the
//!   path `encounters[<i>] <id>`, `rewards[<i>] <id>` or `roomEncounters[<i>] <id>` (or the
//!   bare collection name), containing these message fragments:
//!   unknown monster → `unknown monster`; unknown reward → `unknown reward`; unknown room
//!   encounter → `unknown encounter`; empty group → `empty`; zero count → `count must be
//!   positive`; more than [`MAX_ENCOUNTER_SPAWNS`] members in total → `at most <MAX>`;
//!   the same monster listed twice in one group → `more than once`; duplicate ids →
//!   `duplicate encounter id` / `duplicate reward id`; an encounter or reward nothing
//!   references → `not referenced`; zero gold → `gold must be positive`; empty
//!   `roomEncounters` → `at least one`; bad id → `1..=64 ASCII`.
//!
//! ## Rust API
//!
//! - `pub const MAX_ENCOUNTER_SPAWNS: usize` (re-exported from `arpg_core`): the upper bound
//!   of a spawn group's total member count, within `1..=MAX_WORKBENCH_MONSTERS`.
//! - `ContentBundle` gains `encounters: Vec<EncounterData>`, `rewards: Vec<RewardData>` and
//!   `room_encounters: Vec<String>` (the authored room assignment) in place of
//!   `room_monsters`; `LootData` loses `chest_gold`. Tests read the JSON form, so the
//!   field types of the new definition structs are the implementer's choice.
//!   Monsters and strikes still need no reference: only encounters and rewards do.
//! - `ContentBundle::check(&self) -> Result<(), Vec<ContentError>>`: the same validation the
//!   built-in bundle passes at load, for any parsed bundle.
//! - `ArpgGame::new_with_content(bundle: ContentBundle, run_seed: RunSeed)
//!   -> Result<ArpgGame, GameError>`: the ordinary generated dungeon running `bundle`
//!   (validated first; an invalid bundle is refused with a message naming its violations).
//!   Its snapshots and saves carry `bundle.revision()`.
//! - `ArpgGame::from_save_state_with_content(save: ArpgSaveState, bundle: ContentBundle)
//!   -> Result<ArpgGame, GameError>`: restores a save made with `bundle`; like
//!   `from_save_state`, a save from another revision is refused.
//! - `RoomSnapshot::encounter: Option<String>`: the encounter definition id of a combat room
//!   (`None` for the start room), published like `MonsterSnapshot::definition` so
//!   presentation can key on it; gameplay never reads it back.
//!
//! The dormant/active/cleared states, door locks, chest availability and save fields are
//! unchanged. Run transitions, defeat/checkpoint recovery, objectives and co-op leave are
//! later slices of #66 and deliberately not specified here.
//!
//! Existing tests that keep proving the unchanged lifecycle and must stay green (adapted only
//! where they name the moved content fields): in `src/lib.rs`
//! `a_room_chest_stays_locked_until_its_encounter_is_cleared_and_opens_once`,
//! `entering_combat_room_locks_connected_doors_until_encounter_is_cleared`,
//! `dormant_room_monsters_cannot_be_attacked_before_entry`,
//! `opened_chests_survive_saves_and_unknown_ones_are_rejected`,
//! `combat_rooms_spawn_the_monster_their_content_assigns`,
//! `base_content_reproduces_the_previous_tuning`,
//! `saves_from_another_content_revision_are_rejected`,
//! `save_state_round_trip_preserves_authoritative_state_and_continuation`; in
//! `src/workbench.rs` `the_encounter_follows_each_arrangement_change_at_once`; and
//! `tests/dungeon_properties.rs` (seed sweep and replay determinism).

use arpg_core::{
    ActionKind, ActionPhase, ArpgCommand, ArpgGame, ArpgSaveState, ArpgSnapshot, AuthoritativeGame,
    CONTENT_FORMAT_VERSION, ContentBundle, InteractionRefusal, InteractionResult,
    MAX_ENCOUNTER_SPAWNS, MAX_WORKBENCH_MONSTERS, PlayerActionSnapshot, PlayerCommand,
    RoomEncounterState, RoomKind, RoomSnapshot, base_bundle, content_revision,
};
use serde_json::{Value, json};

/// Distance from a monster at which an arranged light swing lands next tick.
const SIDE_DISTANCE: i32 = 120;
/// Clearance kept between arranged bodies and the room's walls.
const WALL_MARGIN: i32 = 80;
/// Both monster and player bodies are 30 units half extent on the plane.
const BODY_HALF: i32 = 30;

const PACK: &str = "encounter.fixture.pack";
const LONE: &str = "encounter.fixture.lone";
const HOARD: &str = "reward.fixture.hoard";
const PURSE: &str = "reward.fixture.purse";
const HOARD_GOLD: u32 = 70;
const PURSE_GOLD: u32 = 5;

// ---------------------------------------------------------------------------------------
// Content fixtures
// ---------------------------------------------------------------------------------------

fn base_json() -> Value {
    serde_json::to_value(base_bundle()).expect("base bundle serialises")
}

fn parse(value: &Value) -> ContentBundle {
    ContentBundle::from_json(&value.to_string()).expect("fixture parses")
}

fn encounter(id: &str, group: &[(&str, u32)], reward: &str) -> Value {
    json!({
        "id": id,
        "spawnGroup": group
            .iter()
            .map(|(monster, count)| json!({ "monster": monster, "count": count }))
            .collect::<Vec<_>>(),
        "activation": "playerEntersRoom",
        "completion": "spawnGroupDefeated",
        "reward": reward,
    })
}

fn reward(id: &str, gold: u32) -> Value {
    json!({ "id": id, "kind": "chest", "gold": gold })
}

/// A second, independently authored set of encounters: a three-member pack paying a hoard
/// and a lone bruiser paying a purse, alternating over the combat rooms. Only data differs
/// from the base bundle.
fn fixture_json() -> Value {
    let mut value = base_json();
    value["encounters"] = json!([
        encounter(
            PACK,
            &[("monster.skirmisher", 2), ("monster.archer", 1)],
            HOARD
        ),
        encounter(LONE, &[("monster.bruiser", 1)], PURSE),
    ]);
    value["rewards"] = json!([reward(HOARD, HOARD_GOLD), reward(PURSE, PURSE_GOLD)]);
    value["roomEncounters"] = json!([PACK, LONE]);
    value
}

fn fixture() -> ContentBundle {
    parse(&fixture_json())
}

/// Expected group of each combat room (generation order) under the fixture.
fn fixture_groups() -> Vec<(&'static str, Vec<&'static str>, u32)> {
    (0..5)
        .map(|index| {
            if index % 2 == 0 {
                (
                    PACK,
                    vec!["monster.skirmisher", "monster.skirmisher", "monster.archer"],
                    HOARD_GOLD,
                )
            } else {
                (LONE, vec!["monster.bruiser"], PURSE_GOLD)
            }
        })
        .collect()
}

/// Validation messages of the fixture after `mutate`; the mutated bundle must still parse.
fn violations(mutate: impl FnOnce(&mut Value)) -> Vec<String> {
    let mut value = fixture_json();
    mutate(&mut value);
    parse(&value)
        .check()
        .expect_err("mutated bundle must be rejected")
        .into_iter()
        .map(|error| error.to_string())
        .collect()
}

fn reports(errors: &[String], path: &str, id: &str, fragment: &str) -> bool {
    errors
        .iter()
        .any(|error| error.starts_with(path) && error.contains(id) && error.contains(fragment))
}

fn find<'a>(collection: &'a Value, id: &str) -> &'a Value {
    collection
        .as_array()
        .expect("collection is an array")
        .iter()
        .find(|entry| entry["id"] == id)
        .unwrap_or_else(|| panic!("{id} is defined"))
}

// ---------------------------------------------------------------------------------------
// Runtime driving (public API only)
// ---------------------------------------------------------------------------------------

fn fnv(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn spawn_table(snapshot: &ArpgSnapshot) -> String {
    snapshot
        .monsters
        .iter()
        .map(|m| format!("{}:{}:{}:{:?}", m.id, m.definition, m.room_id, m.position))
        .collect::<Vec<_>>()
        .join(";")
}

/// The gameplay outcome of a snapshot, restricted to fields that existed before this
/// slice, so the golden traces pin outcomes rather than the snapshot's shape.
fn projection(snapshot: &ArpgSnapshot) -> String {
    let players = snapshot
        .players
        .iter()
        .map(|p| (p.id, p.position, p.health, p.gold, p.experience, p.alive))
        .collect::<Vec<_>>();
    let monsters = snapshot
        .monsters
        .iter()
        .map(|m| {
            (
                m.id,
                m.definition.clone(),
                m.room_id,
                m.position,
                m.health,
                m.alive,
            )
        })
        .collect::<Vec<_>>();
    let rooms = snapshot
        .rooms
        .iter()
        .map(|r| (r.id, r.encounter_state))
        .collect::<Vec<_>>();
    let doors = snapshot
        .doors
        .iter()
        .map(|d| (d.id, d.locked))
        .collect::<Vec<_>>();
    let chests = snapshot
        .chests
        .iter()
        .map(|c| (c.id, c.room_id, c.position, c.opened, c.available))
        .collect::<Vec<_>>();
    let loot = snapshot
        .ground_loot
        .iter()
        .map(|l| (l.id, l.position, l.amount))
        .collect::<Vec<_>>();
    let events = snapshot
        .interaction_events
        .iter()
        .map(|e| (e.player_id, e.result))
        .collect::<Vec<_>>();
    format!(
        "{}|{players:?}|{monsters:?}|{rooms:?}|{doors:?}|{chests:?}|{loot:?}|{events:?}",
        snapshot.tick
    )
}

fn inside(room: &RoomSnapshot, x: i32, z: i32) -> bool {
    x >= room.min_x + WALL_MARGIN
        && x <= room.max_x - WALL_MARGIN
        && z >= room.min_z + WALL_MARGIN
        && z <= room.max_z - WALL_MARGIN
}

/// One player in a generated dungeon running the built-in bundle or `bundle`. Arrangements
/// go through the save boundary, so everything an encounter needs must survive it.
struct Session {
    game: ArpgGame,
    bundle: Option<ContentBundle>,
    sequence: u32,
    /// Running hash over the projection of every ticked snapshot.
    trace: u64,
}

impl Session {
    fn base(seed: u32) -> Self {
        let mut game = ArpgGame::new_with_seed(seed).expect("seeded dungeon constructs");
        game.add_player(1).expect("player joins");
        Self {
            game,
            bundle: None,
            sequence: 0,
            trace: 0,
        }
    }

    fn with_content(bundle: ContentBundle, seed: u32) -> Self {
        let mut game = ArpgGame::new_with_content(bundle.clone(), seed)
            .expect("a valid bundle runs a generated dungeon");
        game.add_player(1).expect("player joins");
        Self {
            game,
            bundle: Some(bundle),
            sequence: 0,
            trace: 0,
        }
    }

    fn snapshot(&self) -> ArpgSnapshot {
        self.game.snapshot().expect("snapshot")
    }

    fn tick(&mut self) -> ArpgSnapshot {
        self.game.advance_tick().expect("tick");
        let snapshot = self.snapshot();
        self.trace = fnv(&format!("{:016x}{}", self.trace, projection(&snapshot)));
        snapshot
    }

    fn command(&mut self, command: ArpgCommand) {
        self.sequence += 1;
        self.game
            .apply_command(PlayerCommand::new(1, self.sequence, command).expect("command"))
            .expect("command accepted");
    }

    fn restore(&self, save: ArpgSaveState) -> ArpgGame {
        // Through the serialised form, exactly as a stored save comes back.
        let json = serde_json::to_string(&save).expect("save serialises");
        let save: ArpgSaveState = serde_json::from_str(&json).expect("save deserialises");
        match &self.bundle {
            None => ArpgGame::from_save_state(save),
            Some(bundle) => ArpgGame::from_save_state_with_content(save, bundle.clone()),
        }
        .expect("arranged save restores")
    }

    fn edit(&mut self, edit: impl FnOnce(&mut ArpgSaveState)) {
        let mut save = self.game.save_state().expect("save");
        edit(&mut save);
        self.game = self.restore(save);
    }

    fn room(&self, room_id: u32) -> RoomSnapshot {
        self.snapshot()
            .rooms
            .into_iter()
            .find(|room| room.id == room_id)
            .expect("room exists")
    }

    fn state(&self, room_id: u32) -> RoomEncounterState {
        self.room(room_id).encounter_state
    }

    fn living(&self, room_id: u32) -> usize {
        self.snapshot()
            .monsters
            .iter()
            .filter(|monster| monster.room_id == room_id && monster.alive)
            .count()
    }

    fn doors_of_room_locked(&self, room_id: u32) -> Vec<bool> {
        self.snapshot()
            .doors
            .iter()
            .filter(|door| door.room_a == room_id || door.room_b == room_id)
            .map(|door| door.locked)
            .collect()
    }

    /// Places player 1 at `position`, idle, at full health.
    fn place(&mut self, position: [i32; 2], facing: [i8; 2]) {
        let max_health = self.snapshot().players[0].max_health;
        self.edit(|save| {
            let player = save.players.iter_mut().find(|p| p.id == 1).expect("saved");
            player.position = [position[0] as f32, player.position[1], position[1] as f32];
            player.velocity = [0.0; 3];
            player.movement = [0, 0];
            player.facing = facing;
            player.aim = None;
            player.locked_monster_id = None;
            player.draw_ticks = None;
            player.health = max_health;
            player.action = None;
        });
    }

    /// Defeats `monster_id` with real light swings: each attempt arranges player 1 beside
    /// it with a swing about to land, the target at one health point and every other living
    /// monster back at full health, so exactly this monster dies.
    fn defeat(&mut self, monster_id: u32) {
        for _ in 0..40 {
            let snapshot = self.snapshot();
            let monster = snapshot
                .monsters
                .iter()
                .find(|m| m.id == monster_id)
                .expect("monster exists")
                .clone();
            if !monster.alive {
                return;
            }
            let room = self.room(monster.room_id);
            let [x, _, z] = monster.position;
            let (dx, dz) = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .into_iter()
                .find(|&(dx, dz)| {
                    let (px, pz) = (x + dx * SIDE_DISTANCE, z + dz * SIDE_DISTANCE);
                    inside(&room, px, pz)
                        && snapshot.monsters.iter().all(|other| {
                            other.id == monster_id
                                || !other.alive
                                || (other.position[0] - px)
                                    .abs()
                                    .max((other.position[2] - pz).abs())
                                    > 2 * BODY_HALF + 10
                        })
                })
                .expect("room beside the monster");
            let max_health = snapshot.players[0].max_health;
            let maxima = snapshot
                .monsters
                .iter()
                .map(|m| (m.id, m.max_health))
                .collect::<Vec<_>>();
            self.edit(|save| {
                for saved in &mut save.monsters {
                    if saved.health == 0 {
                        continue;
                    }
                    saved.health = if saved.id == monster_id {
                        1
                    } else {
                        maxima.iter().find(|(id, _)| *id == saved.id).unwrap().1
                    };
                }
                let player = save.players.iter_mut().find(|p| p.id == 1).expect("saved");
                let facing = [-dx as i8, -dz as i8];
                player.position = [
                    (x + dx * SIDE_DISTANCE) as f32,
                    player.position[1],
                    (z + dz * SIDE_DISTANCE) as f32,
                ];
                player.velocity = [0.0; 3];
                player.movement = [0, 0];
                player.facing = facing;
                player.aim = None;
                player.locked_monster_id = None;
                player.draw_ticks = None;
                player.health = max_health;
                player.action = Some(PlayerActionSnapshot {
                    kind: ActionKind::PrimaryAttack,
                    phase: ActionPhase::Windup,
                    ticks_remaining: 1,
                    facing,
                    connected: false,
                    buffered: None,
                    charge: 0,
                    aim: None,
                });
            });
            for _ in 0..15 {
                self.tick();
            }
        }
        panic!("monster {monster_id} is defeated by light swings");
    }

    /// Stands at the room's chest and presses Interact twice; returns every interaction.
    fn open_chest(&mut self, room_id: u32) -> Vec<InteractionResult> {
        let chest = self
            .snapshot()
            .chests
            .into_iter()
            .find(|chest| chest.room_id == room_id)
            .expect("combat room has a chest");
        let [cx, _, cz] = chest.position;
        self.edit(|save| {
            let player = save.players.iter_mut().find(|p| p.id == 1).expect("saved");
            player.position = [cx as f32, player.position[1], (cz - 100) as f32];
            player.velocity = [0.0; 3];
            player.movement = [0, 0];
            player.facing = [0, 1];
            player.aim = None;
            player.locked_monster_id = None;
            player.action = None;
        });
        let mut results = Vec::new();
        for _ in 0..2 {
            self.command(ArpgCommand::Interact);
            for _ in 0..8 {
                let snapshot = self.tick();
                results.extend(snapshot.interaction_events.iter().map(|event| event.result));
            }
        }
        results
    }
}

fn combat_rooms(snapshot: &ArpgSnapshot) -> Vec<&RoomSnapshot> {
    snapshot
        .rooms
        .iter()
        .filter(|room| room.kind == RoomKind::Combat)
        .collect()
}

/// No interaction opened the chest, and at least one was refused because it is locked.
fn refused_as_locked(results: &[InteractionResult]) -> bool {
    opened_gold(results).is_empty()
        && results.contains(&InteractionResult::Refused {
            reason: InteractionRefusal::ChestLocked,
        })
}

fn opened_gold(results: &[InteractionResult]) -> Vec<u32> {
    results
        .iter()
        .filter_map(|result| match result {
            InteractionResult::Opened { gold, .. } => Some(*gold),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------
// The built-in bundle expresses the current behaviour through definitions
// ---------------------------------------------------------------------------------------

#[test]
fn base_bundle_assigns_each_combat_room_a_single_member_encounter_with_a_chest_reward() {
    // Format 9 added jump tuning (#59) after the encounter definitions of format 8.
    assert_eq!(CONTENT_FORMAT_VERSION, 14, "the bundle's shape changed");
    assert_eq!(base_bundle().format_version, CONTENT_FORMAT_VERSION);
    base_bundle()
        .check()
        .expect("built-in bundle passes the public validation");
    assert!((1..=MAX_WORKBENCH_MONSTERS).contains(&MAX_ENCOUNTER_SPAWNS));

    let value = base_json();
    assert!(
        value.get("roomMonsters").is_none(),
        "replaced by encounters"
    );
    assert!(
        value["loot"].get("chestGold").is_none(),
        "moved to a reward definition"
    );
    let room_encounters = value["roomEncounters"]
        .as_array()
        .expect("roomEncounters is authored");
    // The previous `roomMonsters`, in the same authored order.
    let previous = [
        "monster.brute",
        "monster.archer",
        "monster.skirmisher",
        "monster.bruiser",
        "monster.defender",
    ];
    assert_eq!(room_encounters.len(), previous.len());
    for (entry, monster) in room_encounters.iter().zip(previous) {
        let encounter = find(&value["encounters"], entry.as_str().expect("id"));
        assert_eq!(
            encounter["spawnGroup"],
            json!([{ "monster": monster, "count": 1 }]),
            "{entry}"
        );
        assert_eq!(encounter["activation"], "playerEntersRoom", "{entry}");
        assert_eq!(encounter["completion"], "spawnGroupDefeated", "{entry}");
        let reward = find(&value["rewards"], encounter["reward"].as_str().expect("id"));
        assert_eq!(reward["kind"], "chest", "{entry}");
        assert_eq!(reward["gold"], 25, "{entry}: the previous loot.chestGold");
    }
    assert_eq!(value["loot"]["monsterGold"], 10);
}

/// Spawn tables (id, definition, room, position) and a scripted encounter trace recorded
/// with the pre-definition runtime (`roomMonsters`, `loot.chestGold`): the built-in bundle
/// must reproduce both exactly. Update only for a deliberate gameplay change.
const GOLDEN: [(u32, u64, &str); 6] = [
    (
        0,
        0xbaa0_bb59_7dfb_c610,
        "1:monster.brute:2:[-48, 50, -652];2:monster.archer:3:[1676, 50, -369];3:monster.skirmisher:4:[-2272, 50, 543];4:monster.bruiser:5:[345, 50, 737];5:monster.defender:6:[1579, 50, 1191]",
    ),
    (
        1,
        0x1e24_92ad_dd99_b566,
        "1:monster.brute:2:[472, 50, -1215];2:monster.archer:3:[2622, 50, -1010];3:monster.skirmisher:4:[-2023, 50, 363];4:monster.bruiser:5:[-113, 50, 1159];5:monster.defender:6:[1527, 50, 1060]",
    ),
    (
        7,
        0x4baa_befa_5a68_aec3,
        "1:monster.brute:2:[-504, 50, -1433];2:monster.archer:3:[2022, 50, -955];3:monster.skirmisher:4:[-1629, 50, 1514];4:monster.bruiser:5:[-903, 50, 1548];5:monster.defender:6:[2577, 50, 293]",
    ),
    (
        42,
        0x7af4_8e1d_2fb3_6ff1,
        "1:monster.brute:2:[-703, 50, -667];2:monster.archer:3:[1882, 50, -1287];3:monster.skirmisher:4:[-2140, 50, 334];4:monster.bruiser:5:[-439, 50, 361];5:monster.defender:6:[1765, 50, 1265]",
    ),
    (
        65_535,
        0x7450_54a8_d8ba_60d3,
        "1:monster.brute:2:[-168, 50, -210];2:monster.archer:3:[2474, 50, -528];3:monster.skirmisher:4:[-1943, 50, 1092];4:monster.bruiser:5:[-650, 50, 1267];5:monster.defender:6:[1771, 50, 1403]",
    ),
    (
        u32::MAX,
        0x1b28_2ecd_6955_63dd,
        "1:monster.brute:2:[-552, 50, -1484];2:monster.archer:3:[2136, 50, -1457];3:monster.skirmisher:4:[-2583, 50, 594];4:monster.bruiser:5:[-899, 50, 1292];5:monster.defender:6:[2260, 50, 1088]",
    ),
];

#[test]
fn base_gameplay_reproduces_the_pre_definition_spawns_and_encounter_trace() {
    for (seed, trace, table) in GOLDEN {
        let mut session = Session::base(seed);
        assert_eq!(
            spawn_table(&session.snapshot()),
            table,
            "seed {seed} spawns"
        );
        let room = session.snapshot().monsters[0].room_id;
        session.defeat(1);
        let opened = opened_gold(&session.open_chest(room));
        assert_eq!(opened, [25], "seed {seed}");
        assert_eq!(session.snapshot().players[0].gold, 25, "seed {seed}");
        assert_eq!(
            session.trace, trace,
            "seed {seed}: scripted encounter trace"
        );
    }
}

#[test]
fn base_rooms_publish_their_encounter_and_keep_the_dormant_active_cleared_lifecycle() {
    let room_encounters = base_bundle().room_encounters.clone();
    for seed in [0, 7, 42] {
        let mut session = Session::base(seed);
        session.tick();
        let snapshot = session.snapshot();
        let start = snapshot
            .rooms
            .iter()
            .find(|room| room.kind == RoomKind::Start)
            .expect("start room");
        assert_eq!(
            start.encounter, None,
            "seed {seed}: the start room has none"
        );
        assert_eq!(start.encounter_state, RoomEncounterState::Cleared);
        assert!(
            snapshot
                .chests
                .iter()
                .all(|chest| chest.room_id != start.id)
        );
        for (index, room) in combat_rooms(&snapshot).into_iter().enumerate() {
            assert_eq!(
                room.encounter.as_deref(),
                Some(room_encounters[index % room_encounters.len()].as_str()),
                "seed {seed}: combat room {} publishes its encounter",
                room.id
            );
            assert_eq!(
                room.encounter_state,
                RoomEncounterState::Dormant,
                "seed {seed}: no player entered room {}",
                room.id
            );
            assert_eq!(
                snapshot
                    .chests
                    .iter()
                    .filter(|chest| chest.room_id == room.id)
                    .count(),
                1,
                "seed {seed}: one reward chest per combat room"
            );
        }

        // Activation: a player enters. The reward stays locked while the group lives.
        let room_id = snapshot.monsters[0].room_id;
        let results = session.open_chest(room_id);
        assert_eq!(
            session.state(room_id),
            RoomEncounterState::Active,
            "seed {seed}"
        );
        assert!(session.doors_of_room_locked(room_id).iter().all(|&l| l));
        assert!(refused_as_locked(&results), "seed {seed}: {results:?}");

        // Completion: the group is defeated; the reward opens once.
        session.defeat(1);
        assert_eq!(
            session.state(room_id),
            RoomEncounterState::Cleared,
            "seed {seed}"
        );
        assert!(session.doors_of_room_locked(room_id).iter().all(|&l| !l));
        let saved = session.game.save_state().expect("save");
        assert_eq!(
            opened_gold(&session.open_chest(room_id)),
            [25],
            "seed {seed}"
        );

        // Save/restore keeps the cleared state and the one-time reward.
        let reopened = session.restore(session.game.save_state().expect("save"));
        session.game = reopened;
        assert_eq!(session.state(room_id), RoomEncounterState::Cleared);
        assert!(opened_gold(&session.open_chest(room_id)).is_empty());
        // A save from before the chest opened still has it to open, exactly once.
        session.game = session.restore(saved);
        assert_eq!(
            opened_gold(&session.open_chest(room_id)),
            [25],
            "seed {seed}"
        );
    }
}

#[test]
fn a_base_save_mid_encounter_continues_like_the_uninterrupted_game() {
    let mut session = Session::base(42);
    let room_id = session.snapshot().monsters[0].room_id;
    // A spot in the room well away from its monster.
    let monster = session.snapshot().monsters[0].position;
    let spot = {
        let room = session.room(room_id);
        let center = [(room.min_x + room.max_x) / 2, (room.min_z + room.max_z) / 2];
        [-400, 0, 400]
            .into_iter()
            .map(|dx| [center[0] + dx, center[1]])
            .max_by_key(|spot| (spot[0] - monster[0]).abs() + (spot[1] - monster[2]).abs())
            .expect("candidate spots")
    };
    session.place(spot, [1, 0]);
    for _ in 0..5 {
        session.tick();
    }
    assert_eq!(session.state(room_id), RoomEncounterState::Active);

    let mut restored = Session {
        game: session.restore(session.game.save_state().expect("save")),
        bundle: None,
        sequence: session.sequence,
        trace: session.trace,
    };
    assert_eq!(restored.state(room_id), RoomEncounterState::Active);
    for _ in 0..60 {
        assert_eq!(projection(&session.tick()), projection(&restored.tick()));
    }
}

// ---------------------------------------------------------------------------------------
// Validation fails closed
// ---------------------------------------------------------------------------------------

#[test]
fn the_fixture_bundle_is_valid_and_its_source_order_is_canonical() {
    let bundle = fixture();
    bundle.check().expect("the fixture is a valid bundle");
    assert_ne!(bundle.revision(), content_revision());

    let mut shuffled = fixture_json();
    for key in ["encounters", "rewards"] {
        shuffled[key].as_array_mut().unwrap().reverse();
    }
    let reparsed = parse(&shuffled);
    assert_eq!(reparsed, bundle, "encounters and rewards are ordered by id");
    assert_eq!(reparsed.revision(), bundle.revision());

    // Room assignment and spawn-group order are authored: reordering is another bundle.
    let mut reassigned = fixture_json();
    reassigned["roomEncounters"]
        .as_array_mut()
        .unwrap()
        .reverse();
    assert_ne!(parse(&reassigned).revision(), bundle.revision());
    let mut regrouped = fixture_json();
    regrouped["encounters"][0]["spawnGroup"]
        .as_array_mut()
        .unwrap()
        .reverse();
    let regrouped = parse(&regrouped);
    regrouped.check().expect("still valid");
    assert_ne!(regrouped.revision(), bundle.revision());
}

#[test]
fn encounters_with_unknown_references_are_rejected() {
    let errors = violations(|value| {
        value["encounters"][0]["spawnGroup"][0]["monster"] = json!("monster.missing");
        value["encounters"][1]["reward"] = json!("reward.missing");
        value["roomEncounters"][1] = json!("encounter.missing");
    });
    assert!(
        reports(&errors, "encounters[", PACK, "unknown monster"),
        "{errors:?}"
    );
    assert!(
        reports(&errors, "encounters[", LONE, "unknown reward"),
        "{errors:?}"
    );
    assert!(
        reports(
            &errors,
            "roomEncounters[1]",
            "encounter.missing",
            "unknown encounter"
        ),
        "{errors:?}"
    );
    // Every violation is reported, and the now unused definitions are named too.
    assert!(
        reports(&errors, "encounters[", LONE, "not referenced"),
        "{errors:?}"
    );
    assert!(
        reports(&errors, "rewards[", PURSE, "not referenced"),
        "{errors:?}"
    );
}

#[test]
fn spawn_groups_must_be_non_empty_positive_unique_and_bounded() {
    let empty = violations(|value| value["encounters"][0]["spawnGroup"] = json!([]));
    assert!(reports(&empty, "encounters[", PACK, "empty"), "{empty:?}");

    let zero = violations(|value| value["encounters"][0]["spawnGroup"][0]["count"] = json!(0));
    assert!(
        reports(&zero, "encounters[", PACK, "count must be positive"),
        "{zero:?}"
    );

    let twice = violations(|value| {
        value["encounters"][0]["spawnGroup"] = json!([
            { "monster": "monster.skirmisher", "count": 1 },
            { "monster": "monster.skirmisher", "count": 1 }
        ]);
    });
    assert!(
        reports(&twice, "encounters[", PACK, "more than once"),
        "{twice:?}"
    );

    // The bound counts every member over all entries.
    let over = MAX_ENCOUNTER_SPAWNS + 1;
    let too_many = violations(|value| {
        value["encounters"][0]["spawnGroup"] = json!([
            { "monster": "monster.skirmisher", "count": over - 1 },
            { "monster": "monster.archer", "count": 1 }
        ]);
    });
    assert!(
        reports(
            &too_many,
            "encounters[",
            PACK,
            &format!("at most {MAX_ENCOUNTER_SPAWNS}")
        ),
        "{too_many:?}"
    );

    let mut at_bound = fixture_json();
    at_bound["encounters"][0]["spawnGroup"] = json!([
        { "monster": "monster.skirmisher", "count": MAX_ENCOUNTER_SPAWNS - 1 },
        { "monster": "monster.archer", "count": 1 }
    ]);
    parse(&at_bound)
        .check()
        .expect("a group of exactly MAX_ENCOUNTER_SPAWNS is allowed");
}

#[test]
fn encounter_and_reward_ids_are_unique_bounded_and_referenced() {
    let duplicate = violations(|value| {
        let pack = value["encounters"][0].clone();
        value["encounters"].as_array_mut().unwrap().push(pack);
        let hoard = value["rewards"][0].clone();
        value["rewards"].as_array_mut().unwrap().push(hoard);
    });
    assert!(
        reports(&duplicate, "encounters[", PACK, "duplicate encounter id"),
        "{duplicate:?}"
    );
    assert!(
        reports(&duplicate, "rewards[", HOARD, "duplicate reward id"),
        "{duplicate:?}"
    );

    let unreferenced = violations(|value| {
        value["encounters"].as_array_mut().unwrap().push(encounter(
            "encounter.spare",
            &[("monster.brute", 1)],
            HOARD,
        ));
        value["rewards"]
            .as_array_mut()
            .unwrap()
            .push(reward("reward.spare", 1));
    });
    assert!(
        reports(
            &unreferenced,
            "encounters[",
            "encounter.spare",
            "not referenced"
        ),
        "{unreferenced:?}"
    );
    assert!(
        reports(&unreferenced, "rewards[", "reward.spare", "not referenced"),
        "{unreferenced:?}"
    );

    let long = "e".repeat(65);
    let bad_ids = violations(|value| {
        value["encounters"][1]["id"] = json!(long);
        value["roomEncounters"][1] = json!(long);
    });
    assert!(
        bad_ids
            .iter()
            .any(|error| error.starts_with("encounters") && error.contains("1..=64 ASCII")),
        "{bad_ids:?}"
    );

    let poor = violations(|value| value["rewards"][1]["gold"] = json!(0));
    assert!(
        reports(&poor, "rewards[", PURSE, "gold must be positive"),
        "{poor:?}"
    );

    let none = violations(|value| value["roomEncounters"] = json!([]));
    assert!(
        none.iter()
            .any(|error| error.starts_with("roomEncounters") && error.contains("at least one")),
        "{none:?}"
    );
}

#[test]
fn unsupported_vocabulary_and_the_previous_fields_fail_to_parse() {
    let unparsable = |mutate: &dyn Fn(&mut Value)| {
        let mut value = fixture_json();
        mutate(&mut value);
        ContentBundle::from_json(&value.to_string()).is_err()
    };
    assert!(unparsable(
        &|v| v["encounters"][0]["activation"] = json!("timer")
    ));
    assert!(unparsable(
        &|v| v["encounters"][0]["completion"] = json!("survive")
    ));
    assert!(unparsable(&|v| v["rewards"][0]["kind"] = json!("portal")));
    assert!(unparsable(&|v| v["encounters"][0]["script"] = json!("x")));
    assert!(unparsable(
        &|v| v["encounters"][0]["spawnGroup"][0]["count"] = json!(-1)
    ));
    assert!(unparsable(&|v| {
        v.as_object_mut().unwrap().remove("roomEncounters");
    }));
    assert!(unparsable(&|v| v["roomMonsters"] = json!(["monster.brute"])));
    assert!(unparsable(&|v| v["loot"]["chestGold"] = json!(25)));

    // A format-7 bundle reports its version rather than the first new field.
    let mut previous = base_json();
    previous["formatVersion"] = json!(7);
    let errors = ContentBundle::from_json(&previous.to_string()).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].path, "formatVersion");
}

#[test]
fn an_invalid_bundle_never_starts_or_restores_an_authority() {
    let mut value = fixture_json();
    value["encounters"][0]["spawnGroup"][0]["monster"] = json!("monster.missing");
    let invalid = parse(&value);
    let Err(error) = ArpgGame::new_with_content(invalid.clone(), 7) else {
        panic!("an invalid bundle is refused");
    };
    assert!(error.message().contains("unknown monster"), "{error}");

    let save = Session::with_content(fixture(), 7)
        .game
        .save_state()
        .expect("save");
    assert!(ArpgGame::from_save_state_with_content(save, invalid).is_err());
}

// ---------------------------------------------------------------------------------------
// A second, independently authored encounter set changes spawns and reward by data only
// ---------------------------------------------------------------------------------------

#[test]
fn fixture_encounters_spawn_their_groups_in_authored_order() {
    let bundle = fixture();
    for seed in [0, 7, 42] {
        let session = Session::with_content(bundle.clone(), seed);
        let snapshot = session.snapshot();
        assert_eq!(snapshot.content_revision, bundle.revision());

        let mut expected_id = 1;
        for (room, (encounter, members, _)) in
            combat_rooms(&snapshot).into_iter().zip(fixture_groups())
        {
            assert_eq!(room.encounter.as_deref(), Some(encounter), "seed {seed}");
            assert_eq!(room.encounter_state, RoomEncounterState::Dormant);
            let spawned = snapshot
                .monsters
                .iter()
                .filter(|monster| monster.room_id == room.id)
                .map(|monster| (monster.id, monster.definition.as_str()))
                .collect::<Vec<_>>();
            let expected = members
                .iter()
                .map(|&definition| {
                    expected_id += 1;
                    (expected_id - 1, definition)
                })
                .collect::<Vec<_>>();
            assert_eq!(spawned, expected, "seed {seed}: room {}", room.id);
        }
        assert_eq!(snapshot.monsters.len(), 11, "3 + 1 + 3 + 1 + 3 members");
    }
}

#[test]
fn a_fixture_group_activates_on_entry_and_clears_only_when_every_member_is_defeated() {
    let mut session = Session::with_content(fixture(), 7);
    let snapshot = session.snapshot();
    let pack_room = combat_rooms(&snapshot)[0].id;
    let members = snapshot
        .monsters
        .iter()
        .filter(|monster| monster.room_id == pack_room)
        .map(|monster| monster.id)
        .collect::<Vec<_>>();
    assert_eq!(members.len(), 3);

    let locked = session.open_chest(pack_room);
    assert_eq!(session.state(pack_room), RoomEncounterState::Active);
    assert!(refused_as_locked(&locked), "{locked:?}");

    for (defeated, &member) in members.iter().enumerate() {
        session.defeat(member);
        let remaining = members.len() - defeated - 1;
        assert_eq!(session.living(pack_room), remaining);
        if remaining > 0 {
            assert_eq!(
                session.state(pack_room),
                RoomEncounterState::Active,
                "{remaining} members still live"
            );
            assert!(session.doors_of_room_locked(pack_room).iter().all(|&l| l));
            assert!(
                !session
                    .snapshot()
                    .chests
                    .iter()
                    .any(|chest| chest.room_id == pack_room && chest.available),
                "the reward stays locked during a partial clear"
            );
        }
    }
    assert_eq!(session.state(pack_room), RoomEncounterState::Cleared);
    assert!(session.doors_of_room_locked(pack_room).iter().all(|&l| !l));
    let gold = session.snapshot().players[0].gold;
    assert_eq!(opened_gold(&session.open_chest(pack_room)), [HOARD_GOLD]);
    assert_eq!(session.snapshot().players[0].gold, gold + HOARD_GOLD);
    assert!(
        opened_gold(&session.open_chest(pack_room)).is_empty(),
        "opens once"
    );

    // The neighbouring encounter pays its own reward.
    let lone_room = combat_rooms(&session.snapshot())[1].id;
    let lone = session
        .snapshot()
        .monsters
        .iter()
        .find(|monster| monster.room_id == lone_room)
        .expect("lone bruiser")
        .id;
    session.defeat(lone);
    assert_eq!(session.state(lone_room), RoomEncounterState::Cleared);
    assert_eq!(opened_gold(&session.open_chest(lone_room)), [PURSE_GOLD]);
}

#[test]
fn a_partially_cleared_fixture_encounter_survives_save_and_continues_identically() {
    let mut session = Session::with_content(fixture(), 42);
    let snapshot = session.snapshot();
    let pack_room = combat_rooms(&snapshot)[0].id;
    let first = snapshot
        .monsters
        .iter()
        .find(|monster| monster.room_id == pack_room)
        .expect("pack member")
        .id;
    session.defeat(first);
    assert_eq!(session.living(pack_room), 2);
    assert_eq!(session.state(pack_room), RoomEncounterState::Active);

    let save = session.game.save_state().expect("partial encounter saves");
    assert_eq!(save.content_revision, fixture().revision());
    assert!(
        ArpgGame::from_save_state(save.clone()).is_err(),
        "the built-in authority refuses a save from another bundle"
    );
    let base_save = Session::base(42).game.save_state().expect("save");
    assert!(
        ArpgGame::from_save_state_with_content(base_save, fixture()).is_err(),
        "the fixture authority refuses a built-in save"
    );

    let mut restored = Session {
        game: session.restore(save),
        bundle: Some(fixture()),
        sequence: session.sequence,
        trace: session.trace,
    };
    assert_eq!(restored.state(pack_room), RoomEncounterState::Active);
    assert_eq!(restored.living(pack_room), 2);
    assert_eq!(
        restored.room(pack_room).encounter.as_deref(),
        Some(PACK),
        "the encounter assignment follows from seed and bundle"
    );
    for _ in 0..60 {
        assert_eq!(projection(&session.tick()), projection(&restored.tick()));
    }
}

// ---------------------------------------------------------------------------------------
// Bounded spawning stays valid and deterministic across seeds
// ---------------------------------------------------------------------------------------

#[test]
fn full_spawn_groups_spawn_inside_their_room_without_overlap_across_seeds() {
    let mut value = fixture_json();
    value["encounters"][0]["spawnGroup"] = json!([
        { "monster": "monster.skirmisher", "count": MAX_ENCOUNTER_SPAWNS - 1 },
        { "monster": "monster.archer", "count": 1 }
    ]);
    let bundle = parse(&value);
    for seed in 0..128 {
        let snapshot = Session::with_content(bundle.clone(), seed).snapshot();
        for room in combat_rooms(&snapshot) {
            let members = snapshot
                .monsters
                .iter()
                .filter(|monster| monster.room_id == room.id)
                .collect::<Vec<_>>();
            let expected = if room.encounter.as_deref() == Some(PACK) {
                MAX_ENCOUNTER_SPAWNS
            } else {
                1
            };
            assert_eq!(members.len(), expected, "seed {seed}: room {}", room.id);
            for (index, monster) in members.iter().enumerate() {
                let [x, _, z] = monster.position;
                assert!(
                    x - BODY_HALF >= room.min_x
                        && x + BODY_HALF <= room.max_x
                        && z - BODY_HALF >= room.min_z
                        && z + BODY_HALF <= room.max_z,
                    "seed {seed}: monster {} outside room {}",
                    monster.id,
                    room.id
                );
                assert!(
                    snapshot.static_colliders.iter().all(|collider| {
                        (x - collider.position[0]).abs() >= collider.half_extents[0] + BODY_HALF
                            || (z - collider.position[2]).abs()
                                >= collider.half_extents[2] + BODY_HALF
                    }),
                    "seed {seed}: monster {} overlaps a fixed collider",
                    monster.id
                );
                for other in &members[index + 1..] {
                    let separation = (x - other.position[0])
                        .abs()
                        .max((z - other.position[2]).abs());
                    assert!(
                        separation >= 2 * BODY_HALF,
                        "seed {seed}: monsters {} and {} overlap",
                        monster.id,
                        other.id
                    );
                }
            }
        }
    }
}

#[test]
fn fixture_runs_replay_identically_per_seed_and_vary_between_seeds() {
    let bundle = fixture();
    let mut tables = std::collections::BTreeSet::new();
    for seed in [0, 1, 2, 7, 42, 255, 65_535, u32::MAX] {
        let run = |bundle: &ContentBundle| {
            let mut session = Session::with_content(bundle.clone(), seed);
            let table = spawn_table(&session.snapshot());
            let room = session.snapshot().monsters[0].room_id;
            for member in session
                .snapshot()
                .monsters
                .iter()
                .filter(|monster| monster.room_id == room)
                .map(|monster| monster.id)
                .collect::<Vec<_>>()
            {
                session.defeat(member);
            }
            let results = session.open_chest(room);
            (table, session.trace, session.snapshot(), results)
        };
        let first = run(&bundle);
        let replay = run(&bundle);
        assert_eq!(first, replay, "seed {seed} must replay identically");
        let (table, _, snapshot, results) = first;
        // A defeated member's gold drop can lie nearer the standing spot than the chest,
        // so Interact may pick it up first; the chest itself pays the hoard exactly once.
        assert_eq!(
            opened_gold(&results),
            [HOARD_GOLD],
            "seed {seed}: {results:?}"
        );
        let credited = results
            .iter()
            .map(|result| match result {
                InteractionResult::PickedUp { gold, .. }
                | InteractionResult::Opened { gold, .. } => *gold,
                InteractionResult::Refused { .. } => 0,
            })
            .sum::<u32>();
        assert_eq!(
            snapshot.players[0].gold, credited,
            "seed {seed}: gold comes only from the resolved interactions"
        );
        tables.insert(table);
    }
    assert!(tables.len() > 1, "spawn positions follow the seed");
}
