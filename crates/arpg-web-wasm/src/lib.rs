#![forbid(unsafe_code)]

use arpg_core::{
    ArpgGame, ArpgSaveState, AuthoritativeGame, PlayerCommand, ReproductionRecorder, ScenarioId,
    WorkbenchOperation, content_revision,
};
use arpg_protocol::{JsonProtocol, WireProtocol};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct WasmGame {
    game: ArpgGame,
    protocol: JsonProtocol,
    // Workbench (scenario) sessions record their accepted inputs for reproduction export.
    // Ordinary, hosted and restored games record nothing.
    recorder: Option<ReproductionRecorder>,
}

#[wasm_bindgen]
impl WasmGame {
    #[wasm_bindgen(constructor)]
    pub fn new(run_seed: u32) -> Result<WasmGame, JsValue> {
        Ok(Self {
            game: ArpgGame::new_with_seed(run_seed).map_err(js_error)?,
            protocol: JsonProtocol,
            recorder: None,
        })
    }

    /// The generated dungeon for `run_seed` arranged as the named workbench scenario. The
    /// session records its accepted players, commands and ticks for `reproductionJson`.
    #[wasm_bindgen(js_name = newScenario)]
    pub fn new_scenario(scenario: &str, run_seed: u32) -> Result<WasmGame, JsValue> {
        let scenario = ScenarioId::parse(scenario)
            .ok_or_else(|| js_error(format!("unknown scenario {scenario:?}")))?;
        let game = ArpgGame::new_scenario(scenario, run_seed).map_err(js_error)?;
        let recorder = ReproductionRecorder::start(&game).map_err(js_error)?;
        Ok(Self {
            game,
            protocol: JsonProtocol,
            recorder: Some(recorder),
        })
    }

    #[wasm_bindgen(js_name = tickHz)]
    pub fn tick_hz(&self) -> u16 {
        self.game.tick_hz()
    }

    #[wasm_bindgen(js_name = addPlayer)]
    pub fn add_player(&mut self, player_id: u32) -> Result<(), JsValue> {
        self.game.add_player(player_id).map_err(js_error)?;
        if let Some(recorder) = &mut self.recorder {
            recorder.record_player_added(player_id);
        }
        Ok(())
    }

    #[wasm_bindgen(js_name = removePlayer)]
    pub fn remove_player(&mut self, player_id: u32) -> bool {
        let removed = self.game.remove_player(player_id);
        if removed && let Some(recorder) = &mut self.recorder {
            recorder.record_player_removed();
        }
        removed
    }

    #[wasm_bindgen(js_name = applyCommand)]
    pub fn apply_command(
        &mut self,
        player_id: u32,
        sequence: u32,
        encoded_command: &str,
    ) -> Result<(), JsValue> {
        let command = self
            .protocol
            .decode_command(encoded_command.as_bytes())
            .map_err(js_error)?;
        let command = PlayerCommand::new(player_id, sequence, command).map_err(js_error)?;
        self.game.apply_command(command.clone()).map_err(js_error)?;
        if let Some(recorder) = &mut self.recorder {
            recorder.record_command(&command);
        }
        Ok(())
    }

    #[wasm_bindgen(js_name = advanceTick)]
    pub fn advance_tick(&mut self) -> Result<(), JsValue> {
        self.game.advance_tick().map_err(js_error)?;
        if let Some(recorder) = &mut self.recorder {
            recorder.record_tick();
        }
        Ok(())
    }

    /// Applies one workbench operation (`WorkbenchOperation` JSON: spawn, remove, reset or
    /// exact tuning) and records it for the reproduction. Only workbench sessions accept
    /// them; an invalid operation fails with its validation message and changes nothing.
    #[wasm_bindgen(js_name = applyWorkbench)]
    pub fn apply_workbench(&mut self, encoded_operation: &str) -> Result<(), JsValue> {
        self.workbench_operation(encoded_operation)
            .map_err(js_error)
    }

