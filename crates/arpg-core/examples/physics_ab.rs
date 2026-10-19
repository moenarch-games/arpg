//! Manual whole-physics-path timing of the real game on Rapier and on physics-engine
//! (physics-engine#330). Not a gate: wall-clock time on shared runners is too noisy.
//!
//! ```sh
//! cargo run --release --locked -p arpg-core --features physics-ab --example physics_ab
//! ```
//!
//! Every seeded workload runs through the actual `ArpgGame` tick on each backend. Each tick's
//! time is the sum of every call into the game's physics module during it (commands included):
//! adapter conversions, Rapier's per-tick rebuild, character moves, door colliders and every
//! query. Gameplay outside physics is not counted. Setup (floor, static boxes, first bodies)
//! is reported apart. Runs are paired (the backends alternate which goes first) after
//! warm-up runs; the table gives median, p95 and p99 per tick over every tick of every run, the
//! share of the ticks' physics time spent in the per-tick world rebuild (which ARPG needs on
//! Rapier so that a restored save continues exactly; physics-engine needs none), and the median
//! of the per-run ratio of total physics time. The timer itself adds two clock reads (tens of
//! nanoseconds) to every physics call on both backends.

use std::time::Duration;

use arpg_core::physics_ab::{Backend, Timing, engine_unsupported, time_workload, workloads};

const SEED: u32 = 42;
const TICKS: u32 = 240;
const WARMUP_RUNS: usize = 3;
const RUNS: usize = 20;

fn micros(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e6
}

/// The `q` quantile of sorted samples (nearest rank).
fn quantile(sorted: &[Duration], q: f64) -> Duration {
    let rank = ((q * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1]
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

#[derive(Default)]
struct Samples {
    ticks: Vec<Duration>,
    setups: Vec<Duration>,
    totals: Vec<Duration>,
    rebuilds: Vec<Duration>,
    last: Option<Timing>,
}

impl Samples {
    fn add(&mut self, timing: Timing) {
        self.ticks.extend(&timing.ticks);
        self.setups.push(timing.setup);
        self.totals.push(timing.ticks.iter().sum());
        self.rebuilds.push(timing.rebuild);
        self.last = Some(timing);
    }
}

fn revision() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map_or_else(|| "unknown".to_owned(), |rev| rev.trim().to_owned())
}

fn engine_revision() -> &'static str {
    let manifest = include_str!("../Cargo.toml");
    let line = manifest
        .lines()
        .find(|line| line.starts_with("physics-engine = "))
        .expect("physics-engine dependency");
    let start = line.find("rev = \"").expect("pinned revision") + 7;
    &line[start..start + 12]
}

fn cpu() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|info| {
            info.lines()
                .find(|line| line.starts_with("model name"))
                .and_then(|line| line.split(':').nth(1))
                .map(|name| name.trim().to_owned())
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

fn main() {
    println!(
        "ARPG {} | physics-engine {} | rapier3d 0.36.1 | {} {} | {}",
        revision(),
        engine_revision(),
        std::env::consts::OS,
        std::env::consts::ARCH,
        cpu()
    );
    println!(
        "seed {SEED}, {TICKS} ticks per run, {WARMUP_RUNS} warm-up + {RUNS} paired runs per \
         workload; physics time per tick in µs"
    );
    println!();
    println!(
        "| workload | players | static boxes | peak monster bodies | backend | median | p95 | p99 \
         | setup median | rebuild share | engine/rapier total |"
    );
    println!("|---|---:|---:|---:|---|---:|---:|---:|---:|---:|---:|");
    let mut all: [Vec<Duration>; 2] = Default::default();
    for workload in workloads() {
        if let Some(reason) = engine_unsupported(workload) {
            println!("| {workload} | | | | physics-engine | unsupported: {reason} | | | | | |");
            continue;
        }
        for _ in 0..WARMUP_RUNS {
            for backend in Backend::ALL {
                time_workload(backend, workload, SEED, TICKS);
            }
        }
        let mut samples: [Samples; 2] = Default::default();
        for run in 0..RUNS {
            let order = if run % 2 == 0 { [0, 1] } else { [1, 0] };
            for index in order {
                let timing = time_workload(Backend::ALL[index], workload, SEED, TICKS);
                all[index].extend(&timing.ticks);
                samples[index].add(timing);
            }
        }
        let ratio = median(
            samples[1]
                .totals
                .iter()
                .zip(&samples[0].totals)
                .map(|(engine, rapier)| engine.as_secs_f64() / rapier.as_secs_f64())
                .collect(),
        );
        for (index, backend) in Backend::ALL.into_iter().enumerate() {
            let samples = &mut samples[index];
            samples.ticks.sort();
            samples.setups.sort();
            let last = samples.last.as_ref().expect("timed runs");
            let rebuild_share = samples.rebuilds.iter().sum::<Duration>().as_secs_f64()
                / samples.totals.iter().sum::<Duration>().as_secs_f64();
            println!(
                "| {workload} | {} | {} | {} | {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.0}% | {} |",
                last.players,
                last.fixed,
                last.peak_monster_bodies,
                backend.name(),
                micros(quantile(&samples.ticks, 0.5)),
                micros(quantile(&samples.ticks, 0.95)),
                micros(quantile(&samples.ticks, 0.99)),
                micros(quantile(&samples.setups, 0.5)),
                rebuild_share * 100.0,
                if index == 1 {
                    format!("{ratio:.2}")
                } else {
                    String::new()
                },
            );
        }
    }
    for (index, backend) in Backend::ALL.into_iter().enumerate() {
        let ticks = &mut all[index];
        ticks.sort();
        println!(
            "| all | | | | {} | {:.1} | {:.1} | {:.1} | | | |",
            backend.name(),
            micros(quantile(ticks, 0.5)),
            micros(quantile(ticks, 0.95)),
            micros(quantile(ticks, 0.99)),
        );
    }
}
