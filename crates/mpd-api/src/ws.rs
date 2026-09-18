//! One-way WebSocket: server pushes player snapshots and database notices.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use serde_json::json;

use mpd_client::{MpdEvent, Snapshot};

use crate::AppState;

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let mut events = state.client.events();

    let initial = state.client.snapshot();
    if let Some(message) = encode_snapshot(&initial) {
        if send(&mut socket, message).await.is_err() {
            return;
        }
    }

    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(event) => {
                    if let Some(message) = encode_event(&event) {
                        if send(&mut socket, message).await.is_err() {
                            return;
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None => return,
                Some(Ok(Message::Ping(payload))) => {
                    if socket.send(Message::Pong(payload)).await.is_err() {
                        return;
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(_)) => return,
            },
        }
    }
}

async fn send(socket: &mut WebSocket, value: serde_json::Value) -> Result<(), ()> {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .map_err(|_| ())
}

pub fn encode_snapshot(snapshot: &Snapshot) -> Option<serde_json::Value> {
    Some(json!({
        "type": "status",
        "snapshot": snapshot,
    }))
}

pub fn encode_event(event: &MpdEvent) -> Option<serde_json::Value> {
    match event {
        MpdEvent::Snapshot(snapshot) => Some(json!({
            "type": "status",
            "snapshot": **snapshot,
        })),
        MpdEvent::DatabaseChanged => Some(json!({ "type": "database-changed" })),
        MpdEvent::Disconnected => Some(json!({ "type": "disconnected" })),
        MpdEvent::Reconnected => Some(json!({ "type": "reconnected" })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpd_client::Status;

    #[test]
    fn encodes_snapshot_events() {
        let snapshot = Snapshot {
            status: Status {
                state: mpd_client::PlayState::Play,
                ..Default::default()
            },
            song: None,
        };
        let value = encode_snapshot(&snapshot).expect("value");
        assert_eq!(value["type"], "status");
        assert_eq!(value["snapshot"]["status"]["state"], "play");

        let value = encode_event(&MpdEvent::Snapshot(Box::new(snapshot))).expect("value");
        assert_eq!(value["type"], "status");

        assert_eq!(
            encode_event(&MpdEvent::DatabaseChanged).unwrap()["type"],
            "database-changed"
        );
        assert_eq!(
            encode_event(&MpdEvent::Disconnected).unwrap()["type"],
            "disconnected"
        );
        assert_eq!(
            encode_event(&MpdEvent::Reconnected).unwrap()["type"],
            "reconnected"
        );
    }
}