    /// The room workbench operations arrange; `undefined` outside workbench sessions.
    #[wasm_bindgen(js_name = workbenchRoomId)]
    pub fn workbench_room_id(&self) -> Option<u32> {
        self.recorder.as_ref().and(self.game.workbench_room())
    }

    /// The tuning values the session runs, as `[{ parameter, value }]` JSON.
    #[wasm_bindgen(js_name = tuningJson)]
    pub fn tuning_json(&self) -> Result<String, JsValue> {
        serde_json::to_string(&self.game.tuning_values()).map_err(js_error)
    }

    /// The recorded workbench session as portable `Reproduction` JSON, which the native
    /// `replay_reproduction` runner replays. Fails for sessions that do not record.
    #[wasm_bindgen(js_name = reproductionJson)]
    pub fn reproduction_json(&self) -> Result<String, JsValue> {
        self.export_reproduction().map_err(js_error)
    }

    #[wasm_bindgen(js_name = snapshotJson)]
    pub fn snapshot_json(&self) -> Result<String, JsValue> {
        let snapshot = self.game.snapshot().map_err(js_error)?;
        let bytes = self.protocol.encode_snapshot(&snapshot).map_err(js_error)?;
        String::from_utf8(bytes).map_err(js_error)
    }

    #[wasm_bindgen(js_name = saveStateJson)]
    pub fn save_state_json(&self) -> Result<String, JsValue> {
        let save = self.game.save_state().map_err(js_error)?;
        serde_json::to_string(&save).map_err(js_error)
    }
}

/// Revision of the content bundle this build runs. Guests and dedicated clients compare it
/// with the authority's published `contentRevision` before playing.
#[wasm_bindgen(js_name = contentRevision)]
pub fn wasm_content_revision() -> String {
    content_revision().to_owned()
}

#[wasm_bindgen(js_name = loadGameFromSaveStateJson)]
pub fn load_game_from_save_state_json(encoded: &str) -> Result<WasmGame, JsValue> {
    let save = serde_json::from_str::<ArpgSaveState>(encoded).map_err(js_error)?;
    Ok(WasmGame {
        game: ArpgGame::from_save_state(save).map_err(js_error)?,
        protocol: JsonProtocol,
        recorder: None,
    })
}

impl WasmGame {
    fn workbench_operation(&mut self, encoded_operation: &str) -> Result<(), String> {
        let recorder = self
            .recorder
            .as_mut()
            .ok_or("only workbench sessions accept workbench operations")?;
        let operation = serde_json::from_str::<WorkbenchOperation>(encoded_operation)
            .map_err(|error| format!("invalid workbench operation: {error}"))?;
        self.game
            .apply_workbench(&operation)
            .map_err(|error| error.to_string())?;
        recorder.record_workbench(&operation);
        Ok(())
    }

    fn export_reproduction(&self) -> Result<String, String> {
        let recorder = self
            .recorder
            .as_ref()
            .ok_or("only workbench sessions record a reproduction")?;
        let reproduction = recorder.reproduction().map_err(|error| error.to_string())?;
        serde_json::to_string(&reproduction).map_err(|error| error.to_string())
    }
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arpg_core::{ArpgCommand, Reproduction, replay_reproduction};

