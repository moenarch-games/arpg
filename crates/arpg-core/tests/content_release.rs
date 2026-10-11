//! Acceptance for #154: custom content is freed with the last game that runs it.
//!
//! The workspace forbids `unsafe` code, so this black-box check cannot install a counting
//! `#[global_allocator]`; it observes the process's resident memory (`VmRSS`, Linux only)
//! instead. Each bundle is made sizeable by data alone (a long `roomEncounters` list of
//! 64-character ids, at least `PAYLOAD` bytes of strings), so retaining bundles shows as
//! growth proportional to their number, while freed bundles are reused by the allocator and
//! resident memory stays flat. A control phase that parses, validates and drops the same
//! bundles next to built-in games calibrates what "flat" means on the running machine.
//!
//! This is its own test binary with a single test, so no other test allocates while it
//! measures. See `tests/content_ownership.rs` for the full contract.

#[cfg(target_os = "linux")]
mod linux {
    use arpg_core::{ArpgGame, AuthoritativeGame, ContentBundle, base_bundle};
    use serde_json::{Value, json};

    /// Entries in each sizeable bundle's `roomEncounters`.
    const ENTRIES: usize = 16_384;
    /// The longest id content allows.
    const ID_LENGTH: usize = 64;
    /// Lower bound of the string bytes one sizeable bundle holds.
    const PAYLOAD: usize = ENTRIES * ID_LENGTH;
    /// Distinct bundles measured per phase. Kept below 16 so that today's process registry
    /// does not refuse them: the measured property is retention, not capacity.
    const CYCLES: u32 = 12;

    fn resident_bytes() -> usize {
        let status = std::fs::read_to_string("/proc/self/status").expect("proc status");
        let line = status
            .lines()
            .find(|line| line.starts_with("VmRSS:"))
            .expect("VmRSS line");
        let kib: usize = line
            .split_whitespace()
            .nth(1)
            .expect("VmRSS value")
            .parse()
            .expect("VmRSS number");
        kib * 1024
    }

    /// A valid bundle distinct by `gold` whose combat rooms cycle through [`ENTRIES`]
    /// references to an extra encounter with a maximal-length id.
    fn sizeable(gold: u32) -> ContentBundle {
        let mut value: Value = serde_json::to_value(base_bundle()).expect("base serialises");
        for reward in value["rewards"].as_array_mut().expect("rewards") {
            reward["gold"] = json!(gold);
        }
        let id = format!("encounter.{}", "x".repeat(ID_LENGTH - "encounter.".len()));
        assert_eq!(id.len(), ID_LENGTH);
        let mut extra = value["encounters"][0].clone();
        extra["id"] = json!(id);
        value["encounters"]
            .as_array_mut()
            .expect("encounters")
            .push(extra);
        let rooms = value["roomEncounters"]
            .as_array_mut()
            .expect("roomEncounters");
        rooms.extend(std::iter::repeat_n(json!(id), ENTRIES));
        let bundle = ContentBundle::from_json(&value.to_string()).expect("sizeable parses");
        bundle.check().expect("sizeable bundle is valid");
        bundle
    }

    fn run_and_drop(gold: u32) {
        let bundle = sizeable(gold);
        let revision = bundle.revision();
        let mut game = ArpgGame::new_with_content(bundle, 5)
            .unwrap_or_else(|error| panic!("bundle {gold} is refused: {}", error.message()));
        game.add_player(1).expect("player joins");
        for _ in 0..3 {
            game.advance_tick().expect("tick");
        }
        let save = game.save_state().expect("save");
        assert_eq!(save.content_revision, revision);
        drop(game);
    }

    /// The control: the same sizeable bundle parsed, validated and dropped next to a
    /// built-in game that runs the same steps, so everything but custom content ownership
    /// is allocated and freed alike.
    fn check_and_run_built_in(gold: u32) {
        let bundle = sizeable(gold);
        assert!(!bundle.revision().is_empty());
        let mut game = ArpgGame::new_with_seed(5).expect("built-in game");
        game.add_player(1).expect("player joins");
        for _ in 0..3 {
            game.advance_tick().expect("tick");
        }
        game.save_state().expect("save");
        drop(game);
        drop(bundle);
    }

    /// Resident growth over [`CYCLES`] runs of `step` with distinct golds after one warm-up.
    fn growth(first_gold: u32, step: fn(u32)) -> usize {
        step(first_gold);
        let before = resident_bytes();
        for gold in first_gold + 1..=first_gold + CYCLES {
            step(gold);
        }
        resident_bytes().saturating_sub(before)
    }

    #[test]
    fn dropping_games_releases_their_content() {
        // Bounded growth the allocator may show without retaining anything per bundle.
        let budget = 3 * PAYLOAD;
        // Calibration: bundles that are only parsed, validated and dropped, next to built-in
        // games.
        let control = growth(1_000, check_and_run_built_in);
        assert!(
            control < budget,
            "resident memory is not stable enough to measure: control grew {control} bytes"
        );
        // Games built and dropped, each with its own distinct sizeable bundle. Retaining
        // each bundle would grow by at least CYCLES * PAYLOAD.
        let games = growth(2_000, run_and_drop);
        assert!(
            games < budget,
            "{CYCLES} dropped games with distinct {PAYLOAD}-byte bundles left {games} bytes \
             resident (control {control}); content must be freed with its last game"
        );
    }
}
