//! Checked-in save fixtures and the save version policy (#68).
//!
//! Policy: a save loads only under the exact `SAVE_STATE_SCHEMA_VERSION`,
//! `SAVE_STATE_RULES_VERSION` and content revision that wrote it. Before 1.0 nothing is
//! migrated. Every other version is refused with an error, and the refusal never replaces
//! the game already running.
//!
//! `tests/fixtures/saves/` holds:
//! - `current.json`: a mid-run, mid-jump save of the current schema. It must load, and
//!   saving the loaded game must give the same document back.
//! - `previous-schema.json`: the save the schema before this one wrote. It is refused by
//!   its version.
//! - `corrupt.json`: the current schema with a broken invariant (a checkpoint room that
//!   does not exist). It is refused by validation.
//! - `partial.json`: `current.json` cut off halfway. It is refused as malformed.
//!
//! A schema or rules bump makes `current.json` fail to load. Regenerate the fixtures with
//! `UPDATE_SAVE_FIXTURES=1 cargo test -p arpg-core --test save_fixtures`. That moves the
//! outdated `current.json` to `previous-schema.json` and writes the others, so the format
//! change shows up as a fixture diff in review. The browser workflow test
//! `web/tests/browser/savestate.spec.js` imports the same files through the Wasm load.

use std::{fs, path::PathBuf};

use arpg_core::{
    ArpgCommand, ArpgGame, ArpgSaveState, AuthoritativeGame, PlayerCommand,
    SAVE_STATE_RULES_VERSION, SAVE_STATE_SCHEMA_VERSION,
};
use serde_json::Value;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/saves")
}

fn read(name: &str) -> String {
    fs::read_to_string(fixtures().join(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn write(name: &str, text: &str) {
    fs::write(fixtures().join(name), text).unwrap_or_else(|error| panic!("{name}: {error}"));
}

fn pretty(value: &Value) -> String {
    format!("{}\n", serde_json::to_string_pretty(value).unwrap())
}

/// Seed 42, one player walking for 40 ticks and then 10 ticks into a jump.
fn current_save() -> ArpgSaveState {
    let mut game = ArpgGame::new_with_seed(42).unwrap();
    game.add_player(1).unwrap();
    let mut sequence = 0;
    let mut send = |game: &mut ArpgGame, command| {
        sequence += 1;
        game.apply_command(PlayerCommand::new(1, sequence, command).unwrap())
            .unwrap();
    };
    send(&mut game, ArpgCommand::SetMovement { x: 1, z: 0 });
    for _ in 0..40 {
        game.advance_tick().unwrap();
    }
    send(&mut game, ArpgCommand::Jump);
    for _ in 0..10 {
        game.advance_tick().unwrap();
    }
    let save = game.save_state().unwrap();
    assert!(
        !game.snapshot().unwrap().players[0].motion.grounded,
        "the fixture is saved mid-jump"
    );
    save
}

fn regenerate() {
    fs::create_dir_all(fixtures()).unwrap();
    let outdated = fs::read_to_string(fixtures().join("current.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(|old| {
            old["schemaVersion"] != SAVE_STATE_SCHEMA_VERSION
                || old["rulesVersion"] != SAVE_STATE_RULES_VERSION
        });
    if let Some(old) = outdated {
        write("previous-schema.json", &pretty(&old));
    }
    let current = serde_json::to_value(current_save()).unwrap();
    let text = pretty(&current);
    write("current.json", &text);
    let mut corrupt = current;
    corrupt["players"][0]["checkpointRoom"] = 9_999.into();
    write("corrupt.json", &pretty(&corrupt));
    write("partial.json", &text[..text.len() / 2]);
}

fn load(text: &str) -> Result<ArpgGame, String> {
    ArpgSaveState::from_json(text)
        .and_then(ArpgGame::from_save_state)
        .map_err(|error| error.message().to_owned())
}

#[test]
fn the_current_schema_fixture_loads_and_saves_back_unchanged() {
    if std::env::var_os("UPDATE_SAVE_FIXTURES").is_some() {
        regenerate();
    }
    let text = read("current.json");
    let game = load(&text).unwrap_or_else(|error| {
        panic!(
            "current.json no longer loads ({error}); regenerate it with \
             UPDATE_SAVE_FIXTURES=1 cargo test -p arpg-core --test save_fixtures"
        )
    });
    assert!(!game.snapshot().unwrap().players[0].motion.grounded);
    assert_eq!(
        game.save_state().unwrap(),
        ArpgSaveState::from_json(&text).unwrap(),
        "every saved field survives a load"
    );
}

#[test]
fn refused_fixtures_never_replace_the_running_game() {
    let mut running =
        ArpgGame::from_save_state(ArpgSaveState::from_json(&read("current.json")).unwrap())
            .unwrap();
    running.advance_tick().unwrap();
    let before = running.save_state().unwrap();

    let previous = read("previous-schema.json");
    let previous_version = serde_json::from_str::<Value>(&previous).unwrap()["schemaVersion"]
        .as_u64()
        .unwrap();
    assert!(previous_version < u64::from(SAVE_STATE_SCHEMA_VERSION));
    let expectations = [
        (
            "previous-schema.json",
            format!(
                "unsupported ARPG save schema version {previous_version}; expected {SAVE_STATE_SCHEMA_VERSION}"
            ),
        ),
        (
            "corrupt.json",
            "saved player checkpoint is not a cleared room".to_owned(),
        ),
        ("partial.json", "invalid ARPG save".to_owned()),
    ];
    for (name, expected) in expectations {
        let error = load(&read(name))
            .err()
            .unwrap_or_else(|| panic!("{name} loaded"));
        assert!(error.contains(&expected), "{name}: {error}");
    }
    assert_eq!(running.save_state().unwrap(), before);
    running.advance_tick().unwrap();
}
