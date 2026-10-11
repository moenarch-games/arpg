//! Acceptance for enemy shield users (#92, "enemy shield users" slice).
//!
//! Contract: a shielded defender (`monster.defender`, the roadmap's "shielded defender"
//! role) raises a guard. Its guard follows the same rules as the player's shield guard:
//! it rises over the shared `guard.raiseTicks` before it protects, a raised guard protects
//! only the front sector against blockable strikes, each block spends the strike's
//! authored guard cost, a strike whose cost reaches the remaining guard breaks it, and a
//! broken guard stays down for the shared `guard.breakTicks`. Its AI decides when to guard
//! deterministically: it closes on its target under a raised guard and lowers the guard
//! only to commit to its own attack. The monster snapshot publishes the guard so
//! presentation can show it.
//!
//! The defender faces its engaged target, so "front" is the side of its target.
//!
//! Tests use only the public runtime: generated dungeons, commands, ticks, snapshots and
//! (validated) save states, which place players and the defender exactly.

use arpg_core::{
    ActionKind, ActionPhase, ArpgCommand, ArpgGame, ArpgSaveState, ArpgSnapshot, AuthoritativeGame,
    GuardPhase, GuardStance, MonsterSnapshot, PlayerActionSnapshot, PlayerCommand, PlayerId,
    RoomSnapshot, StrikeResult, StrikeSource, StrikeTarget, base_bundle,
};

const DEFENDER: &str = "monster.defender";
/// Seeds searched for a generated dungeon that holds a defender with room to arrange.
const SEED_SEARCH: u32 = 512;
/// Player distance while the defender closes on it: outside any melee reach.
const FAR: i32 = 360;
/// Clearance kept between arranged bodies and the room's walls.
const WALL_MARGIN: i32 = 80;

/// Content values the contract is expressed in, read from the base bundle.
struct Values {
    light_reach: i32,
    light_guard_cost: u16,
    raise_ticks: u8,
    break_ticks: u8,
    defender_speed: i32,
}

fn values() -> Values {
    let bundle = base_bundle();
    let light = bundle
        .strikes
        .iter()
        .find(|strike| strike.id == "sword.lightSwing")
        .expect("the light swing strike is authored");
    let defender = bundle
        .monsters
        .iter()
        .find(|monster| monster.id == DEFENDER)
        .unwrap_or_else(|| panic!("base content must define the shielded defender {DEFENDER}"));
    assert!(
        i64::from(defender.aggro_range) > i64::from(FAR),
        "the arranged target must be inside the defender's aggro range"
    );
    Values {
        light_reach: i32::try_from(light.reach).expect("reach fits i32"),
        light_guard_cost: light.guard_cost,
        raise_ticks: bundle.guard.raise_ticks,
        break_ticks: bundle.guard.break_ticks,
        defender_speed: i32::from(defender.pursuit_speed),
    }
}

/// Distance at which a strike resolving next tick reaches the defender whether it steps
/// toward or away from the striker during that tick.
fn near(values: &Values) -> i32 {
    values.light_reach - values.defender_speed - 2
}

fn inside(room: &RoomSnapshot, x: i32, z: i32) -> bool {
    x >= room.min_x + WALL_MARGIN
        && x <= room.max_x - WALL_MARGIN
        && z >= room.min_z + WALL_MARGIN
        && z <= room.max_z - WALL_MARGIN
}

fn defender_of(snapshot: &ArpgSnapshot) -> Option<&MonsterSnapshot> {
    snapshot
        .monsters
        .iter()
        .find(|monster| monster.definition == DEFENDER)
}

fn raising(ticks_remaining: u8) -> Option<GuardStance> {
    Some(GuardStance {
        phase: GuardPhase::Raising,
        ticks_remaining,
    })
}

const RAISED: Option<GuardStance> = Some(GuardStance {
    phase: GuardPhase::Raised,
    ticks_remaining: 0,
});

/// A light swing one tick before its active phase: it resolves during the next tick.
fn swing_now(facing: [i8; 2]) -> PlayerActionSnapshot {
    PlayerActionSnapshot {
        kind: ActionKind::PrimaryAttack,
        phase: ActionPhase::Windup,
        ticks_remaining: 1,
        facing,
        connected: false,
        buffered: None,
        charge: 0,
        aim: None,
    }
}

fn sign(value: i32) -> i8 {
    if value < 0 { -1 } else { 1 }
}

