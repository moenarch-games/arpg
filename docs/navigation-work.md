# Enemy navigation work

Pursuit (#65) rasterizes a room into 20-unit cells and plans with A* to a cell within
strike reach of the target. Its first version rebuilt the room grid and planned anew
for every pursuing monster on every tick. Since #110 the plans are retained:

- **Room grids** are kept while the fixed bodies are unchanged. Every tick that someone
  pursues, the fixed-body footprints are compared with the ones the grids came from. A
  door that locks or unlocks, or a fixed body added or removed, drops all grids and
  fields.
- **Target fields** are kept per room and *target cell* while a pursuer still chases that
  cell. A field holds exact distances to its *strict goals*: free cells whose centre is
  within reach of every point of the target's cell, with a clear line to all of it. A
  separating-axis test of the line fan against each fixed footprint, grown by one unit,
  checks the lines.
- **Routes** are the *canonical descent* of a field. From each cell, the route takes the
  first move in the fixed neighbour order that is exactly one step nearer to a strict
  goal. A search is a multi-source A* from the goals toward the body's cell. It closes
  every cell on any shortest route, so the route from any later cell on that route is its
  suffix, and a body walking its route never searches again.
- **Repaths** happen only when the target enters another cell, the fixed bodies change,
  or a body is pushed off every route its field knows. A per-tick exact plan, the #65
  planner, is used in three cases. A body already on a strict goal finishes its approach
  with it, which costs no expansion and one physics query. It is also used when no strict
  goal is reachable, and when the retained route cannot be shown to cost at most
  `ROUTE_SLACK` (two straight steps) more than the exact plan. The bound comes from
  distances to the *loose goals*: every free cell within reach of some point of the
  target's cell. They include every exact goal, so their distance bounds the exact plan's
  cost from below. When the octile cost to the nearest loose goal already proves the
  bound, nothing is searched.

## Determinism

A route is a pure function of the fixed bodies, the target's cell and the body's cell.
It does not depend on what the cache holds. The cache only decides how much has to be
searched, so it is never saved. After a load it starts empty and gives the same routes.
The fallback decisions are pure functions of the same inputs, so the
pursue-or-stand decision is exactly the per-tick planner's: a strict goal is an exact
goal for every target position in its cell, and an unreachable loose goal set means the
exact planner has no route either. A retained route reaches an exact goal for the
current target position and costs at most `ROUTE_SLACK` more than the exact plan.

The checks are:

- `navigation::tests::retained_routes_match_fresh_fields_and_the_per_tick_reference`
  covers 120 random rooms with 3 to 8 obstacles and 40 steps each. Bodies walk their
  routes or are knocked off them, and targets walk across cells. Each retained route
  equals the route from a freshly built field. Reachability agrees with the exact
  planner. Every retained route ends at an exact goal and exceeds the exact cost by at
  most 20, which is reached. Retained work there is 205,578 expansions, including exact
  plans, against 295,502 for planning every step.
- `retained_navigation_never_changes_the_game` covers 16 random strike-arena chases with
  pillars, up to three pursuers, a walking target, and a pillar that appears and later
  vanishes. Each one gives identical snapshots on every tick with the cache kept and
  with it dropped every tick.
- `retained_and_per_tick_pursuers_set_out_alike`: in 60 random chases, each pursuer
  moves on its first pursuit tick exactly when the per-tick reference moves it.
- `strict_goal_cells_have_a_clear_physics_strike_line_to_their_whole_target_cell` checks
  the line test against the physics ray.
- `a_chase_continues_identically_after_a_save_that_drops_the_navigation_cache` runs 95
  ticks after a save taken mid-chase while fields are held. Every snapshot equals the
  uninterrupted run's.

## Engagement and separation (#109)

The engagement policy decides *whom* a monster chases and whether it chases at all, and
separation bends the velocity toward the next route point. Neither touches the cache:
the target position a field is built for is still a player's position, and separation is
a pure function of the bodies' positions before the step. So the determinism argument
above holds unchanged, and `retained_navigation_never_changes_the_game` runs the chases
with engagement and separation on. A body that separation pushes off its route is a body
pushed off its route: the field repaths it like any other. A returning monster plans
with the exact planner every tick (no line test, goal within one cell of its post);
returning is rare and short, so it is not retained.

## Enemy roles (#108)

A ranged monster that a player comes too close to backs off with an exact plan every
tick: a single-source A* into its band (one cell beyond its retreat range out to one
cell inside reach, with the physics strike line), expanding only cells no nearer to the
target than its own. Like returning it is short-lived and never retained, so the cache
and its determinism argument are unchanged; the room grid it reads is the same pure
function of the fixed bodies. These plans count as `retreat_plans`, not repaths, and their
line tests count as physics queries. `retained_navigation_never_changes_the_game` and
`retained_and_per_tick_pursuers_set_out_alike` now mix brutes, archers and bruisers in their
random chases. The heavy role only changes content values (slower pursuit, a longer
wind-up), so it plans exactly like any pursuer.

## Counts

`seeded_workloads::chase_navigation_work_against_the_per_tick_reference` replays the
chase workload for 120 ticks per seed. The five generated monsters engage on the first
tick (#109) and chase from the second; two of them are an archer, which holds its band
and shoots instead of closing in, and a slow bruiser (#108). The counts are deterministic.
A repath is a field search or an exact plan. They were measured again after the switch to
Rapier physics (#175), which moves the bodies along the routes. No retreat happened in
this chase.

| Seed | Planner | Expansions | Peak expansions in one tick | Repaths | Physics queries | Grid builds |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 42 | per tick (#65) | 92,299 | 1,045 | 534 | 534 | 119 |
| 42 | retained | 12,649 | 771 | 99 | 0 | 1 |
| 3735928559 | per tick (#65) | 63,952 | 790 | 507 | 507 | 119 |
| 3735928559 | retained | 9,060 | 605 | 95 | 0 | 1 |
| 2753562902 | per tick (#65) | 92,876 | 1,068 | 534 | 534 | 119 |
| 2753562902 | retained | 14,126 | 798 | 109 | 0 | 1 |

The retained planner needs about a seventh of the expansions and a fifth of the repaths.
It made no physics query in this chase, because no pursuer reached a strict goal and
every route was within the bound. It rasterizes the room once instead of on every tick.
The peak tick is not much lower. When the target enters a new cell, all five pursuers
search on that tick, and closing every shortest route costs more than one A* path. The
test asserts that expansions and physics queries are at most half of the reference's. It
also asserts that repaths are fewer than half of the pursuit ticks, and that the retained
trace equals the trace with the cache dropped every tick. The two planners give different
traces, because their shortest routes may break ties differently.

The same test prints advisory whole-tick timings. The table shows medians of three
release runs on `x86_64-unknown-linux-gnu` with rustc 1.98.1, dated 2026-10-17 (Rapier
physics, #175), in milliseconds for 120 ticks. *Without cache* is the retained planner
with its cache dropped every tick.

| Seed | Per tick (#65) | Without cache | Retained |
| --- | ---: | ---: | ---: |
| 42 | 38.64 | 23.19 | 7.27 |
| 3735928559 | 29.71 | 19.60 | 5.67 |
| 2753562902 | 38.18 | 22.69 | 6.43 |

Both uncached planners rasterize the room on every tick. The test does not separate
that cost from the searches.

## Reproduce

```sh
cargo test --release --locked -p arpg-core chase_navigation -- --nocapture
```
