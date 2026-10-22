# ARPG — Full Game Roadmap

This document is the long-term game vision. The near-term execution order and its issues live in the content-readiness roadmap [#50](https://github.com/moritzbrantner/arpg/issues/50); the delivered/in-flight/planned status table is in the [README](../README.md#capability-status).

## Vision

Build an isometric action RPG whose strengths are:

- responsive, weighty combat;
- enemies that reward positioning, timing, target priority, and interruption;
- builds that substantially alter how the game plays;
- procedural but authored-feeling adventures;
- strong animation, visual, audio, and impact feedback;
- seamless solo and co-op play using the same deterministic simulation;
- systemic mechanics that let a relatively small amount of authored content produce substantial variety.

The architecture remains a means to this end. New infrastructure should normally require a concrete improvement to the playable game.

## Core experience

The game needs to work at several timescales.

**Second-to-second:** move, aim, commit to an attack, read an enemy, dodge or interrupt, land a satisfying hit, and reposition.

**Minute-to-minute:** fight an interesting group, obtain loot, make a build decision, discover something, and face a new tactical situation.

**Run-to-run:** enter a reproducible but varied dungeon or region, make choices that alter the run, fight elites and events, defeat a boss, and return meaningfully stronger or with new possibilities.

**Character-to-character:** experiment with fundamentally different weapons, skills, equipment interactions, and play styles.

Everything on this roadmap should strengthen at least one of these loops.

---

## Stage 1 — Make fighting one monster excellent

This is the highest priority. The game should already be enjoyable in the combat training arena with one weapon and one enemy before broad content expansion.

### 1.1 Combat training arena

Build a first-class deterministic Pages scenario for mechanical iteration.

Initial controls:

- deterministic seed display and restart;
- pause and resume;
- single simulation step while paused;
- explicit simulation speed choices;
- live tick and combat-state diagnostics.

Follow-up controls:

- spawn and remove enemies;
- enemy archetype and count;
- invulnerable target dummy;
- exact player movement and attack parameters;
- collision visualization;
- action-phase visualization;
- target visualization;
- combat event timeline;
- projectile counts.

The arena is a development and acceptance surface, not a second gameplay implementation. It runs the same `arpg-core` authority as normal local play.

### 1.2 Movement

Finish one reusable ARPG locomotion model:

- responsive acceleration and stopping;
- reliable direction changes;
- diagonal and analog normalization;
- good collision sliding and corner handling through the physics backend (Rapier for now);
- explicit movement/action cancellation rules;
- controller, keyboard, and touch intent projected into the same gameplay commands;
- movement skills through the same authoritative movement boundary.

Integrate shared humanoid work from `3d-lab`:

- proper skeleton;
- locomotion animation;
- procedural leg IK;
- foot planting;
- slope adaptation;
- animation blending;
- directional attack animation;
- hit reactions;
- death animation.

Animation follows gameplay truth rather than defining it.

### 1.3 Combat actions

Continue the existing action lifecycle as the permanent combat foundation:

1. intent accepted;
2. wind-up / commitment;
3. active effect;
4. authoritative hit or effect;
5. recovery;
6. permitted interruption or cancellation.

Use this foundation for:

- light attack;
- heavy attack;
- defensive action;
- movement/dodge action;
- targeted skill;
- directional skill;
- ground-targeted skill;
- projectile;
- channel;
- interrupt;
- stagger;
- knockback;
- block/parry;
- status effects.

Do not create a private timing model for each skill.

### 1.4 Targeting and interaction

Use one game-specific targeting/interaction boundary for attacks, skills, enemies, loot, doors, and later NPC interactions.

It should cover:

- explicit target selection where appropriate;
- direction and ground-position actions;
- range validation;
- target stickiness and loss;
- controller/touch assistance without changing simulation rules;
- overlap priority;
- deterministic fallback when a target becomes invalid.

React, the renderer, and input adapters must not invent ARPG targeting semantics.

### 1.5 Impact

Make authoritative combat outcomes legible through presentation cues:

- weapon trails;
- contact flashes;
- particles;
- decals;
- restrained presentation-only hit stop;
- camera impulse;
- enemy recoil;
- knockback;
- directional audio;
- distinct hit, block, critical, and kill sounds;
- damage numbers where useful;
- clear death feedback.

The presentation layer reacts to authoritative cues. It never decides whether gameplay happened.

### 1.6 Enemy reaction foundation

A basic monster should already be enjoyable to fight.

Support reusable:

- approach and spacing;
- attack commitment;
- hurt/stagger response;
- knockback/displacement;
- death;
- recovery and re-engagement.

### Stage 1 milestone — Combat game

A player can enter the arena and fighting is genuinely enjoyable.

Requires movement, animation, multiple attacks, enemy pursuit, telegraphs, hit reactions, audio/VFX, and several enemy roles.

---

## Stage 2 — Make combat tactically interesting

### Enemy movement and navigation

Add:

- pursuit;
- navigation around walls;
- spacing;
- retreat;
- strafing;
- preferred range maintenance;
- obstacle avoidance;
- formation pressure;
- separation between monsters.

Physical movement stays in the physics backend (Rapier for now); ARPG owns tactical decisions.

### Enemy roles

Build a small number of genuinely distinct roles before producing many enemy models:

- melee pressure;
- heavy/bruiser;
- ranged attacker;
- flanker;
- shielded defender;
- support/healer;
- summoner;
- artillery;
- crowd controller.

Mixed groups should create target-priority and positioning decisions.

### Telegraphs

Share authoritative intent through reusable telegraph data:

- directional cones;
- lines;
- ground areas;
- charges;
- projectile trajectories;
- interruptible wind-ups;
- persistent hazards.

### Elite modifiers

Prefer behavioral modifiers to percentage inflation:

- teleporting;
- splitting;
- reviving allies;
- projectile interception;
- hazard creation;
- ally shielding;
- retaliation;
- summoning;
- pursuit after being wounded;
- death effects.

Modifiers should compose with ordinary enemy archetypes.

---

## Stage 3 — Establish real character identity

### Weapons

Start with a compact mechanical vocabulary:

- sword / fast melee;
- hammer / heavy stagger;
- spear / reach and space control;
- bow / projectile play;
- shield / defensive timing;
- magical focus / skill-oriented ranged play.

Weapon identity changes animation, reach, timing, positioning, and tactics—not merely DPS.

### Skills

Introduce a small loadout with:

- basic action;
- several active skills;
- movement/utility skill;
- defensive option;
- passive/modifier choices.

Support composable transformations:

- multishot;
- pierce;
- chain;
- bounce;
- return;
- orbit;
- explosion;
- altered geometry;
- delayed detonation;
- damage/status conversion.

A modifier should normally transform an existing mechanic rather than fork a complete skill.

### Resources

Use a small resource vocabulary such as health plus one character/skill resource. Do not add multiple resource bars without a concrete gameplay reason.

---

## Stage 4 — Loot and buildcraft

The existing kill → drop → pickup loop becomes a complete ARPG reward loop.

### Inventory and equipment

Add authoritative:

- equipment slots;
- inventory;
- item comparison;
- equipping;
- dropping;
- item identity;
- item provenance.

Controller and touch navigation are designed with mouse interaction, not added afterward.

### Itemization

Build item meaning in this order:

1. base item identity;
2. mechanically useful affixes;
3. rarity/affix combinations;
4. unique items;
5. transformative items.

Examples:

- heavy attacks create a shockwave;
- perfect blocks reset a skill;
- arrows return;
- knockback becomes bonus damage;
- movement leaves damaging terrain;
- projectiles split after their first hit;
- frozen enemies shatter into damaging fragments.

Avoid building hundreds of small percentage modifiers before mechanically transformative items exist.

### Crafting

Prefer a small set of meaningful transformations:

- reroll;
- replace;
- extract;
- transfer;
- upgrade;
- fracture;
- corrupt or sacrifice with risk.

### Stage 4 milestone — Build game

A player can create substantially different play styles using weapons, skills, modifiers, equipment, loot, and crafting choices.

---

## Stage 5 — Build the first complete adventure

Create a roughly 20–30 minute complete run containing:

- entry area;
- several connected dungeon sections;
- ordinary packs;
- elite encounters;
- environmental hazards;
- optional encounter;
- treasure/reward room;
- mini-boss or major event;
- final boss;
- meaningful loot;
- character progression during the run;
- return/completion state.

It does not need huge content breadth. It needs a beginning, escalation, climax, and reward.

### Stage 5 milestone — Complete run

This is the first point where ARPG should be treated as a small game rather than primarily a technology demonstration.

---

## Stage 6 — Boss foundation

Do not implement a separate boss engine.

Compose bosses from:

- the normal action lifecycle;
- movement policies;
- reusable abilities;
- phases;
- arena mechanics;
- summons;
- environmental hazards;
- vulnerability windows;
- triggered transitions.

Develop several mechanically different boss patterns:

- duel;
- positioning/area-control boss;
- summoner;
- mobile pursuit boss;
- multi-phase encounter.

A successful boss mechanic should usually become reusable content vocabulary afterward.

---

## Stage 7 — Dungeon and encounter system

### Dungeon pieces

Support authored procedural pieces:

- rooms;
- corridors;
- arenas;
- elevation;
- doors;
- traps;
- shrines;
- environmental obstacles;
- destructibles;
- secret and optional spaces.

Prefer composing authored pieces to arbitrary procedural noise.

### Height and traversal topology

Dungeon generation may create real vertical overlap: bridges, balconies,
stacked floors, slopes, jumps, and drops. Generated and authored layouts both
produce stable walkable regions plus typed traversal connections alongside
their geometry. Regions may contain continuous height variation.

Gameplay treats that topology as authoritative connectivity. Ordinary combat is
region-local; mechanics such as projectiles may explicitly opt into
cross-region 3D checks. See [ADR 0002](adr/0002-height-traversal-topology.md).

### Encounter definitions

Make encounter composition reusable:

- enemy groups;
- spawn rules;
- reinforcement triggers;
- environmental elements;
- objectives;
- rewards;
- difficulty budget.

### Objectives

Support reusable contracts for:

- extermination;
- boss hunt;
- survival;
- defense;
- rescue;
- portal destruction;
- escape;
- control point;
- escort.

Geometry, enemy composition, objectives, and modifiers should be independently composable.

---

## Stage 8 — Visual identity

Make this a major workstream once the core mechanics are stable.

Use `3d-lab` and `asset-tooling` rather than private ARPG rendering or asset pipelines.

Develop:

- proper humanoid characters;
- equipped weapon rendering;
- armor/equipment visualization where viable;
- multiple monster silhouettes;
- coherent dungeon kits;
- lighting;
- shadows;
- environmental effects;
- particles;
- decals;
- destruction;
- spell effects;
- polished animation transitions.

Prioritize silhouette and combat readability over raw graphical detail.

The camera may react to encounter scale, occlusion, boss framing, indoor/outdoor spaces, and co-op separation, but remains presentation policy.

---

## Stage 9 — Audio

Drive audio from authoritative gameplay cues.

Add layers for:

- footsteps;
- weapon movement;
- material-sensitive impacts;
- monster vocalization;
- skills;
- environment;
- loot;
- UI;
- music;
- boss transitions.

Music may react to encounter state but never own encounter state.

---

## Stage 10 — World structure

Once the dungeon loop works, give runs context.

Build a compact structure with:

- safe hub;
- dungeon/region selection;
- NPC interaction;
- quests/objectives;
- progression gates;
- discovered locations;
- roaming encounters;
- world events.

Potential events:

- invasion;
- caravan;
- ritual;
- roaming boss;
- siege;
- corrupted area;
- rare treasure encounter.

Do not build a giant empty open world merely to increase map size.

---

## Stage 11 — Character progression

Build progression around unlocking possibilities rather than only increasing numbers.

Support:

- levels;
- mechanically meaningful attributes;
- skill unlocks;
- skill modifications;
- equipment progression;
- permanent unlocks;
- build respec;
- multiple saved characters.

The character-selection flow and versioned savestate work are natural foundations here.

---

## Stage 12 — Co-op as an actual game mode

The networking foundation already exists. Concentrate on player experience:

- simple host/join flow;
- drop-in/drop-out;
- reconnect;
- party indicators;
- multiplayer enemy behavior;
- revive/downed mechanics if appropriate;
- individual/shared loot policy;
- player-to-player interaction;
- encounter scaling;
- bosses designed for several players.

`game-server` remains runtime/session authority. Gameplay remains in the same `arpg-core` simulation.

Prediction and interpolation may hide latency but may not replace authoritative results.

---

## Stage 13 — Replayability and endgame

Do this only after one complete adventure is enjoyable.

Create reusable high-level challenge generation from existing systems:

- increasingly difficult seeded expeditions;
- dungeon modifiers;
- elite modifiers;
- boss variants;
- optional challenge objectives;
- risk/reward choices;
- rare encounters;
- high-value crafting resources;
- build-defining rewards.

Expose deterministic challenge seeds so interesting runs can be shared and reproduced.

A good endgame remixes the game's strongest systems instead of introducing a second game.

---

## Stage 14 — Authoring and agent-driven content production

Once runtime schemas stabilize, move declarative content into MOEL where appropriate:

- items;
- skills;
- enemies;
- enemy modifiers;
- encounters;
- bosses;
- dungeon pieces;
- dungeon modifiers;
- loot tables.

Executable hot-path behavior remains code unless a concrete need justifies something more dynamic.

Build focused editors:

- skill editor;
- item editor;
- enemy editor;
- encounter editor;
- dungeon editor.

Editors use exact numeric inputs, useful visualization, and direct playable previews.

The long-term content advantage should be that agents can generate and validate substantial amounts of content against stable mechanics without repeatedly changing the engine.

---

## Stage 15 — Game UX

Replace development-oriented UI with a compact game interface.

Gameplay HUD:

- health/resources;
- skills and cooldowns;
- important statuses;
- contextual interaction;
- current objective;
- useful party state.

Out-of-combat surfaces:

- character selection;
- inventory/equipment;
- skills/build;
- map;
- quests/objectives;
- crafting;
- settings.

Avoid generic hero copy, redundant metrics, decorative KPI cards, and explanatory panels that displace actual game actions.

Keyboard/mouse, controller, and touch are first-class input modes. Mobile must not merely display a desktop hotkey UI.

---

## Stage 16 — Accessibility and settings

Support useful control over:

- key/controller/touch bindings;
- aim/target assistance;
- camera sensitivity;
- screen shake;
- hit flashes;
- damage numbers;
- audio channels;
- graphics quality;
- text size;
- UI scale;
- color-dependent combat indicators;
- subtitles where appropriate.

Settings remain presentation/input configuration, not gameplay truth.

---

## Stage 17 — Performance and scale

Optimize representative gameplay scenarios rather than isolated microbenchmarks.

Maintain scenarios for:

- dense melee;
- large enemy pack;
- projectile swarm;
- status-heavy combat;
- destructible environment;
- boss + adds;
- four-player combat;
- large dungeon;
- loot-heavy scene.

Measure simulation, physics, rendering, animation, networking, and UI separately.

Use optimized shared foundations only where representative benchmarks show they help. Avoid copying optimized algorithms into ARPG.

Prioritize stable frame pacing over average FPS.

---

## Stage 18 — Polish

After the complete loop works, repeatedly play and tune:

- attack timing;
- movement responsiveness;
- animation transitions;
- enemy telegraphs;
- hit feedback;
- camera;
- sound;
- lighting;
- loot presentation;
- UI interaction;
- onboarding;
- difficulty curves;
- encounter pacing.

Polish is repeated play and adjustment, not a final feature dump.

---

## Release milestones

### Milestone A — Combat game

Movement, animations, multiple attacks, enemy pursuit, telegraphs, hit reactions, audio/VFX, and several enemy roles make the combat arena genuinely enjoyable.

### Milestone B — Build game

Weapons, skills, modifiers, inventory, equipment, loot, and crafting produce substantially different builds.

### Milestone C — Complete run

A satisfying 20–30 minute adventure works from entry through boss and reward.

### Milestone D — Replayable game

Multiple layouts, encounters, bosses, builds, modifiers, and progression make repeated runs meaningfully different.

### Milestone E — Co-op game

The same experience works cleanly for several players with reconnect and persistence.

### Milestone F — Content platform

MOEL schemas and focused editors make enemies, equipment, skills, encounters, and dungeons inexpensive to produce and validate.

### Milestone G — 1.0-quality game

Several polished build directions, coherent adventure structure, substantial replayability, endgame, strong controls, stable saves, co-op, good performance, and a coherent audiovisual identity.

---

## Architectural guardrails

Keep the existing authority map:

| Concern | Authority |
| --- | --- |
| ARPG gameplay rules and content semantics | `arpg-core` |
| Collision, movement constraints, contacts, spatial queries | Rapier until `physics-engine` is finished |
| Reusable rendering, camera, animation, and asset models | `3d-lab` |
| Reproducible asset processing | `asset-tooling` |
| Reusable binding/runtime input behavior | `input-bindings` |
| Dedicated runtime/session machinery | `game-server` |
| Peer rendezvous/signaling | `multiplayer-setup-service` |
| Declarative game data where appropriate | MOEL |
| Browser composition/HUD/settings | `web` |

Do not introduce separate boss, loot, skill, multiplayer, targeting, movement, or animation authorities.

Do not adopt a generic scripting engine, message bus, event-sourcing system, or generalized game framework merely because the product is becoming larger. Add abstractions after multiple real gameplay slices demonstrate the same requirement.

## Cross-cutting acceptance

Maintain deterministic scenarios for:

- empty-room movement;
- repeated stop/start/reverse movement;
- attack while approaching range;
- target dies during wind-up;
- target leaves range before the active window;
- repeated melee hit/recovery;
- projectile release and impact;
- knockback into a wall;
- competing target selection;
- kill → drop → pickup;
- interrupted action;
- local vs dedicated execution equivalence.

Correctness tests assert state/event semantics. Performance evidence measures representative deterministic work separately from advisory wall-clock timing.

---

## Immediate implementation sequence

The execution order toward **Milestone A** and the content-ready foundation is maintained in [#50](https://github.com/moritzbrantner/arpg/issues/50) and its linked issues rather than duplicated here. Physics solver migration is owned by [#48](https://github.com/moritzbrantner/arpg/issues/48).

Progress should be judged primarily by how much better the next five minutes of play become, not by the number of architectural capabilities added.