/// A generated dungeon with player 1 standing `FAR` from a defender in its room, on the
/// side (`side`, along x) with room for every arrangement the tests use.
struct Arena {
    game: ArpgGame,
    seed: u32,
    defender_id: u32,
    /// Where the defender stood when generated; arrangements put it back here.
    post: [i32; 3],
    side: i32,
    sequence: u32,
}

impl Arena {
    fn new(players: &[PlayerId]) -> Self {
        let values = values();
        let mut found_defender = false;
        for seed in 0..SEED_SEARCH {
            let mut game = ArpgGame::new_with_seed(seed).expect("seeded dungeon constructs");
            for &player_id in players {
                game.add_player(player_id).expect("player joins");
            }
            let snapshot = game.snapshot().expect("snapshot");
            let Some(defender) = defender_of(&snapshot) else {
                continue;
            };
            found_defender = true;
            let room = snapshot
                .rooms
                .iter()
                .find(|room| room.id == defender.room_id)
                .expect("defender room exists");
            let [x, _, z] = defender.position;
            let reach = values.light_reach;
            let Some(side) = [-1, 1].into_iter().find(|&side| {
                inside(room, x + side * FAR, z)
                    && inside(room, x - side * reach, z)
                    && inside(room, x, z + reach)
            }) else {
                continue;
            };
            let mut arena = Self {
                game,
                seed,
                defender_id: defender.id,
                post: defender.position,
                side,
                sequence: 0,
            };
            arena.arrange(FAR, None);
            assert_eq!(
                arena.defender().guard,
                None,
                "a dormant defender is unguarded"
            );
            return arena;
        }
        panic!(
            "{}",
            if found_defender {
                "no generated dungeon leaves room to arrange around a defender"
            } else {
                "base content must place monster.defender in generated dungeons (roomEncounters)"
            }
        );
    }

    fn snapshot(&self) -> ArpgSnapshot {
        self.game.snapshot().expect("snapshot")
    }

    fn defender(&self) -> MonsterSnapshot {
        defender_of(&self.snapshot())
            .expect("the defender stays in the dungeon")
            .clone()
    }

    fn distance_to_player(&self, player_id: PlayerId) -> i32 {
        let snapshot = self.snapshot();
        let player = snapshot
            .players
            .iter()
            .find(|player| player.id == player_id)
            .expect("player exists");
        let defender = defender_of(&snapshot).expect("defender exists");
        let dx = f64::from(player.position[0] - defender.position[0]);
        let dz = f64::from(player.position[2] - defender.position[2]);
        dx.hypot(dz).round() as i32
    }

    /// Ticks once and returns the outcomes of strikes from `player_id` on the defender.
    fn tick(&mut self, player_id: PlayerId) -> Vec<StrikeResult> {
        self.game.advance_tick().expect("tick");
        self.game
            .strike_outcomes()
            .iter()
            .filter(|outcome| {
                outcome.strike.source == StrikeSource::Player(player_id)
                    && outcome.target == StrikeTarget::Monster(self.defender_id)
            })
            .map(|outcome| outcome.result)
            .collect()
    }

    fn command(&mut self, player_id: PlayerId, command: ArpgCommand) {
        self.sequence += 1;
        self.game
            .apply_command(PlayerCommand::new(player_id, self.sequence, command).expect("command"))
            .expect("command accepted");
    }

    /// Restores the game from its own save after `edit`: the save is the persistence
    /// boundary, so everything the defender's guard needs must survive it.
    fn edit(&mut self, edit: impl FnOnce(&mut ArpgSaveState)) {
        let mut save = self.game.save_state().expect("save");
        edit(&mut save);
        // Through the serialised form, exactly as a stored save comes back.
        let json = serde_json::to_string(&save).expect("save serialises");
        let save = serde_json::from_str(&json).expect("save deserialises");
        self.game = ArpgGame::from_save_state(save).expect("arranged save restores");
    }

    /// Puts the defender back at its post and player 1 `distance` from it on `side`,
    /// facing it, idle or in `action`.
    fn arrange(&mut self, distance: i32, action: Option<PlayerActionSnapshot>) {
        let offset = [self.side * distance, 0];
        self.arrange_player(1, offset, action);
    }

