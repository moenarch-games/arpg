//! Acceptance for the wire encoding of aimed one-shots (#156).
//!
//! Contract (arpg-protocol, JSON envelope `{"protocolVersion": N, "payload": command}`):
//!
//! - `primaryAttack`, `secondaryAttack` and `releaseBow` take an optional `"aim": [x, z]`
//!   with integer components. The field may be absent or `null`; both decode to
//!   `aim: None`, so an unaimed command keeps today's meaning. `ArpgCommand { aim: None }`
//!   encodes as `"aim":null`, the way `setAim` encodes a cleared direction.
//! - The decoder checks the aim's shape the way it checks `setAim`'s `direction`: two
//!   integers that fit `i16`. Anything else is a decode error. The aim limit
//!   (±`AIM_COMPONENT_LIMIT`, non-zero) is a gameplay rule. The core enforces it on apply
//!   for both commands alike (see arpg-core `tests/aimed_one_shots.rs`). The decoder does not.
//! - The command encoding changes, so `PROTOCOL_VERSION` goes from 19 to 20, and an
//!   envelope with version 19 is refused. The browser's `web/src/wire-protocol.js` mirrors
//!   the version, and this test keeps the two in sync.
//! - `drawBow`, `cancelBow` and `interact` are unchanged unit commands.
//!
//! This file does not compile until the core's variants carry `aim`. That is expected.

use arpg_core::{AIM_COMPONENT_LIMIT, ArpgCommand, ArpgSnapshot};
use arpg_protocol::{JsonProtocol, PROTOCOL_VERSION, WireProtocol};

/// The version before aimed one-shots.
const PREVIOUS_PROTOCOL_VERSION: u16 = 19;

fn protocol() -> impl WireProtocol<ArpgCommand, ArpgSnapshot> {
    JsonProtocol
}