    // The browser export path: wire-encoded commands through `WasmGame`, exported JSON
    // replayed by the native runner, every published snapshot compared.
    #[test]
    fn an_exported_workbench_session_replays_natively_to_the_same_snapshots() {
        let mut game = WasmGame::new_scenario("dummy", 42).unwrap();
        game.add_player(1).unwrap();
        let protocol = JsonProtocol;
        let encode = |command: ArpgCommand| {
            String::from_utf8(protocol.encode_command(&command).unwrap()).unwrap()
        };
        let script = [
            (0, ArpgCommand::SetMovement { x: 1, z: 0 }),
            (6, ArpgCommand::SetMovement { x: 0, z: 0 }),
            (7, ArpgCommand::PrimaryAttack { aim: None }),
            (30, ArpgCommand::SecondaryAttack { aim: None }),
            (31, ArpgCommand::SetGuard { raised: true }),
            (60, ArpgCommand::SetGuard { raised: false }),
        ];
        let mut sequence = 0;
        let mut live = Vec::new();
        for tick in 0..120 {
            for &(_, command) in script.iter().filter(|(at, _)| *at == tick) {
                sequence += 1;
                game.apply_command(1, sequence, &encode(command)).unwrap();
            }
            game.advance_tick().unwrap();
            live.push(game.game.snapshot().unwrap());
        }

        let exported = game.export_reproduction().unwrap();
        let reproduction: Reproduction = serde_json::from_str(&exported).unwrap();
        assert_eq!(reproduction.players, vec![1]);
        assert_eq!(reproduction.ticks, 120);
        assert_eq!(reproduction.commands.len(), script.len());
        let replayed = replay_reproduction(&reproduction).unwrap();
        assert!(
            replayed
                .iter()
                .any(|snapshot| !snapshot.strike_events.is_empty())
        );
        assert_eq!(replayed, live);
    }

    #[test]
    fn workbench_operations_are_recorded_and_replay_natively() {
        let mut game = WasmGame::new_scenario("enemy", 42).unwrap();
        game.add_player(1).unwrap();
        let protocol = JsonProtocol;
        let encode = |command: ArpgCommand| {
            String::from_utf8(protocol.encode_command(&command).unwrap()).unwrap()
        };
        game.workbench_operation(
            r#"{"type":"setTuning","parameter":"guard.maxPoints","value":30}"#,
        )
        .unwrap();
        let mut live = Vec::new();
        for tick in 0..90 {
            if tick == 3 {
                game.apply_command(1, 1, &encode(ArpgCommand::SetGuard { raised: true }))
                    .unwrap();
                game.workbench_operation(
                    r#"{"type":"spawnMonster","definition":"monster.skirmisher","offset":[-200,0]}"#,
                )
                .unwrap();
            }
            game.advance_tick().unwrap();
            live.push(game.game.snapshot().unwrap());
        }
        let error = game
            .workbench_operation(r#"{"type":"setTuning","parameter":"guard.maxPoints","value":0}"#)
            .unwrap_err();
        assert!(error.contains("guard.maxPoints"), "{error}");
        assert!(
            game.workbench_operation(r#"{"type":"launchMeteor"}"#)
                .is_err()
        );
        let tuning: serde_json::Value = serde_json::from_str(&game.tuning_json().unwrap()).unwrap();
        assert!(
            tuning
                .as_array()
                .unwrap()
                .contains(&serde_json::json!({ "parameter": "guard.maxPoints", "value": 30 }))
        );

        let reproduction: Reproduction =
            serde_json::from_str(&game.export_reproduction().unwrap()).unwrap();
        assert_eq!(reproduction.operations.len(), 2);
        assert_eq!(replay_reproduction(&reproduction).unwrap(), live);
        // An edited session is reproducible, not saveable.
        assert!(game.game.save_state().is_err());
    }

    #[test]
    fn only_workbench_sessions_accept_operations() {
        let mut game = WasmGame::new(42).unwrap();
        game.add_player(1).unwrap();
        let error = game
            .workbench_operation(r#"{"type":"resetArrangement"}"#)
            .unwrap_err();
        assert!(error.contains("only workbench sessions"), "{error}");
        assert_eq!(game.workbench_room_id(), None);
        assert!(
            WasmGame::new_scenario("ranged", 42)
                .unwrap()
                .workbench_room_id()
                .is_some()
        );
    }

    #[test]
    fn only_workbench_sessions_export_reproductions() {
        assert!(WasmGame::new(42).unwrap().export_reproduction().is_err());
        let mut game = WasmGame::new_scenario("dungeon", 42).unwrap();
        game.add_player(1).unwrap();
        game.advance_tick().unwrap();
        assert!(game.export_reproduction().is_ok());
        assert!(game.remove_player(1));
        assert!(game.export_reproduction().is_err());
    }
}
