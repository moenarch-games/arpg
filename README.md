# arpg

Architecture-first action RPG foundation inspired by the systemic shape of Diablo II, built as an original implementation.

The MVP is intentionally an **integration proof**: gameplay truth lives in one deterministic Rust simulation and the renderer, browser UI, peer networking, and dedicated-server runtime consume that authority rather than replacing it.

## Playable MVP

The browser slice currently proves:

- WASD top-down movement with diagonal normalization;
- a procedurally partitioned, seeded dungeon whose walls and doorways are resolved by Rapier physics inside the Rust simulation;
- an isometric follow camera and simple 3D scene rendered through `3d-lab`;
- nearby primary attacks against deterministic monster state;
- deterministic character progression exposed through authoritative snapshots;
- a deterministic combat training arena available through the character screen or `?scenario=training&seed=<u32>`, with pause, single-step, explicit simulation speed, seed restart, and live combat diagnostics;
- a settings menu with graphics controls and the reusable `input-bindings` keybinding editor;
- local Rust/Wasm play;
- versioned Rust-authoritative savestates with browser quick-save/load and portable JSON import/export;
- one fresh 32-bit run seed per new authority, carried in authoritative snapshots so multiplayer and replay evidence identify the exact generated dungeon;
- host-authoritative peer co-op using `multiplayer-setup-service` for rendezvous and direct WebRTC data channels for commands/snapshots;
- dedicated online play over the shared `game-server` WebTransport protocol, with browser-side framing and snapshot-hash verification but no duplicated gameplay rules;
- the same `ArpgGame` exercised through `game-server::MatchRuntime`, with an equivalence test requiring the dedicated-runtime and direct-local paths to produce the same typed snapshot for the same input stream;
- a runnable TLS/WebTransport dedicated server using the shared `game-server` transport, reconnect, recovery, tick, and shutdown machinery;
- GitHub Pages build/deployment for browser acceptance.

This is deliberately not yet a content-complete ARPG. The slice exists to prove the foundations before inventory, loot, skills, richer AI, assets, character persistence, or larger levels are built on top.

## Capability status

See the [Decision Atlas](docs/decisions/atlas.md) for the permanent owner-approved product tree and narrowly scoped implementation-evidence checkboxes; this capability table is a technical inventory and does not establish the complete user-visible crypt experience.

