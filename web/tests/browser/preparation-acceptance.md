# Issue #145 independent acceptance handoff

Specification: owner-approved issue #145, architecture and roadmap at baseline
`3e19b88`; existing ADRs 0001–0003 retain their authority. This acceptance-only
context saw the baseline without an implementation proposal. Scope is knight
preparation into local crypt play, not the horde, boss, multiplayer party UI or
new foundation machinery.

Run `bun run setup`, `bun run build`, then
`bunx playwright test tests/browser/preparation-production.spec.ts` from `web`.
The preview serves built files on an OS-assigned local port, retains failure
traces/screenshots, and attaches entry/play screenshots when reached. The tests
use real WASM and snapshots. Only the actual core request is delayed/aborted;
retry continues with real bytes. This exercises loading and network failure,
not fabricated asset readiness or game results. Preparation accessibility name
and Start action are independently specified user semantics, not prescribed
component structure.

Executable coverage: fresh entry identifying knight and undead crypt;
keyboard/touch entry, real authoritative shield input, settings, narrow viewport,
exit/re-entry/reload; pending core disables Start; cancellation cannot publish a
late play surface; failed core is visible and retry recovers; duplicate Start
leaves one visible play surface. A single canvas alone does not prove one
simulation or GPU cleanup. Also retain the independently authored lifecycle
contract `src/game-client-core-load.test.ts` and runtime tests covering superseded
starts, disposal and in-flight ownership. Run the full applicable suite because
no complete affected-test map is available. These checks do not establish asset
identity or visual quality merely because text, a canvas or shield state passes.

After cancellation, the test waits for the returned Character selection surface's
actual `Ready` status before checking absence of play or reloading. The existing
runtime sets readiness and invokes any pending activation in the same WASM-load
completion callback. Releasing a delayed HTTP request alone does not establish
that completion. On unchanged baseline `3e19b88`, the focused test still fails
earlier because preparation is absent; the strengthened post-cancellation
assertions remain unexecuted rather than claimed verified.

## Unresolved acceptance prerequisites — fail closed

Baseline renders primitive player/weapon nodes in `src/scene-frame.ts`. It has no
approved packaged knight/weapon or crypt kit and no stable artifact manifest.
Therefore no honest asset-request interception/identity assertion can be pinned
yet. Do not create synthetic GLBs, assume filenames, add a debug game authority,
or bless new box arrangements to make this contract green.

- ARPG #55 owns the approved humanoid/weapon bundle, shared geometry/skin/socket
  integration and same preview/in-world identity. Existing upstream ownership is
  asset-tooling #104/#105/#20 and 3d-lab #87. Closed #62 is private teaching
  material rather than the required public imported-character consumer. #56
  owns animation integration.
- ARPG #72 owns approved environment kit/loading and retained scene integration,
  using asset-tooling #106. Shared renderer capabilities remain owned by 3d-lab.
- ARPG #60 owns navigation/lifecycle. Core/physics/input owners remain as stated
  in AGENTS.md; preparation may load resources, never advance hidden gameplay.

Before implementation is accepted, a separate acceptance context must add tests
against the delivered approved bundle's actual stable identity/provenance/public
render seam: same real knight/sword/shield in entry and world, crypt kit actually
loaded from packaged assets; delay/abort one minimum asset and assert Start
stays disabled, visible failure, retry and cancellation; shared retained
resources and one live authority across double activation/exit/re-entry. Use
existing shared renderer observability if available; report a missing foundation
seam upstream rather than introducing a private renderer. Verify ordinary
movement and attack snapshots remain real Rust/WASM outcomes.

A passing entry subset is not a complete issue #145 handoff until those asset
acceptance prerequisites are independently specified and exercised. This draft
can carry the tests first, but must remain draft while that evidence is missing.

Manual visual/public gate: review screenshots of real entry/play at desktop and
touch sizes against worn steel armor, chainmail, helmet, sword/shield and dark
undead crypt intent. Confirm asset lineage and stable mapping; a successful HTTP
request alone is insufficient. Open https://moritzbrantner.github.io/arpg/ fresh,
Start, move/attack/guard, settings, exit and re-enter, verifying the deployed
revision and actual same character/crypt presentation. Record post-merge live
handoff before any merge request; no public pass is established by preview.

Shared convention sourceRevision:
`46d8793bb3034326561f876dcc67dbaa5aa1e432`.
