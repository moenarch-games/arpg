# Architecture

`arpg` is the game-specific composition layer. It owns ARPG rules and content semantics while consuming reusable foundations for rendering, physics, assets, and multiplayer infrastructure.

## Authority map

- `arpg-core` owns deterministic gameplay state and rules: characters, monsters, stats, skills, damage, cooldowns, inventory, loot, AI decisions, progression, quests, dungeon state, and seeded game randomness.
- `physics-engine` owns collision, movement constraints, spatial queries, CCD, contacts, and other physical truth. ARPG-specific character-controller requirements should be implemented upstream there rather than approximated in the renderer.
- `3d-lab` owns reusable 3D geometry/camera/animation/asset models and browser rendering primitives. `arpg` owns scene composition and game presentation policy.
- `asset-tooling` owns reproducible asset acquisition/generation/processing evidence. Runtime code consumes accepted artifacts rather than generation machinery.
- `MOEL` is the preferred data format for schema-validated game content once the first content slice lands.
- `game-server` owns reusable server-authoritative match runtime concerns.
- `multiplayer-setup-service` owns peer rendezvous/signaling only.

Neither renderer nor network transport may become an alternate source of gameplay truth.

## Height and traversal topology

Real physical height and semantic walkable topology are separate but coordinated
authorities. `physics-engine` owns continuous physical truth; `arpg-core`
owns stable walkable regions, typed traversal connections, actor region/transit
state, and ability-specific cross-region rules.

Combat and interaction are region-local by default. Abilities may explicitly
opt into cross-region 3D range/line-of-sight checks. Authored and procedurally
generated levels must both provide topology matching their geometry; undeclared
physical crossings are invalid level content rather than runtime exceptions.

See [ADR 0002](adr/0002-height-traversal-topology.md) for the full contract.

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

This layer sits **above** reusable foundations such as `physics-engine`, `input-bindings`, and `3d-lab`. It should not be generalized into those repositories merely because multiple ARPG features use it. Weapons, skills, monsters, bosses, loot, and dungeon rules should compose the ARPG game-loop contracts rather than each introducing private movement, timing, targeting, hit-reaction, or pickup systems.

The presentation boundary is intentionally one-way: animation, rendering, camera, audio, HUD, particles, and local visual emphasis may react to authoritative state/events, interpolate them, and decorate them. They may not decide that an attack hit, a target was valid, an action completed, a character was displaced, or an item was picked up.

Likewise, animation timing must not silently become gameplay authority. Gameplay timings live in deterministic core state; animation consumes phase/cue timing and visually represents it.

The first acceptance environment for this foundation is a deterministic combat training arena with reproducible movement, attack, impact, interruption, targeting, projectile, knockback, death, and pickup scenarios. See [ROADMAP.md](ROADMAP.md).


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
        |       +--> physics-engine              |
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
