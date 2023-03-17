use axum::{
    body::Bytes,
    extract::{
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde::Serialize;
use std::{net::SocketAddr, sync::Arc};

use spin_message_types::OutputMessage;

use crate::{broker::MessageBroker, WebsocketConfig};

#[derive(Clone)]
struct GatewayState {
    broker: Arc<dyn MessageBroker>,
    websockets: Option<WebsocketConfig>,
}

pub async fn spawn_gateway(
    port: u16,
    websockets: Option<WebsocketConfig>,
    broker: Arc<dyn MessageBroker>,
) {
    let app = Router::new()
        .route("/publish/*subject", post(publish))
        .route("/subscribe/*subject", get(subscribe))
        .with_state(Arc::new(GatewayState { broker, websockets }));

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("Listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn publish(
    Path(subject): Path<String>,
    State(state): State<Arc<GatewayState>>,
    body: Bytes,
) -> impl IntoResponse {
    let broker = &state.broker;
    match broker
        .publish(OutputMessage {
            subject: Some(subject),
            message: body.to_vec(),
            broker: None,
        })
        .await
    {
        Ok(_) => (StatusCode::ACCEPTED, "published to subject"),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "couldn't publish"),
    }
}

async fn subscribe(
    Path(subject): Path<String>,
    State(state): State<Arc<GatewayState>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    println!("Setting up upgrade");
    let websockets = state.websockets.clone();

    if let Some(websockets) = websockets {
        ws.on_upgrade(move |socket| {
            handle_websocket(socket, subject, state.broker.clone(), websockets)
        })
        .into_response()
    } else {
        (StatusCode::BAD_REQUEST, "Websockets aren't supported").into_response()
    }
}

async fn handle_websocket(
    mut socket: WebSocket,
    subject: String,
    broker: Arc<dyn MessageBroker>,
    websockets: WebsocketConfig,
) {
    println!("upgraded");
    if let Ok(mut result) = broker.subscribe(&subject).await {
        println!("subscribed to {subject}");
        while let Ok(message) = result.recv().await {
            println!("socket subscription message recieved");
            match websockets {
                WebsocketConfig::BinaryBody => {
                    let _ = socket.send(WsMessage::Binary(message.message)).await;
                    println!("socket subscription message sent");
                }
                WebsocketConfig::TextBody => {
                    if let Ok(body) = std::str::from_utf8(&message.message) {
                        let _ = socket.send(WsMessage::Text(body.to_string())).await;
                        println!("socket subscription message sent");
                    }
                }
                WebsocketConfig::Messagepack => {
                    let mut buf = Vec::new();
                    if let Ok(()) = message.serialize(&mut rmp_serde::Serializer::new(&mut buf)) {
                        let _ = socket.send(WsMessage::Binary(buf)).await;
                        println!("socket subscription messagepack sent");
                    }
                }
                WebsocketConfig::Json => {
                    if let Ok(json) = serde_json::to_string(&message) {
                        let _ = socket.send(WsMessage::Text(json)).await;
                        println!("socket subscription message json sent");
                    }
                }
            }
        }
    }
}