    /// Puts the defender back at its post and `player_id` at `offset` `[x, z]` from it,
    /// facing the defender, idle or in `action`.
    fn arrange_player(
        &mut self,
        player_id: PlayerId,
        offset: [i32; 2],
        action: Option<PlayerActionSnapshot>,
    ) {
        let (defender_id, post) = (self.defender_id, self.post);
        let facing = if offset[0] != 0 {
            [-sign(offset[0]), 0]
        } else {
            [0, -sign(offset[1])]
        };
        self.edit(|save| {
            let defender = save
                .monsters
                .iter_mut()
                .find(|monster| monster.id == defender_id)
                .expect("defender saved");
            defender.position = post;
            let player = save
                .players
                .iter_mut()
                .find(|player| player.id == player_id)
                .expect("player saved");
            player.position = [post[0] + offset[0], player.position[1], post[2] + offset[1]];
            player.velocity = [0, 0, 0];
            player.movement = [0, 0];
            player.facing = facing;
            player.aim = None;
            player.locked_monster_id = None;
            player.draw_ticks = None;
            player.action = action.map(|action| PlayerActionSnapshot { facing, ..action });
        });
    }

    /// Ticks (player 1 far away) until the defender's guard is raised and it is not
    /// attacking.
    fn until_raised(&mut self) {
        for _ in 0..600 {
            let defender = self.defender();
            if defender.guard == RAISED && defender.action.is_none() {
                return;
            }
            if self.distance_to_player(1) < FAR / 2 {
                self.arrange(FAR, None);
            }
            self.tick(1);
        }
        panic!("an engaged defender closing on its target raises its guard");
    }

    /// Ticks until the defender's guard is rising with at least two raise ticks left.
    fn until_raising(&mut self) {
        for _ in 0..600 {
            if self.defender().guard.is_some_and(|stance| {
                stance.phase == GuardPhase::Raising && stance.ticks_remaining >= 2
            }) {
                return;
            }
            self.tick(1);
        }
        panic!("an engaged defender raises its guard through the raising phase");
    }

    /// Strikes the raised guard from the front until it breaks.
    fn break_guard(&mut self) {
        let values = values();
        for _ in 0..200 {
            self.until_raised();
            self.arrange(near(&values), Some(swing_now([0, 0])));
            if self.tick(1) == [StrikeResult::GuardBroken] {
                return;
            }
            self.arrange(FAR, None);
        }
        panic!("enough frontal strikes break the defender's guard");
    }
}

/// Every snapshot over `ticks` ticks with player 1 standing still where it was arranged.
fn timeline(arena: &mut Arena, ticks: usize) -> Vec<ArpgSnapshot> {
    let mut snapshots = Vec::new();
    for _ in 0..ticks {
        arena.tick(1);
        snapshots.push(arena.snapshot());
    }
    snapshots
}

#[test]
fn defender_closes_under_a_guard_raised_with_the_shared_timing_and_lowers_it_to_attack() {
    let values = values();
    let mut arena = Arena::new(&[1]);
    let start = arena.defender();
    assert!(start.max_guard_points > 0, "a shield user has guard points");
    assert_eq!(
        start.guard_points, start.max_guard_points,
        "it starts fresh"
    );

    // It engages and raises its guard while closing: Raising(n)..Raising(1), Raised.
    let mut stances = Vec::new();
    for _ in 0..usize::from(values.raise_ticks) + 4 {
        arena.tick(1);
        let defender = arena.defender();
        assert_eq!(defender.action, None, "it is still closing, not attacking");
        if defender.guard.is_some() {
            stances.push(defender.guard);
        }
    }
    let mut expected = (1..=values.raise_ticks)
        .rev()
        .map(raising)
        .collect::<Vec<_>>();
    expected.push(RAISED);
    assert_eq!(
        stances[..expected.len().min(stances.len())],
        expected[..],
        "the defender's guard rises over the shared raise ticks"
    );
    assert!(
        stances[expected.len()..]
            .iter()
            .all(|&stance| stance == RAISED),
        "and stays raised while it closes"
    );

    // It holds the guard until it commits to an attack, and the attack lowers it.
    let mut previous = arena.defender();
    let mut attack_started = false;
    for _ in 0..1_200 {
        arena.tick(1);
        let defender = arena.defender();
        if defender.action.is_some() {
            assert_eq!(previous.guard, RAISED, "it closed under a raised guard");
            assert_eq!(previous.action, None);
            assert_eq!(
                defender.guard, None,
                "committing to an attack lowers the guard"
            );
            attack_started = true;
            previous = defender;
            break;
        }
        assert_eq!(
            defender.guard, RAISED,
            "it keeps its guard up until it attacks"
        );
        previous = defender;
    }
    assert!(attack_started, "a defender in reach of its target attacks");
    assert!(previous.target_player_id == Some(1));

    // The guard stays down for the whole attack.
    while arena.defender().action.is_some() {
        arena.tick(1);
        let defender = arena.defender();
        if defender.action.is_some() {
            assert_eq!(defender.guard, None, "no guard while attacking");
        }
    }

    // Once its target is out of reach again, it raises the guard again from the start.
    arena.arrange(FAR, None);
    let mut stances = Vec::new();
    for _ in 0..usize::from(values.raise_ticks) + 4 {
        arena.tick(1);
        stances.push(arena.defender().guard);
    }
    let first = stances
        .iter()
        .position(Option::is_some)
        .expect("the guard rises again after the attack");
    assert_eq!(stances[first], raising(values.raise_ticks));
    assert!(stances.contains(&RAISED));
}

