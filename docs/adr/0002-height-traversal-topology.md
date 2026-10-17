# ADR 0002: Height, walkable topology, and traversal

Status: Accepted
Date: 2026-10-08

## Context

ARPG needs real vertical traversal: slopes, ramps, ledges, bridges, balconies,
stacked floors, jumps, drops, and procedural dungeon layouts. At the same time,
ordinary combat should remain readable and mostly planar rather than making
every attack, target query, and encounter rule an unconstrained 3D geometry
problem.

Procedural generation also needs a deterministic representation that
navigation, encounters, targeting, saves, and the minimap can share. Inferring
all gameplay structure from rendered or collision geometry at runtime would
make those systems harder to validate and reproduce.

## Decision

### Physical height is real

The physics backend (Rapier until `physics-engine` is finished, Atlas 2.3.1) owns
continuous physical height, support/contact, collision, gravity, and motion
resolution. Height is not a render-only offset.

A walkable region is a connected gameplay surface, not a flat plane. One region
may contain slopes, ramps, stairs, and uneven ground.

Vertically overlapping walkable regions are allowed. A bridge may cross a path,
a balcony may sit above a room, and dungeon floors may overlap in horizontal
coordinates.

### Levels carry explicit walkable topology

Every playable level has stable walkable-region identities plus explicit
connections between regions.

Authored levels author this topology. Procedural generators emit the same
topology together with their geometry; generation does not stop at producing
meshes or colliders.

Connections are typed according to the traversal they permit, such as:

- walk/ramp;
- jump;
- one-way drop;
- door or other gated passage.

The topology is authoritative for gameplay connectivity. If physical geometry
permits an undeclared crossing between regions, that level is invalid and
should fail generation/content validation. Do not repair the mismatch with
invisible ARPG-local blockers.

### Region membership and transit are explicit

An actor supported on a walkable surface belongs to a stable region.

Ordinary airborne movement that remains within one region does not erase that
region membership. When an actor traverses a typed connection between distinct
regions, such as a cross-region jump or drop, it enters an explicit transit
state associated with that connection until the traversal resolves.

Transit is not invulnerability and does not itself cancel actions. Systems that
care about an actor in transit must handle that state explicitly.

### Combat is topology-first by default

Ordinary combat, targeting, and interaction are region-local by default. Exact
vertical position is therefore primarily traversal truth rather than a
universal extra combat axis.

Specific abilities may opt into cross-region behavior. For example, an arrow
may use real 3D range and line-of-sight queries across regions. That permission
belongs to the ability/gameplay rule; physical visibility alone does not make
all attacks cross regions.

This keeps ordinary melee and encounter rules predictable while still allowing
deliberate vertical combat mechanics.

## Authority boundaries

- The physics backend (Rapier for now) owns physical height, collision, support,
  contacts, and spatial queries.
- `arpg-core` owns walkable-region topology, typed traversal semantics, actor
  region/transit state, and ability-specific cross-region eligibility.
- Level content or procedural generation produces geometry and the matching
  topology.
- Rendering, animation, camera, HUD, and minimap project this state; they do not
  infer or correct authoritative connectivity.

## Consequences

- Navigation, encounter gating, targeting, interaction, saves, and minimap
  projection can share one deterministic topology.
- Procedural generation must validate geometry/topology consistency.
- Continuous terrain does not fragment into artificial height bands.
- Bridges, balconies, stacked floors, and other vertical overlap remain
  possible without turning all combat into general 3D combat.
- Cross-region combat is explicit and testable per mechanic.
- Runtime code does not infer semantic regions from render meshes.

## Non-goals

This ADR does not define climbing, swimming, flight, moving-platform semantics,
or a generic navigation-mesh inference system. Those require concrete gameplay
needs before extending the contract.
