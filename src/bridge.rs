//! # Bridged rooms
//!
//! Helpers for rooms that a bridge connects to another chat network, like the ones Beeper or
//! mautrix bridges create for WhatsApp, Signal or Google Messages chats.
use std::collections::BTreeSet;

use matrix_sdk::deserialized_responses::RawAnySyncOrStrippedState;
use matrix_sdk::ruma::events::StateEventType;
use serde::Deserialize;

use crate::prelude::*;

/// State event types that bridges use to describe the network a room is bridged to.
///
/// `m.bridge` comes from [MSC2346](https://github.com/matrix-org/matrix-spec-proposals/pull/2346),
/// and older bridges send the same content as `uk.half-shot.bridge`.
const BRIDGE_EVENT_TYPES: [&str; 2] = ["m.bridge", "uk.half-shot.bridge"];

#[derive(Deserialize)]
struct BridgeEvent {
    content: BridgeEventContent,
}

#[derive(Deserialize)]
struct BridgeEventContent {
    protocol: BridgeProtocol,
}

#[derive(Deserialize)]
struct BridgeProtocol {
    id: String,
    displayname: Option<String>,
}

/// Get the network name from the JSON of a bridge state event, preferring the protocol's display
/// name ("WhatsApp") over its identifier ("whatsapp").
fn network_from_event(json: &str) -> Option<String> {
    let event: BridgeEvent = serde_json::from_str(json).ok()?;
    let BridgeProtocol { id, displayname } = event.content.protocol;

    displayname
        .filter(|name| !name.trim().is_empty())
        .or_else(|| Some(id).filter(|id| !id.trim().is_empty()))
}

/// Look up the name of the network a room is bridged to, if it has bridge state.
pub async fn room_network(room: &MatrixRoom) -> Option<String> {
    for event_type in BRIDGE_EVENT_TYPES {
        let Ok(events) = room.get_state_events(StateEventType::from(event_type)).await else {
            continue;
        };

        for event in events {
            let json = match &event {
                RawAnySyncOrStrippedState::Sync(raw) => raw.json().get(),
                RawAnySyncOrStrippedState::Stripped(raw) => raw.json().get(),
            };

            if let Some(network) = network_from_event(json) {
                return Some(network);
            }
        }
    }

    None
}

/// Whether a state event was sent by, or is about, one of a room's service members.
///
/// Service members are listed in a room's `io.element.functional_members` state, and are usually
/// bridge bots. Checking the state key catches membership changes for the bot that someone else
/// sent, like the bot getting invited to the room.
pub fn is_service_member_event(
    sender: &UserId,
    state_key: &str,
    service_members: &BTreeSet<OwnedUserId>,
) -> bool {
    service_members
        .iter()
        .any(|member| member.as_str() == sender.as_str() || member.as_str() == state_key)
}

#[cfg(test)]
mod tests {
    use matrix_sdk::ruma::user_id;

    use super::*;

    #[test]
    fn test_network_from_event() {
        let whatsapp = r#"{
            "type": "m.bridge",
            "state_key": "fi.mau.whatsapp://whatsapp/123",
            "content": {
                "bridgebot": "@whatsappbot:beeper.local",
                "protocol": {"id": "whatsapp", "displayname": "WhatsApp"},
                "channel": {"id": "123"}
            }
        }"#;
        assert_eq!(network_from_event(whatsapp).as_deref(), Some("WhatsApp"));

        let no_displayname = r#"{"content": {"protocol": {"id": "gmessages"}}}"#;
        assert_eq!(network_from_event(no_displayname).as_deref(), Some("gmessages"));

        let blank = r#"{"content": {"protocol": {"id": "linkedin", "displayname": " "}}}"#;
        assert_eq!(network_from_event(blank).as_deref(), Some("linkedin"));

        // Redacted bridge events have no content left to read.
        let redacted = r#"{"content": {}}"#;
        assert_eq!(network_from_event(redacted), None);
    }

    #[test]
    fn test_is_service_member_event() {
        let bot = user_id!("@whatsappbot:beeper.local");
        let me = user_id!("@gitbranch:beeper.com");
        let ghost = user_id!("@whatsapp_123:beeper.local");
        let service_members = BTreeSet::from([bot.to_owned()]);

        // The bot changing the room name.
        assert!(is_service_member_event(bot, "", &service_members));

        // Someone else inviting the bot.
        assert!(is_service_member_event(me, bot.as_str(), &service_members));

        // The contact's ghost joining.
        assert!(!is_service_member_event(ghost, ghost.as_str(), &service_members));

        // Rooms without functional members hide nothing.
        assert!(!is_service_member_event(bot, "", &BTreeSet::new()));
    }
}
