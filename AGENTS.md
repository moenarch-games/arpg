# Working in arpg

## Shared engineering policy

Use the current shared `coding-agent-conventions` authority for repository work.
Before planning or implementing a non-trivial change, resolve the policy from
the repository root:

```sh
bun /home/moenarch/moritzbrantner/coding-tooling/src/cli.ts conventions resolve --root "$PWD" --registry /home/moenarch/.config/moenarch/environment.toml --json
```

Read every file in `data.files` and any applicable repository-local `AGENTS.md`
or `CLAUDE.md`. Local instructions override shared policy only where they
conflict. Report a resolution failure instead of guessing the policy. Record
the returned `sourceRevision` when reporting reproducibility evidence; do not
pin or copy shared policy into this repository.

The repository currently combines a Rust workspace with a JavaScript/React
browser shell, Vite, Bun, and Playwright. Resolve the stack again when that changes rather
than treating this description as a fixed policy selection.

## Domain ownership

The canonical [Decision Atlas](docs/decisions/atlas.md) records owner-approved product direction. Consult relevant Decisions/Questions before consequential product forks; nested binary implementation checkboxes identify only scoped code/test evidence, not full product acceptance. Treat GitHub issues as a mostly flat queue of bounded handoffs and use one active implementation worker per repository (independent acceptance context operates sequentially). Do not re-ask settled decisions or infer a complete product from a merged technical PR.

- `crates/arpg-core` owns deterministic gameplay rules and authoritative state.
- Rapier (`rapier3d`, inside `arpg-core`'s Rust/WASM simulation) resolves collision
  and physical movement until `physics-engine` is finished (Atlas 2.3.1); report concrete
  Rapier gaps and lessons as physics-engine issues. `3d-lab` owns rendering.
- `input-bindings` owns input semantics and rebinding; the multiplayer setup
  service owns peer rendezvous.
- `crates/arpg-protocol` owns ARPG command and snapshot encoding.
- `crates/arpg-game-server` adapts gameplay to the shared `game-server` runtime.
- `crates/arpg-web-wasm` exposes Rust gameplay to the browser.
- `web` composes rendering, networking, HUD, and settings; it must not redefine
  authoritative gameplay semantics.

Read `docs/ARCHITECTURE.md` for trust boundaries and `docs/ROADMAP.md` for planned
capabilities. Fix foundation defects in their owning repositories rather than
adding competing implementations here.

## Setup and validation

Use the toolchain in `rust-toolchain.toml` and Bun version in `web/package.json`.
Install browser dependencies with `bun install --frozen-lockfile` in `web`.
Install `wasm-pack` with `cargo install wasm-pack --locked --version 0.13.1`.

Run affected checks first. The repository's completion checks are:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cd web
bun run format:check
bun run lint
bun run test
bun run setup
bun run build
bun run test:browser
```

`bun run setup` acquires pinned browser foundation sources and generates WASM,
so cold setup requires network access. `bun run build` and `bun run dev` use
prepared inputs without acquiring dependencies. Run `bun run wasm` after Rust
changes. The ordinary
browser tests do not require vendored foundations or a running service.
Install Chromium with `bunx playwright install chromium` before browser workflow
tests. CI also installs its system dependencies with `--with-deps`. Workflow
tests own disposable servers and ports and retain failures under ignored
`web/test-results/`; they do not require public services.
Generated `web/src/vendor`, `web/src/wasm`, `web/dist`, and `target` are disposable
and uncommitted. Dependency verification must preserve `Cargo.lock` and
`web/bun.lock`.

Server environment settings are documented in `.env.example`; the native
server reads its process environment and does not load that file automatically.

## Integration

Work from an explicit baseline on an `agent/<short-topic>` branch. Use the
shared Git convention for pull requests and integration after repository checks
pass. Keep behavior changes separate from formatting or structural cleanup.
