# Authoring gameplay content

Gameplay tuning lives in one typed, versioned bundle:
[`crates/arpg-core/content/base.json`](../crates/arpg-core/content/base.json), parsed and
validated by [`content.rs`](../crates/arpg-core/src/content.rs). Definitions *select*
supported behaviour; executable rules stay in Rust. Appearance and audio never live here.

## What the bundle holds (format 7)

| Section | Contents |
| --- | --- |
| `strikes` | Melee geometry: reach, frontal cone, target cap, blockable, guard cost. |
| `actions` | One definition per supported `ActionKind`: phase ticks, strike, damage ratio, stagger. |
| `combos` | Light/heavy transitions between striking actions and their input windows. |
| `counterWindowTicks` | How long a successful block keeps the counter open. |
| `monsters` | Enemy definitions: health, strike, phase ticks, damage, experience, pursuit speed, aggro/leash/switch/reacquire/separation, and the role parameters `retreatRange`, `projectile` and `shield` (see below). |
| `roomMonsters` | The monster definition of each combat room, in generation order (repeats when rooms outnumber entries). |
| `guard` | Raise ticks, guard points and regeneration, block reaction, guard-break ticks. |
| `bow` | Draw thresholds, arrow speed/damage range, lifetime, stagger and live-arrow cap. |
| `progression` | Experience per level and the base/per-level health and attack damage. |
| `loot` | Gold dropped by a defeated monster and held by a reward chest. |
| `targeting` | Target-lock acquisition range and the larger break range that keeps a held lock (stickiness). |

Ids are 1–64 ASCII characters, unique per collection and published to clients (strike
events, `MonsterSnapshot.definition`), so presentation can key appearance and audio by them.

## Enemy roles

A role is a combination of definition values on the one shared monster path: the same
pursuit, engagement, separation, wind-up/active/recovery machine and strike outcomes. There
is no per-role AI.