fn envelope(version: u16, payload: &str) -> Vec<u8> {
    format!(r#"{{"protocolVersion":{version},"payload":{payload}}}"#).into_bytes()
}

fn decode(payload: &str) -> Result<ArpgCommand, String> {
    protocol()
        .decode_command(&envelope(PROTOCOL_VERSION, payload))
        .map_err(|error| error.message().to_owned())
}

fn aimed(name: &str, aim: Option<[i16; 2]>) -> ArpgCommand {
    match name {
        "primaryAttack" => ArpgCommand::PrimaryAttack { aim },
        "secondaryAttack" => ArpgCommand::SecondaryAttack { aim },
        "releaseBow" => ArpgCommand::ReleaseBow { aim },
        other => panic!("not an aimed one-shot: {other}"),
    }
}

const AIMED: [&str; 3] = ["primaryAttack", "secondaryAttack", "releaseBow"];

#[test]
fn the_protocol_version_is_bumped_for_aimed_one_shots() {
    // Version 20 introduced aimed one-shots; later versions (21: jump, #59) keep them.
    const { assert!(PROTOCOL_VERSION > PREVIOUS_PROTOCOL_VERSION) };
    let bytes = protocol()
        .encode_command(&ArpgCommand::ReleaseBow {
            aim: Some([1000, 0]),
        })
        .unwrap();
    assert!(
        String::from_utf8(bytes)
            .unwrap()
            .starts_with(&format!(r#"{{"protocolVersion":{PROTOCOL_VERSION},"#))
    );
    // A client still speaking the previous version is refused, aimed or not.
    for payload in [
        r#"{"type":"releaseBow"}"#,
        r#"{"type":"releaseBow","aim":[1000,0]}"#,
    ] {
        assert!(
            protocol()
                .decode_command(&envelope(PREVIOUS_PROTOCOL_VERSION, payload))
                .is_err(),
            "{payload}"
        );
    }
}

#[test]
fn the_browser_wire_protocol_speaks_the_same_version() {
    let browser = include_str!("../../../web/src/wire-protocol.js");
    assert!(
        browser.contains(&format!(
            "export const PROTOCOL_VERSION = {PROTOCOL_VERSION};"
        )),
        "web/src/wire-protocol.js must export PROTOCOL_VERSION = {PROTOCOL_VERSION}"
    );
    assert!(
        browser.contains(&format!(
            "export const AIM_COMPONENT_LIMIT = {AIM_COMPONENT_LIMIT};"
        )),
        "web/src/wire-protocol.js must mirror AIM_COMPONENT_LIMIT"
    );
}

#[test]
fn aimed_one_shots_round_trip_with_their_aim() {
    for name in AIMED {
        for (aim, json) in [
            ([-250, 1000], "[-250,1000]"),
            ([1000, 0], "[1000,0]"),
            ([0, -1], "[0,-1]"),
        ] {
            let command = aimed(name, Some(aim));
            let bytes = protocol().encode_command(&command).unwrap();
            let encoded = String::from_utf8(bytes.clone()).unwrap();
            assert!(
                encoded.contains(&format!(r#""type":"{name}","aim":{json}"#)),
                "{encoded}"
            );
            assert_eq!(protocol().decode_command(&bytes).unwrap(), command);
        }
    }
}

#[test]
fn unaimed_one_shots_round_trip_and_an_absent_aim_still_decodes() {
    for name in AIMED {
        let command = aimed(name, None);
        let bytes = protocol().encode_command(&command).unwrap();
        let encoded = String::from_utf8(bytes.clone()).unwrap();
        assert!(
            encoded.contains(&format!(r#""type":"{name}","aim":null"#)),
            "{encoded}"
        );
        assert_eq!(protocol().decode_command(&bytes).unwrap(), command);

        // Clients that send no aim (and the browser before it fills the field) still decode.
        assert_eq!(decode(&format!(r#"{{"type":"{name}"}}"#)), Ok(command));
        assert_eq!(
            decode(&format!(r#"{{"type":"{name}","aim":null}}"#)),
            Ok(command)
        );
    }
}

#[test]
fn a_malformed_aim_is_rejected_at_decode_like_set_aim() {
    for value in [
        "[40000,0]",
        "[0,-40000]",
        "[0.5,1]",
        "[1]",
        "[1,2,3]",
        "\"north\"",
        "{\"x\":1,\"z\":0}",
    ] {
        assert!(
            decode(&format!(r#"{{"type":"setAim","direction":{value}}}"#)).is_err(),
            "fixture: setAim rejects {value}"
        );
        for name in AIMED {
            assert!(
                decode(&format!(r#"{{"type":"{name}","aim":{value}}}"#)).is_err(),
                "{name} with aim {value} must not decode"
            );
        }
    }
}

/// Range and non-zero checks belong to the core, for `setAim` and aimed one-shots alike:
/// whatever `setAim` lets through to the core, the aimed one-shots let through too.
#[test]
fn the_aim_limit_is_left_to_the_core_like_set_aim() {
    let over = AIM_COMPONENT_LIMIT + 1;
    for aim in [[over, 0], [0, 0], [i16::MIN, i16::MAX]] {
        let json = format!("[{},{}]", aim[0], aim[1]);
        assert_eq!(
            decode(&format!(r#"{{"type":"setAim","direction":{json}}}"#)),
            Ok(ArpgCommand::SetAim {
                direction: Some(aim)
            }),
            "fixture"
        );
        for name in AIMED {
            assert_eq!(
                decode(&format!(r#"{{"type":"{name}","aim":{json}}}"#)),
                Ok(aimed(name, Some(aim))),
                "{name} {json}"
            );
        }
    }
}

#[test]
fn unaimed_commands_keep_their_encoding() {
    for (command, json) in [
        (ArpgCommand::DrawBow, r#"{"type":"drawBow"}"#),
        (ArpgCommand::CancelBow, r#"{"type":"cancelBow"}"#),
        (ArpgCommand::Interact, r#"{"type":"interact"}"#),
    ] {
        let bytes = protocol().encode_command(&command).unwrap();
        assert!(
            String::from_utf8(bytes.clone())
                .unwrap()
                .ends_with(&format!(r#""payload":{json}}}"#))
        );
        assert_eq!(decode(json), Ok(command));
    }
}
