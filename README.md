# arpg

Architecture-first action RPG foundation inspired by the systemic shape of Diablo II, built as an original implementation.

The MVP is intentionally an **integration proof**: gameplay truth lives in one deterministic Rust simulation and the renderer, browser UI, peer networking, and dedicated-server runtime consume that authority rather than replacing it.

## Playable MVP

The browser slice currently proves:

- WASD top-down movement with diagonal normalization;
- a procedurally partitioned, seeded dungeon whose walls and doorways are resolved by Rapier physics inside the Rust simulation;
- an isometric follow camera and 3D scene rendered through `3d-lab`, with players shown as the released asset-tooling Ranger humanoid (unpacked by `bun run setup`);
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
| Physics backend | Implemented: Rapier (`rapier3d`) is authoritative until `physics-engine` is finished ([Atlas 2.3.1](docs/decisions/atlas.md)); saves rebuild the physics world from game state; the test/bench-only `physics-ab` feature compares the real game on Rapier and physics-engine ([ARCHITECTURE](docs/ARCHITECTURE.md#physics-ab-against-physics-engine-test-and-bench-only)) | [`physics.rs`](crates/arpg-core/src/physics.rs), [#175](https://github.com/moritzbrantner/arpg/issues/175) | `saves_taken_mid_play_continue_exactly_as_the_uninterrupted_game`, `actual_seeded_ticks_preserve_wall_door_correction_combat_and_lifecycle_acceptance`, `physics_ab::tests`, `examples/physics_ab.rs` |
| Flat-ground jumping and support | Implemented (first slice of [#59](https://github.com/moritzbrantner/arpg/issues/59)): `Space` jumps on the standard profile (`R` is primary attack); coyote time, jump buffer and air control from [`base.json`](crates/arpg-core/content/base.json); guard and counter need the ground, airborne attacks are allowed | [`arpg-core`](crates/arpg-core/src/lib.rs), [`physics.rs`](crates/arpg-core/src/physics.rs) | [`jumping.rs`](crates/arpg-core/tests/jumping.rs), `walking_off_the_ground_opens_the_coyote_window`, `shots_meet_bodies_at_any_height`, [`jump_parity.rs`](crates/arpg-game-server/tests/jump_parity.rs), `jump.spec.js` |
| Typed client runtime, scenario workbench | Planned (F0) | [#52](https://github.com/moritzbrantner/arpg/issues/52), [#53](https://github.com/moritzbrantner/arpg/issues/53) | — |
| Walkable regions | Implemented ([#207](https://github.com/moritzbrantner/arpg/issues/207)): one region per flat floor, typed connections and transit; ordinary melee, engagement, locks and interaction are region-local | [`topology.rs`](crates/arpg-core/src/topology.rs) | `ordinary_combat_targeting_and_interaction_are_region_local`, `an_actor_in_transit_stays_reachable_from_both_ends`, `saves_keep_the_region_and_reject_an_unknown_or_unsupported_one` |
| Platforms and ledges | Implemented ([#208](https://github.com/moritzbrantner/arpg/issues/208)): the `traversal` scenario's platform is its own region; jump onto it, walk off its ledge, saves continue on it and mid-drop; seeded A/B `ledge` workload | [`lib.rs`](crates/arpg-core/src/lib.rs) | [`traversal.rs`](crates/arpg-core/tests/traversal.rs), `physics_ab::tests`, `workbench.spec.js` |
| Low ceilings | Implemented ([#209](https://github.com/moritzbrantner/arpg/issues/209)): a jump under the `traversal` scenario's ceiling stops at it and falls back; monsters walk under it; seeded A/B `ceiling` workload | [`lib.rs`](crates/arpg-core/src/lib.rs) | [`traversal.rs`](crates/arpg-core/tests/traversal.rs), `monsters_walk_under_a_low_ceiling_but_around_a_platform`, `physics_ab::tests`, `workbench.spec.js` |
| Bridge and typed transit | Implemented ([#210](https://github.com/moritzbrantner/arpg/issues/210)): a bridge over a floor path (overlapping regions), transit along declared jump/drop connections, layouts with undeclared crossings rejected; seeded A/B `traversal` workload | [`topology.rs`](crates/arpg-core/src/topology.rs) | [`traversal.rs`](crates/arpg-core/tests/traversal.rs), `a_player_on_the_bridge_and_a_monster_on_the_path_below_never_meet_in_melee`, `the_traversal_layout_rejects_an_undeclared_crossing`, `physics_ab::tests` |
| Fall recovery | Implemented ([#211](https://github.com/moritzbrantner/arpg/issues/211)): a player out of the world returns to its last supported position, unharmed | [`lib.rs`](crates/arpg-core/src/lib.rs) | `a_player_out_of_the_world_returns_to_its_last_supported_position_unharmed`, `saves_reject_a_last_supported_position_out_of_the_world` |
| Swept lunge | Implemented ([#90](https://github.com/moritzbrantner/arpg/issues/90)): the skirmisher lunges through a 5-tick active window and its strike sweeps the travelled path, connecting once; walls stop it; seeded A/B `lunge` workload | [`lib.rs`](crates/arpg-core/src/lib.rs), [`base.json`](crates/arpg-core/content/base.json) | `a_lunge_catches_a_target_beyond_its_reach_once_along_its_path`, `a_lunge_into_a_thin_wall_stops_at_it_and_is_obstructed`, `a_save_mid_lunge_continues_exactly`, `physics_ab::tests` |
| Damageable crates | Implemented ([#90](https://github.com/moritzbrantner/arpg/issues/90)): two crates per combat room block movement and navigation until player strikes or arrows break them; no drops; strike rays ignore the struck crate on both physics backends; seeded A/B `crates` workload | [`lib.rs`](crates/arpg-core/src/lib.rs), [`base.json`](crates/arpg-core/content/base.json) | `strikes_break_a_crate_which_then_stops_blocking_and_drops_nothing`, `a_segment_can_ignore_one_static_body_but_not_the_others`, `physics_ab::tests` |
| Walkable stairs | Implemented ([#222](https://github.com/moritzbrantner/arpg/issues/222)): walking players climb 25-unit steps (autostep on both backends); the `traversal` scenario's stairs lead onto the platform; seeded A/B `stairs` workload (engine climbs a rounded edge a tick behind, disclosed) | [`physics/rapier.rs`](crates/arpg-core/src/physics/rapier.rs) | `a_walking_player_climbs_the_stairs_onto_the_platform_without_jumping`, `a_walking_body_climbs_a_low_step_but_not_a_high_one_or_in_the_air`, `workbench.spec.js` |
| Ground burst | Implemented ([#64](https://github.com/moritzbrantner/arpg/issues/64)): the bruiser's telegraphed slam strikes every player in reach (area query on the physics seam, both backends), unblockable, no friendly fire; seeded A/B `slam` workload | [`lib.rs`](crates/arpg-core/src/lib.rs), [`base.json`](crates/arpg-core/content/base.json) | `a_bruiser_burst_strikes_every_player_in_reach_but_no_monster`, `an_area_column_finds_the_characters_it_overlaps_in_key_order`, `physics_ab::tests` |
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
- `3d-lab`: `c08cc252070d0eb887731cd174803ed26e07aefa`
- `input-bindings`: `b3b7204faa47d3b0af56eebc55cdfd4ced127ddc`
- `multiplayer-setup-service`: `cc5c20fabdac0fc9da4329a5be04d0f4e8eae383`
- `game-server`: `ebd6aaee156c8e12a1444d1dd55a305eae13df86`

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the authority and trust boundaries.
