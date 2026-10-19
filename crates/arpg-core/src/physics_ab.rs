//! The real game on both physics backends (physics-engine#330), compiled only with the
//! test/bench-only `physics-ab` feature. Rapier stays the production authority (Atlas 2.3.1).
//!
//! The seeded whole-game workloads (`seeded_workloads`: door, correction, combat, crowd, chase
//! and the rest) run through the actual `ArpgGame` tick, its commands, unit conversions and
//! physics calls, once on Rapier and once on physics-engine, from the same initial state with
//! the same inputs and tick count. The merge-blocking test below checks the shared invariants
//! on both and compares positions and query decisions; [`time_workload`] serves the manual
//! `physics_ab` timing example.

use std::time::Duration;

use crate::seeded_workloads::{Case, commands, fixture};
use crate::{ArpgGame, AuthoritativeGame};

pub use crate::physics::{
    Backend, take_physics_time, take_rebuild_time, time_physics, with_backend,
};

/// The names of the seeded workloads, in order.
pub fn workloads() -> impl Iterator<Item = &'static str> {
    Case::ALL.into_iter().map(Case::name)
}

fn case(name: &str) -> Case {
    Case::ALL
        .into_iter()
        .find(|case| case.name() == name)
        .unwrap_or_else(|| panic!("unknown workload {name}"))
}

/// Physics time of one workload run on one backend.
#[derive(Clone, Debug)]
pub struct Timing {
    /// Physics time while the game was built: floor, static boxes, doors and the first
    /// players' bodies.
    pub setup: Duration,
    /// Physics time of each tick (its commands included): the whole physics path, from the
    /// game's calls through the adapter to the backend and back.
    pub ticks: Vec<Duration>,
    /// The part of the ticks' physics time spent rebuilding the world (Rapier's per-tick
    /// rebuild; nothing on physics-engine).
    pub rebuild: Duration,
    /// Static boxes (walls, pillars, locked doors) at the end of the run.
    pub fixed: usize,
    /// Players at the end of the run.
    pub players: usize,
    /// Most monsters with a physics body in any tick.
    pub peak_monster_bodies: usize,
}

fn monster_bodies(game: &ArpgGame) -> usize {
    game.monsters
        .iter()
        .filter(|monster| {
            game.physics
                .has_character(ArpgGame::monster_body_id(monster.id))
        })
        .count()
}

/// Runs `workload` with `seed` for `ticks` ticks on `backend`, timing every physics call.
pub fn time_workload(backend: Backend, workload: &str, seed: u32, ticks: u32) -> Timing {
    let case = case(workload);
    with_backend(backend, || {
        take_physics_time();
        time_physics(true);
        let mut game = fixture(case, seed);
        let setup = take_physics_time();
        take_rebuild_time();
        let mut timing = Timing {
            setup,
            ticks: Vec::with_capacity(ticks as usize),
            rebuild: Duration::ZERO,
            fixed: 0,
            players: 0,
            peak_monster_bodies: 0,
        };
        for tick in 0..ticks {
            commands(&mut game, case, tick);
            game.advance_tick().expect("workload tick");
            timing.ticks.push(take_physics_time());
            time_physics(false);
            timing.peak_monster_bodies = timing.peak_monster_bodies.max(monster_bodies(&game));
            time_physics(true);
        }
        time_physics(false);
        take_physics_time();
        timing.rebuild = take_rebuild_time();
        timing.fixed = game.physics.fixed_boxes().count();
        timing.players = game.players.len();
        timing
    })
}

#[cfg(test)]
mod tests;
