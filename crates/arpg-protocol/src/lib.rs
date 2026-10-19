#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt;

use arpg_core::{ArpgCommand, ArpgSnapshot};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// 21: the `jump` command and the players' `motion` (height and support) in snapshots (#59).
pub const PROTOCOL_VERSION: u16 = 21;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolError {
    message: String,
}

impl ProtocolError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ProtocolError {}

/// Game-specific command/snapshot encoding independent of transport topology.
pub trait WireProtocol<Command, Snapshot>: Send + Sync + 'static {
    fn encode_command(&self, command: &Command) -> Result<Vec<u8>, ProtocolError>;
    fn decode_command(&self, payload: &[u8]) -> Result<Command, ProtocolError>;
    fn encode_snapshot(&self, snapshot: &Snapshot) -> Result<Vec<u8>, ProtocolError>;
    fn decode_snapshot(&self, payload: &[u8]) -> Result<Snapshot, ProtocolError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct JsonProtocol;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionedPayload<T> {
    protocol_version: u16,
    payload: T,
}

impl WireProtocol<ArpgCommand, ArpgSnapshot> for JsonProtocol {
    fn encode_command(&self, command: &ArpgCommand) -> Result<Vec<u8>, ProtocolError> {
        encode(command)
    }

    fn decode_command(&self, payload: &[u8]) -> Result<ArpgCommand, ProtocolError> {
        decode(payload)
    }

    fn encode_snapshot(&self, snapshot: &ArpgSnapshot) -> Result<Vec<u8>, ProtocolError> {
        encode(snapshot)
    }

    fn decode_snapshot(&self, payload: &[u8]) -> Result<ArpgSnapshot, ProtocolError> {
        decode(payload)
    }
}

