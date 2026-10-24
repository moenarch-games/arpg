# Architecture

`arpg` is the game-specific composition layer. It owns ARPG rules and content semantics while consuming reusable foundations for rendering, physics, assets, and multiplayer infrastructure.

## Authority map

- `arpg-core` owns deterministic gameplay state and rules: characters, monsters, stats, skills, damage, cooldowns, inventory, loot, AI decisions, progression, quests, dungeon state, and seeded game randomness.
- Rapier (`rapier3d`) is the authoritative physics backend until `physics-engine` is finished ([Atlas 2.3.1](decisions/atlas.md)): it resolves collision, contacts, sliding and spatial queries inside the Rust/WASM simulation ([`physics.rs`](../crates/arpg-core/src/physics.rs)), while `arpg-core` keeps movement intent and every gameplay outcome. Concrete Rapier gaps and lessons become physics-engine issues. Physical correction is never approximated in the renderer.
- `3d-lab` owns reusable 3D geometry/camera/animation/asset models and browser rendering primitives. `arpg` owns scene composition and game presentation policy.
- `asset-tooling` owns reproducible asset acquisition/generation/processing evidence. Runtime code consumes accepted artifacts rather than generation machinery.
- `MOEL` is the preferred data format for schema-validated game content once the first content slice lands.
- `game-server` owns reusable server-authoritative match runtime concerns.
- `multiplayer-setup-service` owns peer rendezvous/signaling only.

Neither renderer nor network transport may become an alternate source of gameplay truth.

## Physics A/B against physics-engine (test and bench only)