#[test]
fn defender_guard_timeline_is_deterministic_for_a_seed() {
    let mut first = Arena::new(&[1]);
    let mut second = Arena::new(&[1]);
    assert_eq!(first.seed, second.seed);
    let a = timeline(&mut first, 400);
    let b = timeline(&mut second, 400);
    assert!(
        a.iter()
            .any(|snapshot| defender_of(snapshot).is_some_and(|defender| defender.guard == RAISED)),
        "the timeline covers a raised guard"
    );
    assert_eq!(a, b, "same seed and inputs, same guard timeline");
}

#[test]
fn a_frontal_sword_swing_meets_the_raised_shield_and_spends_its_guard() {
    let values = values();
    assert!(
        values.light_guard_cost > 0,
        "a blocked sword swing must spend guard, or a raised shield can never break"
    );
    let mut arena = Arena::new(&[1]);
    arena.until_raised();
    // The player steps up to sword reach of the advancing defender and swings.
    arena.arrange(values.light_reach - 1, None);
    let before = arena.defender();
    assert_eq!(before.guard, RAISED);
    arena.command(1, ArpgCommand::PrimaryAttack);
    let mut results = Vec::new();
    for _ in 0..20 {
        results.extend(arena.tick(1));
        if !results.is_empty() {
            break;
        }
    }
    assert_eq!(
        results,
        [StrikeResult::Blocked {
            guard_damage: values.light_guard_cost
        }],
        "a frontal swing at an advancing defender is caught by its shield"
    );
    let after = arena.defender();
    assert_eq!(after.health, before.health, "a block deals no damage");
    assert_eq!(
        after.guard_points,
        before.guard_points - values.light_guard_cost,
        "the block spends the strike's guard cost"
    );
    assert_eq!(after.guard, RAISED, "a block keeps the guard up");
}

#[test]
fn a_raising_defender_guard_does_not_protect_yet() {
    let values = values();
    let mut arena = Arena::new(&[1]);
    arena.until_raising();
    let before = arena.defender();
    arena.arrange(near(&values), Some(swing_now([0, 0])));
    let results = arena.tick(1);
    let [StrikeResult::Hit { damage, .. }] = results[..] else {
        panic!("a strike on a rising guard hits, like on a rising player shield: {results:?}");
    };
    assert!(damage > 0);
    let after = arena.defender();
    assert_eq!(after.health, before.health - damage);
    assert_eq!(after.guard_points, before.guard_points, "no guard is spent");
}

#[test]
fn repeated_frontal_strikes_break_the_guard_which_recovers_by_the_shared_rules() {
    let values = values();
    let mut arena = Arena::new(&[1]);
    let health = arena.defender().health;
    let mut broken = false;
    for _ in 0..200 {
        arena.until_raised();
        arena.arrange(near(&values), Some(swing_now([0, 0])));
        let before = arena.defender();
        assert_eq!(before.guard, RAISED);
        let results = arena.tick(1);
        let after = arena.defender();
        assert_eq!(
            after.health, health,
            "blocked and breaking strikes deal no damage"
        );
        if values.light_guard_cost >= before.guard_points {
            assert_eq!(results, [StrikeResult::GuardBroken]);
            assert_eq!(after.guard, None, "a broken guard drops");
            assert_eq!(after.guard_points, 0);
            broken = true;
            break;
        }
        assert_eq!(
            results,
            [StrikeResult::Blocked {
                guard_damage: values.light_guard_cost
            }]
        );
        assert_eq!(
            after.guard_points,
            before.guard_points - values.light_guard_cost
        );
        arena.arrange(FAR, None);
    }
    assert!(broken, "enough frontal strikes break the defender's guard");

    // Broken: down and not regenerating for the shared break ticks, then it rises again.
    arena.arrange(FAR, None);
    for elapsed in 1..values.break_ticks {
        arena.tick(1);
        let defender = arena.defender();
        assert_eq!(defender.guard, None, "{elapsed} ticks after the break");
        assert_eq!(defender.guard_points, 0, "no regeneration while broken");
        if arena.distance_to_player(1) < FAR / 2 {
            arena.arrange(FAR, None);
        }
    }
    let mut rose = false;
    for _ in 0..usize::from(values.raise_ticks) + 4 {
        arena.tick(1);
        if arena.defender().guard == raising(values.raise_ticks) {
            rose = true;
            break;
        }
    }
    assert!(
        rose,
        "after the break period the defender raises its guard again"
    );
}

