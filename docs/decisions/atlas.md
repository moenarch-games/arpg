# ARPG Decision Atlas

This is ARPG's canonical product-intent tree. It consolidates **existing, owner-approved** direction from [ROADMAP](../ROADMAP.md), [ARCHITECTURE](../ARCHITECTURE.md), [AGENTS](../../AGENTS.md) and existing product issues, rather than treating code as a new product decision. Only **Decisions** and **Questions** are semantic entries; numbered headings are navigation. IDs are stable and must not be renumbered.

## Implementation evidence (retrospective pilot)

Child `- [x]` means **only the specifically named implemented capability** has cited source/test evidence. `- [ ]` means that particular behavior or verification remains outstanding. A checked technical foundation does **not** mean the complete parent Decision, public Pages experience or issue is finished. No checklist means *not audited*. These checklists do not create a third entry kind or an issue queue. Source paths plus symbols are durable pointers; tests and exact revisions establish evidence. The current `daq` indexer does not yet project child checkbox lines (see [coding-tooling #322](https://github.com/moritzbrantner/coding-tooling/issues/322) and [#323](https://github.com/moritzbrantner/coding-tooling/issues/323)).

- **1** Playable experience
  - **1.1** Undead crypt vertical slice
    - **1.1.1** [decision] A practical armored knight prepares before entering actual Rust-authoritative undead-crypt gameplay; real assets and honest loading are required, not a decorative mock. ([#145](https://github.com/moritzbrantner/arpg/issues/145))
      - [x] Local-game starting and saved character selection already exist: `web/src/character-selection.js::resolveCharacter`, `::persistSelectedCharacterId`, `web/src/main.tsx`; tested in `web/src/character-selection.test.js` ([current capability inventory](../../README.md)).
      - [x] WASM exposes real seeded gameplay creation: `crates/arpg-web-wasm/src/lib.rs::WasmGame::new_scenario` and `::advance_tick`.
      - [ ] Actual approved knight/weapon models and crypt entry/ready/failure journey have not passed the production-browser contract; acceptance is still draft ([#145](https://github.com/moritzbrantner/arpg/issues/145), [PR #160](https://github.com/moritzbrantner/arpg/pull/160)).
    - **1.1.2** [decision] The Pages combat experience progresses from a physically shattering skeleton horde to an armored tactical duel and an executioner boss with victory/retry; independently test the real published game. ([#146](https://github.com/moritzbrantner/arpg/issues/146), [#147](https://github.com/moritzbrantner/arpg/issues/147), [#148](https://github.com/moritzbrantner/arpg/issues/148))
      - [x] Authoritative action phases, facing, telegraphs, staggers and defeat mechanics already exist in `crates/arpg-core/src/lib.rs` (named regressions `attacks_have_authoritative_commitment_active_and_recovery_phases`, `monster_attack_is_telegraphed_before_damage` in the [capability inventory](../../README.md)).
      - [ ] Real horde destruction, knight encounter, boss progression and published Pages visual acceptance have not been established by the existing core tests.
- **2** Authority and implementation boundaries
  - **2.1** One gameplay authority
    - **2.1.1** [decision] Deterministic combat, character and encounter outcomes belong to `arpg-core`; the browser, network adapter and 3D renderer consume outcomes rather than deciding them. ([architecture](../ARCHITECTURE.md), [#63](https://github.com/moritzbrantner/arpg/issues/63))
      - [x] The Rust-authoritative `crates/arpg-core/src/lib.rs` and `crates/arpg-web-wasm/src/lib.rs::WasmGame::apply_command` exist; movement/combat/snapshot tests are enumerated in the [current capability inventory](../../README.md).
      - [ ] This does not prove the intended real humanoid locomotion, integrated crypt animation and physical bone breakup on production Pages ([#55](https://github.com/moritzbrantner/arpg/issues/55), [#56](https://github.com/moritzbrantner/arpg/issues/56)).
  - **2.2** Reuse the same core across modes
    - **2.2.1** [decision] Local WASM, peer-hosted and dedicated play consume the same ARPG core commands and snapshot authority, with transport-specific delivery outside gameplay rules. ([architecture](../ARCHITECTURE.md), [#71](https://github.com/moritzbrantner/arpg/issues/71))
      - [x] Existing adapters: `crates/arpg-web-wasm/src/lib.rs::WasmGame`, `web/src/peer-session.js::attachPeerGameSession`, `crates/arpg-game-server/src/lib.rs` and `web/src/dedicated-session.js`; server/lane tests documented in [README](../../README.md).
      - [x] Aimed one-shot delivery carries validated aim through the correct command lane (`crates/arpg-core/tests/aimed_one_shots.rs`, `crates/arpg-core/tests/command_lanes.rs`, merged [PR #158](https://github.com/moritzbrantner/arpg/pull/158)).
      - [ ] Complete co-op session continuity and public gameplay parity remain in [#71](https://github.com/moritzbrantner/arpg/issues/71).
- **3** Content and authoring
  - **3.1** Authoritative data definitions
    - **3.1.1** [decision] Use typed versioned content definitions for game-specific weapons, monsters, abilities and encounters; do not create gameplay authority inside the renderer. ([CONTENT](../CONTENT.md), [#73](https://github.com/moritzbrantner/arpg/issues/73))
      - [x] Versioned content bundle and validation exist in `crates/arpg-core/content/base.json`, `crates/arpg-core/src/content.rs`; tests include `base_content_reproduces_the_previous_tuning` and `saves_from_another_content_revision_are_rejected` ([README](../../README.md)).
      - [x] Seeded rooms/doors and encounter behavior exist (`crates/arpg-core/tests/dungeon_properties.rs`; `entering_combat_room_locks_connected_doors_until_encounter_is_cleared`).
      - [ ] A visually complete, reusable crypt presentation kit and owner-verifiable first adventure remain in [#72](https://github.com/moritzbrantner/arpg/issues/72) and [#145](https://github.com/moritzbrantner/arpg/issues/145).

## Issue handoffs

Product intent remains here. GitHub issues should form a mostly **flat queue of small, verifiable slices**, not persistent nested epics. Close a too-large issue with an explicit successor handoff, marking it superseded rather than completed if its original acceptance did not pass. One active implementation worker per repository; independent acceptance authorship and implementation occur sequentially. Do not create duplicate work already covered by the tests and source above.