[physics-engine#330](https://github.com/moritzbrantner/physics-engine/issues/330) compares the
real game on both physics backends. Rapier stays the production authority (Atlas 2.3.1): the
non-default `arpg-core` feature `physics-ab` compiles the second backend, and no product,
server or WASM build enables it.

- **Seam.** [`physics.rs`](../crates/arpg-core/src/physics.rs) keeps the game-facing API. The
  Rapier body lives in [`physics/rapier.rs`](../crates/arpg-core/src/physics/rapier.rs); with
  `physics-ab`, `Physics` becomes an enum over it and
  [`physics/engine.rs`](../crates/arpg-core/src/physics/engine.rs), a thin adapter over
  `physics_engine::approximate::World`, chosen per thread by `physics_ab::with_backend` when a
  world is created. Call sites do not change.
- **Adapter.** The same metre conversion as Rapier (`Config::length_unit` stays 1); the 0.25
  unit contact offset; the floor a static half-space, excluded from planar moves and rays and
  counted by `penetrates_fixed` (on its own collision layer in the adapter); only static
  ground is support; players snap within the support
  reach; monsters move planar; each game move is its own one-element `move_characters` batch,
  in the game's order; doors are added and removed as static boxes; positions are mirrored as
  the game computes them, so a restored save continues exactly. `rebuild` does nothing: the
  engine's results do not depend on insertion or move history.
- **Acceptance.** `physics_ab::tests` (run by `cargo test --all-features`) drives every seeded
  workload through the real game on both backends with the same fixture, commands and ticks.
  Both must keep finite state, no floor, wall or door penetration or tunnelling, planar
  monsters, grounded players on the ground, non-overlapping characters, blocking locked doors,
  bit-identical replay and exact save continuation. Across them, game-visible outcomes
  (doors, encounters, health, progression, support, region membership, strikes,
  interactions) must be identical
  every tick, sight, strike, ray, ground and placement queries from the same inputs must
  agree (ray distances within 0.01 units), and positions must agree within 0.5 units until two
  characters first touch. The [physics contract tests](../crates/arpg-core/src/physics.rs) run
  on both backends.
- **Intentional divergences.** A character blocked head-on by another sidesteps to its right
  on physics-engine and stops on Rapier, and sliding around another character differs, so
  positions after the first character contact are reported, not bounded (up to about 195 units
  in the four-player workloads, 2.6 units in `door`; outcomes stay identical). On the tick a
  body rounds a platform's edge, the edge contact's 0.01 unit normal nudge can make Rapier's
  move count as rising, which skips its snap to the ground: the body stays up to the rounded
  edge's drop (0.6 units in `ledge`) higher for a tick while the engine follows the edge, so
  edge ticks are bounded at 2 units instead (#208). Rapier adds a 0.01
  unit normal nudge per contact (up to 0.26 units of drift out of a corner) and pushes a body
  placed into a wall out to touching, where the engine keeps the 0.25 unit offset. An
  unobstructed move is exact on both. `ground_height` probes 1 km down instead of an unbounded ray, a zero-length
  `first_fixed_hit` meets nothing, and a character registers as planar until its first
  `move_player`.
- **Timing.** `cargo run --release --locked -p arpg-core --features physics-ab --example
  physics_ab` reports per-tick whole-physics-path time (median, p95, p99), setup time and the
  share of Rapier's per-tick rebuild, natively, for every workload. It is manual, not a gate.

  Last run (physics-engine `baa9244`, seed 42, 240 ticks, 3 warm-up and 20 paired runs per
  workload, µs per tick; one laptop, so read ratios rather than absolute times). Most of
  Rapier's tick is the rebuild that exact save continuation needs; the engine needs none.

  | workload | players | static boxes | peak monster bodies | backend | median | p95 | p99 | setup median | rebuild share | engine/rapier total |
  |---|---:|---:|---:|---|---:|---:|---:|---:|---:|---:|
  | quiet | 1 | 28 | 0 | rapier | 9.4 | 11.0 | 19.0 | 31.2 | 85% |  |
  | quiet | 1 | 28 | 0 | physics-engine | 1.4 | 1.6 | 1.8 | 31.5 | 1% | 0.15 |
  | party-quiet | 4 | 28 | 0 | rapier | 13.5 | 16.2 | 26.6 | 36.5 | 69% |  |
  | party-quiet | 4 | 28 | 0 | physics-engine | 2.3 | 2.7 | 4.0 | 55.2 | 1% | 0.17 |
  | sparse | 4 | 28 | 0 | rapier | 14.8 | 33.5 | 41.1 | 38.6 | 55% |  |
  | sparse | 4 | 28 | 0 | physics-engine | 3.4 | 9.4 | 20.4 | 56.3 | 0% | 0.29 |
  | corner | 1 | 28 | 0 | rapier | 14.2 | 17.9 | 30.9 | 33.0 | 57% |  |
  | corner | 1 | 28 | 0 | physics-engine | 8.0 | 9.4 | 16.3 | 34.7 | 0% | 0.57 |
  | door | 1 | 31 | 1 | rapier | 14.3 | 20.7 | 39.8 | 36.1 | 73% |  |
  | door | 1 | 31 | 1 | physics-engine | 3.1 | 5.3 | 13.6 | 40.4 | 0% | 0.23 |
  | correction | 1 | 28 | 0 | rapier | 9.5 | 11.8 | 23.6 | 35.2 | 84% |  |
  | correction | 1 | 28 | 0 | physics-engine | 1.4 | 1.6 | 1.9 | 33.9 | 1% | 0.15 |
  | combat | 1 | 28 | 1 | rapier | 9.9 | 13.5 | 24.0 | 35.9 | 82% |  |
  | combat | 1 | 28 | 1 | physics-engine | 1.4 | 1.7 | 2.8 | 54.9 | 1% | 0.15 |
  | crowd | 1 | 31 | 5 | rapier | 15.4 | 20.8 | 34.8 | 40.2 | 73% |  |
  | crowd | 1 | 31 | 5 | physics-engine | 2.8 | 6.4 | 11.1 | 60.1 | 0% | 0.20 |
  | lifecycle | 4 | 28 | 0 | rapier | 15.1 | 31.5 | 43.7 | 39.4 | 59% |  |
  | lifecycle | 4 | 28 | 0 | physics-engine | 3.4 | 10.7 | 22.5 | 62.5 | 0% | 0.29 |
  | chase | 1 | 31 | 5 | rapier | 19.6 | 32.4 | 59.1 | 45.2 | 62% |  |
  | chase | 1 | 31 | 5 | physics-engine | 6.0 | 9.2 | 14.9 | 63.0 | 0% | 0.29 |
  | jump | 1 | 28 | 0 | rapier | 11.5 | 14.1 | 24.8 | 30.9 | 68% |  |
  | jump | 1 | 28 | 0 | physics-engine | 3.6 | 4.7 | 6.3 | 33.5 | 0% | 0.31 |
  | ledge | 1 | 36 | 1 | rapier | 16.9 | 40.7 | 70.3 | 68.4 | 65% |  |
  | ledge | 1 | 36 | 1 | physics-engine | 3.9 | 20.7 | 29.6 | 81.7 | 0% | 0.27 |
  | ceiling | 1 | 36 | 1 | rapier | 14.8 | 21.8 | 35.6 | 45.1 | 72% |  |
  | ceiling | 1 | 36 | 1 | physics-engine | 3.5 | 5.7 | 9.0 | 69.9 | 0% | 0.23 |
  | traversal | 1 | 36 | 1 | rapier | 14.8 | 32.3 | 55.8 | 65.1 | 69% |  |
  | traversal | 1 | 36 | 1 | physics-engine | 2.9 | 10.6 | 25.3 | 70.4 | 1% | 0.21 |
  | lunge | 1 | 30 | 1 | rapier | 12.7 | 17.3 | 31.8 | 37.2 | 75% |  |
  | lunge | 1 | 30 | 1 | physics-engine | 2.6 | 6.9 | 8.0 | 61.7 | 1% | 0.23 |
  | crates | 1 | 29 | 1 | rapier | 15.4 | 20.5 | 40.9 | 43.7 | 62% |  |
  | crates | 1 | 29 | 1 | physics-engine | 4.9 | 6.4 | 11.7 | 65.1 | 0% | 0.28 |
  | stairs | 1 | 36 | 1 | rapier | 24.7 | 40.4 | 65.6 | 47.1 | 48% |  |
  | stairs | 1 | 36 | 1 | physics-engine | 3.3 | 9.1 | 12.9 | 71.5 | 0% | 0.16 |
  | slam | 3 | 31 | 1 | rapier | 13.9 | 17.9 | 28.2 | 40.5 | 71% |  |
  | slam | 3 | 31 | 1 | physics-engine | 2.2 | 3.3 | 5.5 | 57.8 | 1% | 0.16 |
  | poison | 1 | 31 | 1 | rapier | 13.0 | 16.1 | 25.5 | 36.6 | 74% |  |
  | poison | 1 | 31 | 1 | physics-engine | 2.7 | 4.0 | 4.7 | 56.6 | 1% | 0.21 |
  | slow | 1 | 31 | 1 | rapier | 11.8 | 14.5 | 24.5 | 35.9 | 77% |  |
  | slow | 1 | 31 | 1 | physics-engine | 2.2 | 2.9 | 3.6 | 54.0 | 1% | 0.18 |
  | all | | | | rapier | 14.1 | 27.3 | 44.7 | | | |
  | all | | | | physics-engine | 2.9 | 8.3 | 18.7 | | | |

## Height and traversal topology

Real physical height and semantic walkable topology are separate but coordinated
authorities. The physics backend (Rapier for now) owns continuous physical truth; `arpg-core`
owns stable walkable regions, typed traversal connections, actor region/transit
state, and ability-specific cross-region rules.

The first slice (#59) is flat-ground jumping: the floor is a Rapier half-space, players
move with gravity under Rapier's character controller, and support (grounded, falling,
landing) comes from the controller's grounded test confirmed by a support query against
static ground. Jump eligibility, the coyote and buffer windows, airborne control and the
grounded-only guard and counter are `arpg-core` rules over that state; the tuning is
content (`jump`). Snapshots publish each player's `motion` (support, vertical velocity,
feet and ground height) for presentation, which places the humanoid at the feet and the
ground ring on the ground, never correcting either.

The walkable topology (#207, [`topology.rs`](../crates/arpg-core/src/topology.rs)) holds
stable regions and typed connections (walk, jump, one-way drop, gated). Until elevation
exists a level is one region per flat floor (Atlas 5.1.2): the generated dungeon is a single
region and its doors are gated passages inside it. Each player belongs to a region or is in
transit along a connection; landing settles it into the region whose floor supports it,
airborne motion keeps it, and undeclared support changes nothing. Monsters, loot and chests
belong to the base floor. Melee strikes, monster engagement, target locks and interaction are
region-local; an actor in transit counts as present at both ends, so transit is no
invulnerability. Arrows and monster projectiles stay physical and hit what they reach (the
cross-region opt-in ADR 0002 names for arrows). Snapshots (schema 23) and saves (schema 16)
carry the membership; a grounded saved player must belong to the region supporting it.

Box tops are support on both backends (#208): the `traversal` workbench scenario lays a
platform 60 units high (within a jump's 90-unit apex) as its own region, with a jump
connection up and a drop down. Without autostep, its side blocks a walk and a jump climbs it;
walking off its ledge opens the coyote window and drops to the floor region. Save validation
measures height against the ground physics finds below the body, and bounds the fall speed
by a drop from a jump above the highest floor. The seeded A/B `ledge` workload runs it on
both backends (saved and restored as well).

The same scenario lays a low ceiling (#209) 140 units up: a standing body walks under it, a
jump stops with the head a contact offset short of it (the tick's vertical velocity becomes
what physics allowed, so the body falls back at once), and monsters path under it, since
navigation ignores boxes entirely above a monster body. The seeded A/B `ceiling` workload
jumps into it on both backends.

The bridge (#210) adjoins the platform's far edge: its top, 140 units up, is a jump from the
platform but out of reach from the floor, and its underside, 120 units up, leaves the floor
path beneath it walkable, so two regions overlap in X/Z at different heights. Raised regions
carry their area (the box top); support picks the region whose floor height and area (plus a
body's radius, for edges) match. An airborne body enters transit along the declared
connection when it rises over a higher region it can land on or its centre leaves its own
region's area, and landing resolves it; a body in transit counts as present at both ends.
Each scenario layout is validated: a crossing the geometry permits (a rise within a jump's
apex over a gap within its reach, or any drop) without a declared connection rejects the
level. The seeded A/B `traversal` workload climbs platform and bridge, drops off the bridge's
side and walks the path beneath it on both backends.

Stairs (Atlas 5.1.5, #222): a walking player climbs steps through Rapier's
character-controller `autostep` with a 25-unit step height. A move walks when the previous
move ended in ground contact (Rapier's `grounded`, confirmed on static geometry at any angle,
so a step's rounded edge counts and another character never does; saved as `groundContact`,
schema 20) and the move does not take off. Gating on walkable support instead would drop a
body half up a rounded edge out of walking and stall it there. Stair rises stay at about 0.8
of the step height or less (the scenario's are 20), where climbing a rounded edge clearly
outpaces gravity; the capsule's rounded base climbs an edge about 9 units further as a
slope (Rapier's 45 degree rule), so steps up to about 33 units climb and 35 block. Jumps and
falls never step, so a jump keeps landing on a ledge only from above. Stairs belong to the
region they lead into: standing on a step keeps the membership, and a saved grounded body is
checked by a downward capsule cast (`supported_within`, edges and steps included) rather than
the ground straight below. The `traversal` scenario's two 20-unit steps climb to the
platform's north face (a declared walk connection). physics-engine (`baa9244`) climbs them
with its own autostep (`CharacterSettings::autostep`, the same height and free depth), which
it applies only to moves that start on the ground and do not rise; the seeded A/B `stairs`
workload compares them. Known divergence, reported to the engine owner: the engine's walking
check counts support at the move's start, which a body half up a step's rounded edge lacks,
where Rapier counts the edge contact as grounded and keeps stepping, so the engine climbs that
edge a tick behind. While a player is on the stairs, positions are bounded at 7.5 units (7.06
measured with `baa9244`) and support and region changes may differ on at most 7 ticks; every other tick and
workload keeps the strict comparison.

Ramps (#59): a ramp is a static wedge inside its box (`StaticColliderKind::Ramp`), rising from
the box's bottom at its +Z end to its top at its -Z end: a convex hull on Rapier, a
quarter-turned `Shape::Wedge` on physics-engine. Walking players follow its slope through the
controllers' 45 degree climb limit, grounded the whole way, with no autostep or jump. The
`traversal` scenario's 180-unit ramp (18 degrees, 80 wide) leads up to the west part of the
platform's south face; like the stairs it belongs to the region it leads into, along the same
declared walk connection, so a body on the slope keeps its membership (no artificial height
bands) until the platform's top supports it. Monsters treat the ramp's footprint as an
obstacle. The ledge east of it stays a drop.

Fall recovery (Atlas 5.1.3, #211): each player keeps the position at which it last ended a
tick supported (saved, schema 17). A body that ends a tick out of the world (its centre
outside the rooms' combined extents, its feet a body height below the lowest floor, or a
non-finite position) returns there at rest and supported, with no damage; actions, guard and
buffered input carry on. Current geometry offers no hole to fall through, so only a body
displaced out of the dungeon triggers it; the rule is tested on both physics backends.

Combat and interaction are region-local by default. Abilities may explicitly
opt into cross-region 3D range/line-of-sight checks. Authored and procedurally
generated levels must both provide topology matching their geometry; undeclared
physical crossings are invalid level content rather than runtime exceptions.

See [ADR 0002](adr/0002-height-traversal-topology.md) for the full contract.

## Swept melee

A lunging monster attack (#90, Atlas 5.2.1; the skirmisher's `monster.lunge`) commits, as its
active window opens, to where its target stands; for every active tick its steering drives
the body that way through the physics seam (`move_character`, so walls and bodies stop it),
and its strike sweeps the segment the body actually travelled that tick: reach and
obstruction are measured from the segment's point nearest the target. The strike keeps the
identity of the tick its window opened on and connects at most once. Saves (schema 18) carry
the lunge in progress. The seeded A/B `lunge` workload runs it on both backends.

Damageable crates (#90, Atlas 5.2.2; content `crate`) stand in the corners of every combat
room, two per room, five units off the walls and taller than a jump's apex (so they are no
walkable ground). A standing crate is a static box: it blocks movement and navigation, stops
monster shots and obstructs strikes at what lies behind it. Player melee strikes may target it
like a monster, nearest first within the strike volume, and a player's arrow that reaches it
first damages it through the same strike outcome path (Atlas 5.2.3, #223; a real box, so a
shot can pass over it; the wall ray meets the crate's own box, which wins that tie); the obstruction ray of a strike at a crate
ignores that crate (`Physics::segment_obstructed_except`, on both backends: Rapier filters the
collider out with a query predicate, the physics-engine adapter puts the floor on its own
collision layer so `QueryFilter::exclude` is free for the crate). A broken crate leaves the
physics world and drops nothing. Snapshots (schema 24) list standing crates and export them as
`crate` colliders; saves (schema 19) keep every crate's health. The seeded A/B `crates`
workload shoots the room's far crate, breaks the near one and walks through its spot on both
backends.

## Area effects

The bruiser's slam is a ground burst (#64, Atlas 5.3.1; content `burst: true`): when its
45-tick telegraph ends, an overlap query through the physics seam
(`Physics::characters_in_column`, an upright capsule of the slam's 220-unit reach around the
bruiser, on both backends) yields the candidate bodies, and the shared strike path lands the
unblockable slam on every living player among them within reach, in the bruiser's region and
not behind a wall. Monsters are never candidates (no friendly fire). Snapshots publish the
attack's delivery as `burst`, which the renderer telegraphs with the strike's range ring. The
seeded A/B `slam` workload runs it with three players on both backends, and the A/B query
comparison checks the area query from every body until characters touch.

Poison (#64, Atlas 5.3.2; content `poison`): each combat room has a poison pool on the floor of
its south-west corner. Every tick, after motion, an overlap query through the same seam (a
sphere of the pool's 60-unit radius on the floor, so a jump clears it) finds the players whose
bodies touch a pool; each is poisoned for 180 ticks, a reapplication refreshing the duration
and keeping the damage cadence (it never stacks). Poison deals 2 damage every 30 ticks through
the shared strike outcome path (source `hazard`, definition `hazard.poison`, no stagger) and
ends with its duration or its player's defeat. Snapshots (schema 25) publish the pools as
`hazards` and each player's poison; saves (schema 21) keep it. The seeded A/B `poison`
workload walks into a pool and out on both backends, and the A/B harness compares every
pool's query each tick.

Slow (#64, Atlas 5.3.2; content `slow`): a player's arrow that hits a living monster slows it
for 90 ticks, a further hit refreshing the duration (never stacking); a slowed monster's
pursuit and retreat steering runs at half speed through the same physics moves. Snapshots
(schema 26) and saves (schema 22) carry each monster's slow; the A/B outcomes compare it every
tick, and the seeded A/B `slow` workload shoots a pursuing monster on both backends.

The browser's presentation snapshot boundary and React update ownership are
documented in [ADR 0001](adr/0001-browser-snapshot-projection.md). Tick-dependent
views and rendering consume one session-owned snapshot store; menus and shell
configuration do not subscribe to simulation ticks.

React composes but does not own browser runtime lifetimes. `web/src/game-client-runtime.ts`
owns the active gameplay source and the Rust/Wasm core load (unloaded, loading, ready or
failed with retry; a start before the core is ready activates only if it is still the
latest request). `web/src/frame-presenter.ts` owns the renderer, frame projection and
resize observer of one canvas, and `web/src/input-attachment.ts` owns the keyboard
attachment and its gameplay/menu context. Components create each owner once, forward
prop changes in place and dispose it on unmount.

## Mechanical game-loop foundation

The moment-to-moment ARPG loop is a game-specific foundation, not a late presentation/polish layer.

`arpg-core` must expose coherent domain contracts for:

- locomotion and movement/action cancellation;
- action phases (intent, commitment/wind-up, active effect, recovery, interruption);
- targeting and interaction resolution;
- authoritative hit/damage/reaction outcomes;
- stagger, knockback, block, immunity, death, and related combat consequences;
- item/drop spawn and pickup interaction;
- typed gameplay events/cues that presentation layers can consume.

This layer sits **above** reusable foundations such as the physics backend, `input-bindings`, and `3d-lab`. It should not be generalized into those repositories merely because multiple ARPG features use it. Weapons, skills, monsters, bosses, loot, and dungeon rules should compose the ARPG game-loop contracts rather than each introducing private movement, timing, targeting, hit-reaction, or pickup systems.

The presentation boundary is intentionally one-way: animation, rendering, camera, audio, HUD, particles, and local visual emphasis may react to authoritative state/events, interpolate them, and decorate them. They may not decide that an attack hit, a target was valid, an action completed, a character was displaced, or an item was picked up.

Likewise, animation timing must not silently become gameplay authority. Gameplay timings live in deterministic core state; animation consumes phase/cue timing and visually represents it.

The first acceptance environment for this foundation is a deterministic combat training arena with reproducible movement, attack, impact, interruption, targeting, projectile, knockback, death, and pickup scenarios. See [ROADMAP.md](ROADMAP.md).

## Saves

`ArpgSaveState` (`arpg-core`) is the gameplay part of a save, and the only part a load
trusts. `ArpgGame::from_save_state` validates the whole save before it builds a new game.
A refused save never touches the game already running, and callers swap authorities only
after a load succeeds.

**Version policy (before 1.0).** A save loads only under the exact
`SAVE_STATE_SCHEMA_VERSION`, `SAVE_STATE_RULES_VERSION` and content revision that wrote it.
Nothing is migrated, and any other version is refused with a reason. A change to the saved
format or to the rules it continues under bumps the version once, with no shim. The
checked-in fixtures in `crates/arpg-core/tests/fixtures/saves` cover this: one current-schema
save that must load and save back unchanged, plus the previous schema, a corrupt save and a
truncated save, all of which must be refused (`tests/save_fixtures.rs`). After a bump,
regenerate them with `UPDATE_SAVE_FIXTURES=1 cargo test -p arpg-core --test save_fixtures`,
which keeps the outdated save as `previous-schema.json`. The browser workflow test imports
the same files, plus an oversized one, through the Wasm load.

**Canonical: saved, and restored exactly.**

- Run identity: run seed, scenario, content revision, tick.
- Players: `f32` position and velocity, support (grounded, ground contact, coyote and
  jump-buffer ticks, last supported position, region membership or transit), movement and
  facing intent, action and its phase progress, hurt ticks, guard, counter opportunity,
  weapon, bow draw, aim and lock, poison, health, experience, gold, checkpoint room, and
  both command-lane watermarks (`lastSequence`, `lastReliableSequence`). The watermarks are
  the command fences, so a stale command stays stale after a load.
- Monsters: position, health, action (phase, ticks, target, lunge), stagger, post,
  engagement, steering flags, guard, slow.
- World: each room's encounter state, opened chests, crate health, ground loot, arrows in
  flight, and the next ground-loot and arrow ids (ID allocation).

**Derived: rebuilt on load from the seed, scenario and content.** Room and door layout
(door locks follow encounter state), static colliders, hazards, chest placement (chest
availability follows room state and opened chests), crate placement, monster definitions
and ids, level and maximum health (from experience), the Rapier world (Atlas 2.3.2), and
the navigation cache, which is dropped and recomputed with the same result.

**Transient: reset on load.** The strike and interaction events of the last tick (a
restored game publishes none until its next tick) and in-tick scratch state such as a
monster's `cornered` and `swept_from`.

**Outside the core save.** The browser save document (`web/src/save-state.js`, file
version 1) wraps the core state with presentation (`characterId`) and client
(`controlledPlayerId`) fields. Those fields never affect gameplay. The shell restores its
held inputs from the saved player. Display preferences such as camera zoom live in their
own storage and are never part of a save.

`saves_at_each_mechanic_continue_the_uninterrupted_fight_exactly` (`arpg-core`, run on
both physics backends) checks exact continuation. It saves one scripted fight at mid-jump,
during a wind-up, during pursuit, with an arrow in flight while a slow runs, and with a
partly cleared encounter. Each save goes through the JSON load and receives the same
remaining commands. On every later tick the restored game's save state, its snapshot with
events, and the snapshot's encoded bytes must equal those of the uninterrupted game.

## Execution modes

All modes execute the same `arpg-core` rules and use the same game-specific command/snapshot protocol.

### Local

The platform shell runs `arpg-core` directly. No networking abstraction is required in the hot path.

### Peer-hosted co-op

`multiplayer-setup-service` creates a short-lived lobby and establishes WebRTC connectivity. ARPG uses the service's **host topology**: one browser is the gameplay host, runs the authoritative Rust/Wasm simulation, accepts sequenced player commands, and publishes authoritative snapshots to guests.

The setup service remains payload-opaque. It must never receive or interpret combat, loot, inventory, movement, world state, or anti-cheat decisions.

Peer-hosted authority is a convenience/trust model, not an anti-cheat boundary. The host can cheat. Guests are never allowed to mutate authoritative state directly.

### Dedicated online

`game-server` hosts the authoritative ARPG simulation. `arpg-game-server` is the narrow adapter from the transport-neutral core/protocol boundary to `game_server::GameSimulation`.

`game-server` remains responsible for server-owned player/session identity, tick scheduling, command watermarks, snapshots, reconnect/resume, replay evidence, multi-match hosting, draining, and graceful recovery. ARPG does not duplicate those mechanisms.

#### Dedicated command delivery

Dedicated commands use two `game-server` command lanes ([game-server ADR 0008](https://github.com/moritzbrantner/game-server/blob/main/docs/adr/0008-reliable-command-lane.md)). The browser picks the lane by command kind; gameplay rules never depend on it.

- **Held state** (`setMovement`, `setAim`, `setGuard`) travels as best-effort WebTransport datagrams with latest-wins semantics. Each one carries an absolute value that the authority can take again at any time without a side effect. A datagram may be lost, and one overtaken by a newer datagram is ignored as stale. The client re-asserts the whole held set on a bounded cadence (#139), so the authority converges on what the player holds within a few refresh periods.
- **One-shots and bow transitions** (`primaryAttack`, `secondaryAttack`, `interact`, `equipWeapon`, `cycleTarget`, `clearTarget`, `jump`, `drawBow`, `releaseBow`, `cancelBow`) travel on the connection's reliable command lane. The bow transitions are held input, but a release consumes the draw it shoots. As datagrams, a lost release followed by a new draw would leave the first draw in place, and the next release would fire one shot for two releases. On the lane, draw, release and cancel are lossless and stay in order among themselves, and the refresh never repeats them. A draw that reaches the authority during a shot is ignored, exactly as in local play. The lane is one bidirectional stream, opened when the first lane command of a connection is sent, that carries the commands in order. The authority acknowledges each one as applied, duplicate or rejected. `DedicatedGameSession` keeps every command until its ack (at most 256), and after a resume re-sends the unacknowledged ones, in order, before newer ones. A re-sent command that already reached the authority before the outage is a duplicate: the reliable watermark ignores it, so an attack or interaction applies at most once while the player keeps its slot.

`MatchRuntime` keeps one sequence watermark per (player, lane), and `ArpgGame` mirrors that through `AuthoritativeGame::apply_lane_command`: a command is stale only against earlier commands of its own lane, so a one-shot overtaken by a newer held datagram still applies. `apply_command` is the datagram lane, which local and peer play use. Player saves keep both watermarks (`lastSequence` and the defaulted `lastReliableSequence`). The browser uses one sequence counter for both lanes, which keeps each lane strictly increasing.

Ordering holds only within a lane. A one-shot and a held datagram may reach the authority in either order. Replay and recovery record the order that actually happened.

The one-shots that commit a direction therefore carry it (#156, protocol v20). `primaryAttack`, `secondaryAttack` and `releaseBow` take an optional `aim: [x, z]`, which the browser fills from its current aim in every mode. `arpg-core` validates it exactly like `setAim` (non-zero, components within ±`AIM_COMPONENT_LIMIT`; an invalid aim rejects the whole command) and applies it as the held aim before the action, in the same command. A lost or late `setAim` datagram then cannot steer a shot or strike. An absent or `null` aim keeps the held aim in charge, and a target lock still takes precedence over aim intent. A lost datagram that *clears* the aim is still repaired only by the held-input refresh.

Failures:

- A **rejected** ack means the simulation refused that command with a `game-server` `command_rejected`. It is final for that attempt, it is reported as a status message, and the session continues. ARPG does not reject one-shots today: a command that cannot take effect, such as a weapon change during an action, is accepted and does nothing.
- A **malformed or unexpected ack**, or an ack stream that ends, fails closed: the client closes the connection and resumes it like any transport failure, keeping its unacknowledged one-shots for the new lane. Lane faults on the server side (a malformed frame, a fatal simulation error, a frozen runtime) also close the connection, with the same outcome.
- When the reconnect grace expires or the session is closed, unacknowledged one-shots are dropped and reported as failed. Delivery is not guaranteed after the player is removed or the server process fails without graceful recovery.

The game-server dependency is pinned to a full commit in `Cargo.toml` (and resolved in `Cargo.lock`) rather than following a branch tip; Renovate proposes digest updates. `Cargo.toml` is authoritative for the accepted revision; the README's pinned-foundations list is a reader-facing mirror and must change in the same PR.

## Shared multiplayer protocol

Network topology must not change gameplay semantics.

A player input enters the core as a `PlayerCommand` containing:

- ARPG player identity;
- a monotonically increasing per-player sequence number;
- a typed game command decoded by the game protocol.

The sequence number is host/runtime metadata, not UI state. It is retained at the core boundary so dedicated-server replay/recovery and peer-hosted replay/divergence tooling can preserve the same input ordering evidence.

Snapshots are typed core values first and encoded only at the protocol boundary. The renderer consumes projected state and must not deserialize network packets as its gameplay model.

## Multiplayer setup service boundary

Browser peer setup consumes the upstream lobby client rather than forking its signaling/reconnect/ICE logic. The accepted setup-service revision is `cc5c20fabdac0fc9da4329a5be04d0f4e8eae383`.

The web shell vendors the accepted upstream browser modules under generated `web/src/vendor/multiplayer-setup-service/` with deterministic pin checks. It uses `DemoLobbySession` with `topology: "host"` to inherit resilient connections, STUN discovery, and participant-authenticated temporary TURN credential refresh. The deployment shared secret remains server-side.

Invite URLs may contain the public lobby identifier only. Participant capability tokens and TURN credentials must never be placed in invite URLs, logs, analytics, or committed configuration.

The setup service is not matchmaking, account storage, ranking, persistence, or a dedicated-server directory. Those concerns remain separate and should only be added when the product requires them.

## Trust boundaries

- Dedicated server: trusted gameplay authority; clients submit commands only.
- Peer-hosted co-op: host is gameplay authority for consistency, but is not trusted against cheating.
- Setup service: trusted only for authenticated short-lived lobby/signaling capabilities; never gameplay authority.
- Peers: untrusted sources of gameplay claims and optional content bytes.
- Asset/content bytes received from peers must be verified against an ARPG-trusted manifest before use.

## Dependency direction

```text
browser / desktop shell
        |
        +--> arpg-core <-------------------------+
        |       |                                |
        |       +--> rapier3d (physics)          |
        |                                        |
        +--> arpg-protocol                       |
        |                                        |
        +--> 3d-lab renderer                     |
        |                                        |
        +--> multiplayer-setup-service           | peer-hosted
                                                 |
arpg-game-server --> game-server --> arpg-core --+ dedicated
        |
        +--> arpg-protocol
```

`arpg-core` must not depend on `game-server`, WebRTC, WebTransport, Three.js, React, or browser APIs.

## Initial implementation slices

1. Establish the transport-neutral `AuthoritativeGame` and wire-protocol seams and prove the `game-server` adapter with deterministic tests.
2. Build the first real game command/snapshot schema as combat/player movement begins; version it explicitly and exercise identical fixtures through local and game-server paths.
3. Add the browser shell and pin the setup-service resilient lobby client for host-topology peer co-op.
4. Run the same ARPG core in Wasm for local/peer host and native Rust inside `game-server`; compare deterministic snapshot fingerprints for the same input stream.
5. Add reconnect/divergence acceptance before treating multiplayer as a product feature.

Do not add event sourcing, distributed read models, a message bus, or a generic game-engine framework merely because multiplayer exists. The simulation hot path stays direct and domain-shaped.

The [application adoption record](conventions-adoption.md) records the audit scope,
verification evidence, and remaining advisories.