| Role | Built-in | What selects it |
| --- | --- | --- |
| Melee pressure | `monster.brute`, `monster.skirmisher` | `retreatRange: null`, `projectile: null`: closes to strike reach and strikes around itself. |
| Ranged | `monster.archer` | `retreatRange` keeps it in a band from that distance out to strike reach: a player who comes nearer makes it back off through its room grid, never nearer to the player and never into fixed bodies, before it shoots again. With nowhere to go it shoots from where it stands. `projectile` (`speed`, `lifetimeTicks`) makes the attack launch a shot along the arrow path at the target's position as the active phase opens; the monster's strike still decides reach, line of sight, block and guard cost, and `damage` the hurt. |
| Shielded defender | `monster.defender` | `shield: true`: it carries the player's shield guard, with the shared `guard` tuning (raise ticks, points, regeneration, block reaction, break ticks) and the same block rules (`guard_outcome`: the raised guard protects 60 degrees either side of where it faces against blockable strikes, each block spends the strike's `guardCost`, and a cost that reaches the remaining points breaks it). Engaged and free, it holds the guard, so it closes on its target with the shield up and faces that target; committing to its attack, a stagger or a guard break lowers it, and it rises again from the start once the monster is free. A short strike reach and slow pursuit leave a sword swing time to meet the shield while it closes; strike from behind or beside, or break the guard, to land damage. Player arrows are not blocked by it yet (#77). |
| Heavy | `monster.bruiser` | Long `windupTicks`, slow `pursuitSpeed`, high `damage` and a `guardCost` that breaks a full guard. The wind-up holds still and lands at the strike's reach around it, so the answer is to step out of it or interrupt it: any player hit staggers the monster and cancels the wind-up. |

Validation: `retreatRange` is at least the minimum monster reach and stays three navigation
cells inside the strike reach (room for the band); a projectile's speed is `16..=200` (slower integer velocities cannot hold a diagonal aim) and it
must fly at least the strike reach within its lifetime.

## Adding a definition

Example: a new enemy.

1. Add its strike to `strikes` (monster strikes must reach at least 105 units so pursuit can
   close to striking distance) and its entry to `monsters`.
2. Put its id into `roomMonsters` where it should spawn. Nothing else changes: transport,
   renderer and physics consume it through the generic monster path.
3. Keep source order irrelevant: `strikes`, `actions`, `combos` and `monsters` are sorted by
   id on load; only `roomMonsters` is authored order.

A new *kind* of behaviour (a new `ActionKind`, a new attack delivery) is a Rust change
first; content can then select it.

## Validating

Loading validates every reference and numeric bound and reports **all** violations with the
definition path, for example `monsters[1] monster.skirmisher: leashRange must exceed
aggroRange`. The built-in bundle is validated on first use, and the tests fail on any
violation:

```sh
cargo test -p arpg-core content
```

When a change touches a section whose previous values were Rust constants,
`base_content_reproduces_the_previous_tuning` pins the outcomes; update it only for a
deliberate tuning change.

## Exercising it deterministically

Gameplay is a pure function of seed, content and commands, so a definition is exercised by
a headless test on the real runtime. The enemy tests in
[`lib.rs`](../crates/arpg-core/src/lib.rs) (`a_skirmisher_*`,
`combat_rooms_spawn_the_monster_their_content_assigns`, the role tests such as
`an_archer_backs_off_around_a_wall_into_its_band_and_never_enters_fixed_bodies` and
`interrupting_a_bruiser_windup_cancels_the_slam_and_it_winds_up_anew`) show the pattern:
place the player and the generated monster, advance ticks, and assert timings, reach,
damage and rewards from the definition rather than from literals. For a recorded session,
replay a `Reproduction` (scenario, seed, players, commands) with `replay_reproduction`; the
browser training arena (`?scenario=training&seed=<u32>`) runs the same authority, and its
`fixture=ranged` and `fixture=heavy` arrange the first archer and bruiser rooms, and
`fixture=retreating` places that archer inside the light swing's reach, where it backs off
at once so a swing started on the first tick misses once its wind-up ends.

To try a definition or a tuning value without editing the bundle, the training panel's
**Arrangement and tuning** section applies workbench operations (`WorkbenchOperation` in
[`workbench.rs`](../crates/arpg-core/src/workbench.rs)) to the local authority: spawn a
content monster at an exact offset from the scenario room's centre, remove one, reset the
room to the scenario's arrangement, or set one of the counter, guard and bow values
(`TuningParameter`, named by its bundle path) to an exact number validated with the same
bounds as the bundle. They are not commands, so no peer or dedicated client can send them.
An exported reproduction records them among the commands (`operations`) and replays them;
a session they changed can no longer be saved.

## Counter window under remote latency

`counterWindowTicks` is a half-open command window `[usableFromTick, expiresAtTick)` judged
at the tick the authority **applies** a command. There is no lag compensation: a peer host
applies a guest command when the reliable channel delivers it, and the dedicated
`game-server` runtime when its reliable command lane delivers it (attacks are one-shots,
see [dedicated command delivery](ARCHITECTURE.md#dedicated-command-delivery)); commands
carry no tick, so neither can be
judged as of the past. A remote player sees the opportunity one-way delay *d* after it was
granted and its attack arrives *d* later again, so the practical reaction window is the
window minus the round trip. Both topologies publish a snapshot every tick, so snapshot
cadence adds no whole-tick loss. Ticks and command delivery are independent events (the
browser host's tick interval and its reliable-channel callback; the game-server tick loop and
its command lane), so a command that arrives at a tick boundary may be applied on either
adjacent tick: read each figure below as exact on the authority's tick grid and ±1 tick at
a boundary. The millisecond values are nominal, assuming steady 60 Hz scheduling; a throttled
or stalled peer host (for example a hidden tab) slows the simulation instead of catching up,
so the same ticks then span more wall-clock time. The rule itself is always judged in ticks.

Measured by [`counter_latency.rs`](../crates/arpg-game-server/tests/counter_latency.rs)
through the peer host and `MatchRuntime` with the real wire encoding, with the bundle's
30-tick window at 60 Hz (identical for both topologies; whole-tick round trips, ±1 tick at
a boundary as above):

| One-way delay | Round trip | Reaction window that still counters |
| --- | --- | --- |
| 0 ms | 0 ticks | 30 ticks (500 ms) |
| 25 ms | 3 ticks | 27 ticks (450 ms) |
| 50 ms | 6 ticks | 24 ticks (400 ms) |
| 75 ms | 9 ticks | 21 ticks (350 ms) |
| 100 ms | 12 ticks | 18 ticks (300 ms) |

A reaction one tick later arrives at or after `expiresAtTick` and starts an ordinary primary
attack. The same tests show that delay, reordering, duplication and reconnect never grant
extra time or a second counter: a counter command overtaken by a newer sequence is ignored
as stale, a duplicated one spends one counter only, and a reconnect within the grace period
neither moves `expiresAtTick` nor restores a spent opportunity (the runtime's sequence
watermarks survive it). Tune the window with the remote round trip in mind. Any future lag
compensation must stay authority-owned and bounded: it may never let a delayed, reordered
or replayed command extend a window or earn a second counter without a new block.

## Packaging and compatibility

The bundle is compiled into `arpg-core` (`include_str!`), so the Wasm build and the dedicated
server always ship the bundle they were built with. Its **content revision** is an FNV-1a
hash of the canonical bundle:

- snapshots publish it as `contentRevision`, and saves record it; a save from another
  revision is refused on load;
- a peer guest compares the host's revision, and a dedicated client the server's, with its
  own build and refuses to play across revisions with a message naming both;
- `formatVersion` (`CONTENT_FORMAT_VERSION`) changes only when the bundle's *shape* changes.
  It is independent of the save schema and the wire protocol versions, which change only
  when their own encodings do.