#[test]
fn strikes_from_behind_or_beside_land_on_a_guarding_defender() {
    let values = values();
    let mut arena = Arena::new(&[1, 2]);
    arena.until_raised();
    let base = arena.game.save_state().expect("save");
    let reach = near(&values);
    let side = arena.side;
    // Player 1 stays FAR in front as the defender's target; player 2 strikes from each
    // position in the very next tick.
    let cases = [
        ("in front", [side * reach, 0], true),
        ("behind", [-side * reach, 0], false),
        ("beside", [0, reach], false),
    ];
    for (name, offset, blocked) in cases {
        arena.game = ArpgGame::from_save_state(base.clone()).expect("restore");
        arena.arrange(FAR, None);
        arena.arrange_player(2, offset, Some(swing_now([0, 0])));
        let before = arena.defender();
        assert_eq!(before.guard, RAISED, "{name}");
        assert_eq!(before.target_player_id, Some(1), "{name}");
        let results = arena.tick(2);
        let after = arena.defender();
        if blocked {
            assert_eq!(
                results,
                [StrikeResult::Blocked {
                    guard_damage: values.light_guard_cost
                }],
                "{name}"
            );
            assert_eq!(after.health, before.health, "{name}");
        } else {
            let [StrikeResult::Hit { damage, .. }] = results[..] else {
                panic!("a strike {name} the defender lands: {results:?}");
            };
            assert!(damage > 0, "{name}");
            assert_eq!(after.health, before.health - damage, "{name}");
            assert_eq!(
                after.guard_points, before.guard_points,
                "{name}: an unblocked strike spends no guard"
            );
        }
    }
}

#[test]
fn defender_guard_survives_save_and_restore() {
    let values = values();
    let mut arena = Arena::new(&[1]);
    // Mid-raise.
    arena.until_raising();
    assert_continues_identically(&mut arena, "mid-raise");

    // Raised with spent guard (a block's reaction still running).
    arena.until_raised();
    arena.arrange(near(&values), Some(swing_now([0, 0])));
    assert!(matches!(arena.tick(1)[..], [StrikeResult::Blocked { .. }]));
    let defender = arena.defender();
    assert!(defender.guard_points < defender.max_guard_points);
    assert_continues_identically(&mut arena, "after a block");

    // Broken.
    arena.break_guard();
    arena.tick(1);
    assert_continues_identically(&mut arena, "while broken");
}

/// Saves `arena`'s game through its serialised form and checks the restored game publishes
/// the same defender and then evolves exactly like the uninterrupted one.
fn assert_continues_identically(arena: &mut Arena, moment: &str) {
    let json = serde_json::to_string(&arena.game.save_state().expect("save")).expect("serialise");
    let mut restored = ArpgGame::from_save_state(serde_json::from_str(&json).expect("deserialise"))
        .expect("restore");
    assert_eq!(
        defender_of(&restored.snapshot().expect("snapshot")),
        Some(&arena.defender()),
        "{moment}: the restored defender publishes the same guard"
    );
    for tick in 0..120 {
        arena.game.advance_tick().expect("tick");
        restored.advance_tick().expect("tick");
        assert_eq!(
            restored.snapshot().expect("snapshot"),
            arena.snapshot(),
            "{moment}: tick {tick} after restore"
        );
    }
}

#[test]
fn monster_snapshot_publishes_the_guard_for_presentation() {
    let mut arena = Arena::new(&[1]);
    arena.until_raised();
    let snapshot = arena.snapshot();
    let defender = defender_of(&snapshot).expect("defender");
    let json = serde_json::to_value(defender).expect("monster snapshot serialises");
    assert_eq!(
        json["guard"],
        serde_json::json!({ "phase": "raised", "ticksRemaining": 0 })
    );
    assert_eq!(json["guardPoints"], defender.guard_points);
    assert_eq!(json["maxGuardPoints"], defender.max_guard_points);
    for monster in &snapshot.monsters {
        if monster.definition != DEFENDER {
            assert_eq!(
                monster.guard, None,
                "{} carries no shield",
                monster.definition
            );
        }
    }
}
