# Ranger loading

Setup acquires the rolling `humanoid-ranger-male` producer index, the default Ranger,
its shared clips, sword, shield, referenced textures and JSON attachments. It serves
these selected per-entry files locally; the browser never downloads a release tar.
The outfit, socket configuration and animation choices in `character-bundles.js`
remain the game’s authority for presentation.

`bun run characters` performs acquisition and copies the installed Three.js Basis
transcoder into `public/decoders/basis/`. Builds do not acquire anything. An unchanged
release performs no file writes; missing files are repaired individually. A failed
replacement retains the previous complete local release. The game canvas stays
mounted while hidden on character selection, so KTX2 capability detection uses the
same renderer that draws the world. Hidden canvases do not render frames.

## Observed production-build comparison (2026-10-23)

Three paired cold Chromium 156.0.8078.4 contexts, SwiftShader, 1280×800, 40 ms network
latency and 1,250,000 bytes/s download. Both are Vite production previews without HTTP
body compression. Medians are observations of this controlled local environment;
GitHub Pages compression, cache state, real GPUs and networks may change the result.
Before: current main plus PR #244’s full-archive change, old renderer `c850bda`.
After: the same Rust/WASM build and default equipment with compressed entries and
renderer `79ecba1` (3d-lab #158). The shared loader separately compares every one of the
52 clips at three sample times with plain Three.js on default and knight entries.

| Observation | Before | After | Reduction |
| --- | ---: | ---: | ---: |
| Character/equipment asset bodies | 14,942,522 bytes | 8,820,637 bytes | 41.0% |
| All resource transfer bytes, including HTTP overhead and codecs | 18,986,425 | 13,442,764 | 29.2% |
| Navigation → Ready | 16.82 s | 12.57 s | 25.3% |
| Navigation → entered world frame | 17.19 s | 12.92 s | 24.8% |

The complete setup tar was 32,044,544 bytes, but the old browser already fetched only
its selected unpacked resources. Therefore the tar-to-entry size ratio is not the
browser startup improvement. Codec acquisition/transcoding and application/WASM
loading remain visible in the measurements above. Leaving and re-entering the world
on the same page reuses the retained character assets without additional downloads.

Run `bun scripts/measure-character-loading.ts <built-baseline-web-root> [output-dir]`
from `web` after both setup/build steps. It owns temporary preview servers and cold
browser contexts, records per-resource encoded bodies/transfer bytes and timings,
and writes observations plus world screenshots. Ordinary CI uses offline preparation
tests and the real humanoid browser scenarios; no timing threshold is added.