The near-term execution order is the content-readiness roadmap [#50](https://github.com/moritzbrantner/arpg/issues/50); [`docs/ROADMAP.md`](docs/ROADMAP.md) keeps the long-term game vision.

| Capability | Status | Implementation | Exercised by |
| --- | --- | --- | --- |
| Startup and character selection | Implemented | [`web/src/main.tsx`](web/src/main.tsx), [`character-selection.js`](web/src/character-selection.js) | `character-selection.test.js`; browser specs in [`web/tests/browser`](web/tests/browser) |
| Movement, locomotion feel and wall sliding | Implemented | [`arpg-core`](crates/arpg-core/src/lib.rs) through Rapier's character controller ([`physics.rs`](crates/arpg-core/src/physics.rs)) | `locomotion_accelerates_brakes_and_reverses_over_bounded_ticks`, `diagonal_control_slides_along_outer_wall`, `wasd_style_movement_is_driven_through_physics`, `physics::tests` |
| Combat phases, facing, telegraphs, stagger, defeat | Implemented | [`arpg-core`](crates/arpg-core/src/lib.rs) | `attacks_have_authoritative_commitment_active_and_recovery_phases`, `attack_targeting_respects_committed_facing`, `monster_attack_is_telegraphed_before_damage`, `player_stagger_interrupts_monster_windup`; `app.spec.js`, `hotkeys.spec.js` |
| Enemy pursuit, engagement and separation | Implemented | [`arpg-core`](crates/arpg-core/src/lib.rs) (aggro, leash and target-switch tunables in [`base.json`](crates/arpg-core/content/base.json)); behaviour published in snapshots | `two_competing_targets_switch_only_beyond_the_hysteresis_margin`, `a_crowd_surrounds_its_target_without_overlapping_bodies`, `a_lost_target_is_searched_for_then_another_is_reacquired`, `an_interrupted_windup_resumes_pursuit_of_the_same_target`, `engagement_matches_between_local_and_dedicated_execution` |
| Enemy roles: melee, ranged spacing, heavy telegraph | Implemented | Role parameters in [`base.json`](crates/arpg-core/content/base.json) on the shared pursuit and strike path ([roles](docs/CONTENT.md#enemy-roles)); monster shots on the arrow path; `ranged` and `heavy` training fixtures | `the_default_dungeon_places_melee_ranged_and_heavy_roles_that_differ_in_play`, `an_archer_backs_off_around_a_wall_into_its_band_and_never_enters_fixed_bodies`, `interrupting_a_bruiser_windup_cancels_the_slam_and_it_winds_up_anew`, `saves_mid_shot_mid_retreat_and_mid_telegraph_continue_like_the_uninterrupted_game`, `engagement_matches_between_local_and_dedicated_execution` |
| Content definitions and revision checks | Implemented | [`base.json`](crates/arpg-core/content/base.json) (combat, guard, bow, progression, loot, four enemies); [authoring guide](docs/CONTENT.md); revision checks in [`peer-session.js`](web/src/peer-session.js) and [`game-client-runtime.ts`](web/src/game-client-runtime.ts) | `base_content_reproduces_the_previous_tuning`, `combat_rooms_spawn_the_monster_their_content_assigns`, `a_skirmisher_attacks_with_its_own_timing_reach_and_damage`, `saves_from_another_content_revision_are_rejected`, `peer-session.test.js`, `game-client-runtime.test.ts` |
| Aim intent and optional target lock | Implemented: semantic aim separate from movement, lock/cycle/clear with sticky break range; mouse aim is opt-in in **Settings → Controls** (`T` lock/cycle, `G` clear) | [`arpg-core`](crates/arpg-core/src/lib.rs); [`aim-input.js`](web/src/aim-input.js) | `exact_aim_directs_a_strike_between_the_eight_facings`, `target_lock_cycles_nearest_first_with_identity_ties_and_line_of_sight`, `losing_the_lock_never_suppresses_an_empty_swing`, `a_released_arrow_is_never_redirected_by_the_lock`, `aim_and_lock_survive_save_and_continue_identically`, `aim_and_target_lock_match_between_local_and_dedicated_execution`; `targeting.spec.js` |
| Gold drop and explicit pickup | Implemented | [`arpg-core`](crates/arpg-core/src/lib.rs) | `kill_drop_pickup_requires_explicit_interaction`, `interaction_respects_pickup_range` |
| Seeded dungeon, rooms, doors and encounters | Implemented | [`arpg-core`](crates/arpg-core/src/lib.rs) | [`dungeon_properties.rs`](crates/arpg-core/tests/dungeon_properties.rs), `entering_combat_room_locks_connected_doors_until_encounter_is_cleared` |
| Training scenario (`?scenario=training&seed=<u32>`) | Implemented | [`training-arena.js`](web/src/training-arena.js) | `training-arena.test.js`; `app.spec.js`, `hotkeys.spec.js` |
| Workbench spawn/remove/reset and exact tuning | Implemented (local training only) | [`workbench.rs`](crates/arpg-core/src/workbench.rs), [`workbench-controls.jsx`](web/src/workbench-controls.jsx) | `recorded_operations_replay_in_their_order_among_commands`, `exact_tuning_values_are_validated_with_the_content_bounds`, `workbench_operations_are_recorded_and_replay_natively`, `workbench.spec.js` |
| Versioned savestates | Implemented | [`arpg-core`](crates/arpg-core/src/lib.rs), [`save-state.js`](web/src/save-state.js) | `save_state_*` tests, `save-state.test.js`, `savestate.spec.js` |
| Local Rust/Wasm authority | Implemented | [`arpg-web-wasm`](crates/arpg-web-wasm/src/lib.rs) | `arpg-web-wasm` unit tests; all browser specs |
| Peer-hosted co-op | Implemented | [`peer-session.js`](web/src/peer-session.js) | `peer-session.test.js`, `turn.spec.ts` |
| Dedicated online | Implemented | [`arpg-game-server`](crates/arpg-game-server/src/lib.rs), [`dedicated-session.js`](web/src/dedicated-session.js) | `real_arpg_snapshot_matches_the_game_server_runtime_path`, `rejects_malformed_game_payloads_before_the_core_sees_them`, `reliable_one_shots.rs`, `dedicated-session.test.js`, `reliable-command-lane.test.js`, `reliable-one-shots.test.ts`, `dedicated.spec.js` |
| Physics backend | Implemented: Rapier (`rapier3d`) is authoritative until `physics-engine` is finished ([Atlas 2.3.1](docs/decisions/atlas.md)); saves rebuild the physics world from game state | [`physics.rs`](crates/arpg-core/src/physics.rs), [#175](https://github.com/moritzbrantner/arpg/issues/175) | `saves_taken_mid_play_continue_exactly_as_the_uninterrupted_game`, `actual_seeded_ticks_preserve_wall_door_correction_combat_and_lifecycle_acceptance` |
| Typed client runtime, scenario workbench | Planned (F0) | [#52](https://github.com/moritzbrantner/arpg/issues/52), [#53](https://github.com/moritzbrantner/arpg/issues/53) | — |
| Humanoid, camera, jumping, unified input, menus | Planned (F1) | [#55](https://github.com/moritzbrantner/arpg/issues/55)–[#61](https://github.com/moritzbrantner/arpg/issues/61) | — |
| Directional melee, bow, shield, combos, counters | Planned (F2) | [#63](https://github.com/moritzbrantner/arpg/issues/63), [#76](https://github.com/moritzbrantner/arpg/issues/76)–[#80](https://github.com/moritzbrantner/arpg/issues/80) | — |
| Saves, loot, loadouts, co-op parity, content readiness | Planned (F3/F4) | [#68](https://github.com/moritzbrantner/arpg/issues/68)–[#75](https://github.com/moritzbrantner/arpg/issues/75) | — |

The next product foundation is the **mechanical game loop**: locomotion, action timing, targeting, hit/reaction semantics, pickup/reward interaction, and the one-way presentation cue boundary. These mechanics are treated as reusable ARPG domain contracts rather than late polish or per-skill/per-monster special cases. See [the roadmap](docs/ROADMAP.md).

## Foundation ownership

| Concern | Authority |
| --- | --- |
| ARPG rules, player intent, monsters, combat state | `arpg-core` |
| Collision and physical movement | Rapier (`rapier3d`, inside `arpg-core`) until `physics-engine` is finished |
| Scene rendering and renderer contract | `3d-lab` |
| Runtime input semantics and rebinding UI | `input-bindings` |
| Game command/snapshot encoding | `arpg-protocol` |
| Peer rendezvous/signaling | `multiplayer-setup-service` |
| Dedicated sessions, ticks, reconnect, replay/recovery | `game-server` |
| Browser composition/HUD/settings | `web` |

Savestate semantics also live in `arpg-core`: the browser only persists or transfers the versioned save document and asks Rust to validate and restore it. Neither renderer nor network transport is a source of gameplay truth.

## Execution modes

- **Local** — the browser runs `arpg-core` through `arpg-web-wasm`.
- **Peer-hosted co-op** — one browser runs that same Rust/Wasm authority; guests submit sequenced commands and consume host snapshots. `multiplayer-setup-service` remains payload-opaque setup infrastructure.
- **Dedicated online** — the browser connects directly to the runnable `arpg-game-server` WebTransport endpoint. `game-server` owns admission, ticks, reconnect identity, recovery, and transport framing; the browser submits versioned `arpg-protocol` command payloads (held input as datagrams, one-shots such as attacks and the bow draw, release and cancel on the acknowledged reliable command lane; see [dedicated command delivery](docs/ARCHITECTURE.md#dedicated-command-delivery)) and renders verified authoritative snapshots.

Peer-hosted co-op is a convenience/trust mode, not an anti-cheat boundary. The host can cheat. Dedicated play is the trusted authority model.

Every new local or peer-hosted authority receives a fresh browser-generated run seed. Guests do not generate an alternate seed: they receive the host's seed as part of the authoritative snapshot. For deterministic local reproduction, open the browser with `?seed=<u32>`; starting another local game afterward intentionally creates a fresh seed. Dedicated runs receive the server-selected seed through authoritative snapshots.

## Run locally

Rust validation:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
```

Browser shell:

```sh
cargo install wasm-pack --locked --version 0.13.1
cd web
bun install --frozen-lockfile
bun run setup
bun run dev
```

Browser validation after setup:

```sh
bun run format:check
bun run lint
bun run test
bun run build
bunx playwright install chromium
bun run test:browser
```

Formatting writes are explicit through `bun run format`. Browser tests start their
own local servers on OS-allocated ports, use the actual WASM game, and retain
failure traces under ignored `web/test-results/`.

`bun run setup` acquires the accepted browser sources from `input-bindings` and `multiplayer-setup-service`, verifies their exact Git blob hashes, and builds the Rust/Wasm package. Verified foundation files are reused without downloads or rewrites. After setup, `bun run build` bundles locally with Vite and `bun run dev` starts Vite without downloading sources. Run `bun run wasm` again after Rust changes. Generated vendored sources and Wasm output are not committed.

For peer co-op, enter `https://moenarch.com` in **Settings → Peer co-op**, or run a separate `multiplayer-setup-service` deployment. Local development defaults to `http://127.0.0.1:8787`. GitHub Pages requires an HTTPS/WSS deployment whose `ALLOWED_ORIGINS` includes the ARPG Pages origin. Hosts and guests fetch short-lived TURN credentials after lobby admission and refresh them in memory before expiry. The upstream client uses TURN during recovery when direct connectivity fails; deployments without TURN still permit direct connections.

Dedicated server:

```sh
ARPG_SERVER_CERT_PEM=cert.pem \
ARPG_SERVER_KEY_PEM=key.pem \
cargo run -p arpg-game-server --bin server
```

The dedicated host defaults to UDP port `4433` and session path `/arpg`. Optional configuration is available through `ARPG_SERVER_PORT`, `ARPG_SERVER_SESSION_PATH`, `ARPG_SERVER_RECOVERY_PATH`, and `ARPG_SERVER_DRAIN_GRACE_MS`. See [`.env.example`](.env.example) for the environment contract; export these variables into the server process because the native server does not load dotenv files. Keep local configuration, certificates, private keys, and recovery data uncommitted. A fresh run seed is generated once when the process starts and logged for replay evidence; set `ARPG_RUN_SEED=<u32>` to reproduce a known run exactly. In a browser with WebTransport support, enter the resulting HTTPS endpoint (for example `https://127.0.0.1:4433/arpg`) under **Settings → Dedicated online**. The certificate must be trusted by the browser.

## Repository development policy

[`AGENTS.md`](AGENTS.md) describes how to resolve the live shared coding-agent conventions, the domain ownership boundaries, and the checks used by this repository. Shared policy remains in the central checkout rather than being copied or pinned here.

## Pinned foundations

- `rapier3d` (crates.io): `0.36.1`
- `3d-lab`: `f484db8a3d2a7a555fa463eddf9c28790b240ce0`
- `input-bindings`: `b3b7204faa47d3b0af56eebc55cdfd4ced127ddc`
- `multiplayer-setup-service`: `cc5c20fabdac0fc9da4329a5be04d0f4e8eae383`
- `game-server`: `ebd6aaee156c8e12a1444d1dd55a305eae13df86`

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the authority and trust boundaries.
