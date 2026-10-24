# ADR 0004 — Cross-platform game audio (native Rust and GitHub Pages)

Status: **proposed**, waiting on the owner's choice (#67 slice 67c, 2026-10-24).
Nothing is implemented until the owner picks an option on #67.

## Context

#67 needs short, overlapping combat and UI sounds (attack, impact, block, hurt,
defeat, pickup, bow release, arrow impact, counter available, UI). They are
driven by the 67a presentation cues. The owner's direction (live, 2026-10-24) is
to choose one worthwhile audio design. It must work inside the native Rust client
(mmorpg's wgpu/winit `mmorpg-client`) and in the browser builds on GitHub Pages
(arpg web, mmorpg web), using only free/open-source libraries and CC0 sounds.

What exists today:

- No audio anywhere: no audio crate or package in arpg, mmorpg (native or web) or
  3d-lab, and no audio assets. arpg has no native client.
- `media-playback` does clock/seek playback for media elements. It is not a
  low-latency SFX mixer. `audio-analysis` owns decoding, analysis, synthesis and
  TTS. Runtime game playback is outside its documented scope (`CONTEXT.md`).
- `settings` already declares an `AudioSettingsAdapter` domain trait.
  Persisted volume and mute preferences belong there, not in a new authority.
- mmorpg Atlas 6.6.37 wants restrained **spatial** anticipation cues. It maps
  cues to an "existing native/browser playback provider", which doesn't exist
  yet, and it forbids inventing a separate mixer or settings authority inside
  mmorpg. So whatever 67c builds has to be the shared provider.
- Neither web build is cross-origin isolated (no COOP/COEP, no
  `SharedArrayBuffer`). GitHub Pages can't send those headers.
- Sounds: asset-tooling#297 (Freesound CC0 fixture set) is **open and not
  started**.

## Candidates

Maintenance data is from crates.io, npm and GitHub on 2026-10-24.

| | kira 0.12.5 | rodio 0.23 | oddio 0.7.4 | fyrox-sound 1.0.1 | web-audio-api-rs 1.7 | Browser Web Audio (built in) | howler.js 2.2.4 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Licence | MIT/Apache-2.0 | MIT/Apache-2.0 | MIT/Apache-2.0 | MIT | MIT | — | MIT |
| Last release | 2026-09-26 | 2026-10-11 | **2023-10** (stale) | 2026-03 | 2026-08 | browser | **2023-09** |
| Adoption | 228k recent dl, 1.1k★ | 3.7M recent dl | 8k | 5.7k (Fyrox-coupled) | 34k, 389★ | universal | 25k★ |
| Native | cpal | cpal | needs own output | tinyaudio | cpal | — | — |
| Web | cpal WebAudio host | cpal WebAudio host | manual | tinyaudio | (it *is* the spec; use the browser) | native API | yes |
| Mixer buses / volume channels | sub-tracks, effects | basic `Mixer`, no bus tree | manual | yes | GainNode graph | GainNode graph | global + per-sound only |
| Tweens / fades | yes, all params | limited | some | some | AudioParam ramps | AudioParam ramps | fade only |
| Spatial | listener + spatial tracks (distance + pan) | simple ear-position `Spatial` | yes | HRTF | PannerNode incl. HRTF | PannerNode incl. HRTF | stereo / panner plugin |
| Voice limiting | track capacity (play errors when full; no stealing) | none | none | none | none | none | pool size per sound |

None of the libraries does priority-based voice stealing or per-cue cooldowns.
Either way the game owns that policy.

### Web-specific findings

- **cpal's default WebAudio host** (which kira and rodio use on wasm32) renders
  through a `ScriptProcessorNode` callback **on the main thread**. The API is
  deprecated but still ships in every browser. Mixing then competes with
  three.js rendering, the WASM simulation step, asset loading and GC. A frame
  hitch becomes an audible crackle unless the buffer is large (2048–4096 frames
  ≈ 46–93 ms at 44.1 kHz), and a large buffer hurts SFX latency.
- **cpal's `audioworklet` host** moves rendering off the main thread. It needs
  wasm atomics (rebuilding std on nightly) and `SharedArrayBuffer`, which needs
  COOP/COEP headers. On GitHub Pages that means the `coi-serviceworker`
  reload workaround. Both arpg and mmorpg use stable Rust and aren't isolated.
- **Browser Web Audio** mixes `AudioBufferSourceNode`s on the browser's own
  audio rendering thread, at the browser's lowest latency. Decoding is free
  (`decodeAudioData`), and per-voice cost is native. A main-thread hitch
  delays the *start* of a sound by at most that hitch. It doesn't break
  playing audio.
- **Unlock and suspend** are the same on every option. Create or resume the
  `AudioContext` on the first user gesture, and `suspend()`/`resume()` on
  `visibilitychange`. On the cpal path this goes through `Stream::pause/play`.
  Natively there's no gesture gate; suspend follows window focus/minimise.

### Measured WASM size (kira probe)

A minimal kira wasm32 `cdylib` (manager + sub-track + listener + play, `opt-level
= "z"`, LTO, wasm-bindgen 0.2.129, before wasm-opt):

| kira features | raw | gzip -9 |
| --- | --- | --- |
| `cpal` only (browser decodes, PCM handed in) | 203 KB | 69 KB |
| `cpal`, `wav`, `pcm` | 327 KB | 104 KB |
| `cpal`, `ogg`, `vorbis` | 574 KB | 195 KB |

A Web Audio backend driven through `web-sys` adds only bindings, a few tens of KB.

## Options

**A — Shared Rust audio director, kira natively, the browser's Web Audio on the
web (recommended).** One backend-neutral Rust crate owns everything that is
*design*:

- the sound vocabulary manifest (cue kind → clip variants, bus, priority);
- the bus tree (master → effects / UI, room for ambient/music later);
- voice limits with priority stealing, and per-cue cooldown;
- listener and distance attenuation, with **pan and gain computed in the director**;
- mapping from `settings` volume/mute;
- suspend/unlock state.

It emits a tiny command set: load clip, start voice (clip, bus, gain, pan, rate),
stop or fade voice, set bus gain, suspend/resume. Two thin backends carry these out:

- native: `kira` (sub-tracks as buses, tweens for fades, cpal output);
- web: a `web-sys` backend in the same crate (GainNode buses,
  StereoPannerNode, AudioBufferSourceNode, `decodeAudioData`), exposed to the
  arpg and mmorpg front ends through one wasm-bindgen package.

Because spatialisation is computed in the director, native and web sound the
same. The policy is unit-tested once and backend-agnostic. On the web, mixing
stays off the main thread on stable Rust with no isolation headers, and the
wasm cost is small. The cost is keeping two backends, roughly 200–300 lines each,
for a deliberately tiny command set.

**B — kira everywhere.** Use the same kira code on native and wasm through
cpal's WebAudio host. It has a single backend and the richest feature set on
both. But it mixes on the main thread in the browser through a deprecated node,
needs a large buffer to avoid crackle under frame hitches, and adds about
70–195 KB gzip. Moving to the AudioWorklet host later needs nightly atomics and
`coi-serviceworker` on Pages.

**C — web-audio-api-rs natively, browser Web Audio on the web.** Program against
W3C Web Audio semantics everywhere: the Rust re-implementation natively, the
real API in the browser. It's elegant and gives HRTF on both. But it's a spec
emulation with a smaller maintainer base (13 contributors), and it isn't
game-oriented: no game-level voice or bus helpers beyond the graph. You still
need adapter code, because web-sys and web-audio-api-rs signatures differ.

**D — howler.js on the web plus a Rust library natively.** This is two
unrelated designs, which is what the owner rejected. howler.js has also had no
release since 2023-09.

Rejected outright: `oddio` (no release since 2023), `fyrox-sound` (tied to the
Fyrox engine, little adoption outside it), and bare `rodio`/`cpal` (we would
rebuild buses, tweens and spatial ourselves).

## Where the shared code lives

The director has to be the same in arpg and mmorpg, and mmorpg Atlas 6.6.37 won't
allow a mixer authority inside mmorpg. No existing repo owns runtime game
playback. The choices are:

1. a new shared repo (for example `game-audio`) consumed by pinned git revision,
   the same way `input-bindings`, `settings` and `game-server` are consumed today
   (recommended);
2. a new crate inside `audio-analysis`, which stretches its analysis/synthesis scope;
3. a crate inside arpg first, extracted when mmorpg adopts it.

Settings keep their owner: volume and mute persist through `settings`'
`AudioSettingsAdapter`. The audio crate only applies them.

## Defaults under any option (agent decisions, listed for review)

- Volume channels for 67c: master, effects, UI, plus mute. Ambient/music buses
  are reserved but not exposed.
- Cosmetic cues are dropped when the voice cap is reached: the lowest priority
  and oldest voice is stolen first. Critical state never depends on a sound
  (67a).
- Replayed or deduplicated cues never replay sounds: the director consumes 67a
  cue keys, not raw snapshots.
- Missing assets, a blocked `AudioContext` or a failed decode → silent, with
  visual feedback still complete. There's no substitute sound.
- Clip format: short PCM WAV for the first CC0 set (tiny clips, decoded
  everywhere). Compressed formats can come later if the size matters.