fn encode<T: Serialize>(payload: &T) -> Result<Vec<u8>, ProtocolError> {
    serde_json::to_vec(&VersionedPayload {
        protocol_version: PROTOCOL_VERSION,
        payload,
    })
    .map_err(|error| ProtocolError::new(format!("encode failed: {error}")))
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ProtocolError> {
    let envelope: VersionedPayload<T> = serde_json::from_slice(bytes)
        .map_err(|error| ProtocolError::new(format!("decode failed: {error}")))?;
    if envelope.protocol_version != PROTOCOL_VERSION {
        return Err(ProtocolError::new(format!(
            "unsupported ARPG protocol version {}",
            envelope.protocol_version
        )));
    }
    Ok(envelope.payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arpg_core::{ArpgGame, AuthoritativeGame};

    #[test]
    fn command_round_trip_is_versioned() {
        let protocol = JsonProtocol;
        let command = ArpgCommand::SetMovement { x: -1, z: 1 };
        let bytes = protocol.encode_command(&command).unwrap();
        let encoded = String::from_utf8(bytes.clone()).unwrap();
        assert!(encoded.contains("\"protocolVersion\":21"));
        assert_eq!(protocol.decode_command(&bytes).unwrap(), command);

        let guard = ArpgCommand::SetGuard { raised: true };
        let bytes = protocol.encode_command(&guard).unwrap();
        assert!(
            String::from_utf8(bytes.clone())
                .unwrap()
                .contains("\"type\":\"setGuard\",\"raised\":true")
        );
        assert_eq!(protocol.decode_command(&bytes).unwrap(), guard);

        for (command, encoded) in [
            (
                ArpgCommand::SetAim {
                    direction: Some([-250, 1000]),
                },
                "\"type\":\"setAim\",\"direction\":[-250,1000]",
            ),
            (
                ArpgCommand::SetAim { direction: None },
                "\"type\":\"setAim\",\"direction\":null",
            ),
            (ArpgCommand::CycleTarget, "\"type\":\"cycleTarget\""),
            (ArpgCommand::ClearTarget, "\"type\":\"clearTarget\""),
            (ArpgCommand::Jump, "\"type\":\"jump\""),
        ] {
            let bytes = protocol.encode_command(&command).unwrap();
            assert!(String::from_utf8(bytes.clone()).unwrap().contains(encoded));
            assert_eq!(protocol.decode_command(&bytes).unwrap(), command);
        }
        // Aim components beyond i16 never reach the core.
        assert!(
            protocol
                .decode_command(
                    br#"{"protocolVersion":16,"payload":{"type":"setAim","direction":[40000,0]}}"#
                )
                .is_err()
        );
    }

    #[test]
    fn snapshot_round_trip_preserves_authoritative_run_seed() {
        let mut game = ArpgGame::new_with_seed(0xCAFE_BABE).unwrap();
        game.add_player(1).unwrap();
        let snapshot = game.snapshot().unwrap();
        let protocol = JsonProtocol;
        let bytes = protocol.encode_snapshot(&snapshot).unwrap();
        let decoded = protocol.decode_snapshot(&bytes).unwrap();
        assert_eq!(decoded, snapshot);
        assert_eq!(decoded.run_seed, 0xCAFE_BABE);
        assert_eq!(decoded.schema_version, 25);
    }

    #[test]
    fn jumps_and_player_motion_are_versioned_wire_data() {
        // A version-20 client predates `jump` and is refused.
        assert!(
            JsonProtocol
                .decode_command(br#"{"protocolVersion":20,"payload":{"type":"jump"}}"#)
                .is_err()
        );
        let mut game = ArpgGame::new_with_seed(5).unwrap();
        game.add_player(1).unwrap();
        game.apply_command(arpg_core::PlayerCommand::new(1, 1, ArpgCommand::Jump).unwrap())
            .unwrap();
        for _ in 0..4 {
            game.advance_tick().unwrap();
        }
        let bytes = JsonProtocol
            .encode_snapshot(&game.snapshot().unwrap())
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let motion = &value["payload"]["players"][0]["motion"];
        assert_eq!(motion["grounded"], false);
        assert!(motion["verticalVelocity"].as_i64().unwrap() > 0);
        assert!(motion["feetY"].as_i64().unwrap() > 0);
        assert_eq!(motion["groundY"], 0);
        assert_eq!(
            JsonProtocol.decode_snapshot(&bytes).unwrap(),
            game.snapshot().unwrap()
        );
    }

    #[test]
    fn monster_behaviour_and_target_are_published_in_camel_case() {
        let mut game = ArpgGame::new_scenario(arpg_core::ScenarioId::Dummy, 42).unwrap();
        game.add_player(1).unwrap();
        for _ in 0..3 {
            game.advance_tick().unwrap();
        }
        let snapshot = game.snapshot().unwrap();
        let bytes = JsonProtocol.encode_snapshot(&snapshot).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let monsters = value["payload"]["monsters"].as_array().unwrap();
        let chaser = monsters
            .iter()
            .find(|monster| monster["roomId"] == 2)
            .unwrap();
        assert_eq!(chaser["behavior"], "pursuing");
        assert_eq!(chaser["targetPlayerId"], 1);
        let dormant = monsters
            .iter()
            .find(|monster| monster["roomId"] == 3)
            .unwrap();
        assert_eq!(dormant["behavior"], "dormant");
        assert!(dormant["targetPlayerId"].is_null());
        // Shield guard (#92): only a shield monster carries guard points.
        assert!(dormant["guard"].is_null());
        assert_eq!(dormant["guardPoints"], 0);
        assert_eq!(dormant["maxGuardPoints"], 0);
        assert_eq!(JsonProtocol.decode_snapshot(&bytes).unwrap(), snapshot);
    }

    #[test]
    fn rooms_publish_their_encounter_definition() {
        let snapshot = ArpgGame::new_with_seed(7).unwrap().snapshot().unwrap();
        let bytes = JsonProtocol.encode_snapshot(&snapshot).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let rooms = value["payload"]["rooms"].as_array().unwrap();
        let start = rooms.iter().find(|room| room["kind"] == "start").unwrap();
        assert!(start["encounter"].is_null());
        let combat = rooms.iter().find(|room| room["id"] == 2).unwrap();
        assert_eq!(combat["encounter"], "encounter.brute");
        assert_eq!(JsonProtocol.decode_snapshot(&bytes).unwrap(), snapshot);
    }
}
