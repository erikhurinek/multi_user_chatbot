mod llm;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::header,
    response::IntoResponse,
    routing::{get, post},
};
use futures_util::StreamExt;
use llm::{LlmClient, Message, Role};
use serde::Deserialize;
use serde::Serialize;

#[derive(Deserialize)]
struct ChatBody {
    prompt: String,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Event {
    Token { text: String },
    Finished,
    Failed { reason: String },
}

async fn ws_handler(ws: WebSocketUpgrade, State(llm): State<LlmClient>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, llm))
}

async fn send_event(socket: &mut WebSocket, event: &Event) -> Result<(), axum::Error> {
    let json = serde_json::to_string(event).expect("Event always serialises");
    socket.send(WsMessage::Text(json.into())).await
}

async fn handle_socket(mut socket: WebSocket, llm: LlmClient) {
    while let Some(Ok(msg)) = socket.recv().await {
        let WsMessage::Text(prompt) = msg else {
            continue;
        };

        let messages = vec![Message {
            role: Role::User,
            content: prompt.as_str().to_owned(),
        }];
        let mut tokens = llm.stream_chat(messages);

        let outcome = loop {
            match tokens.next().await {
                Some(Ok(text)) => {
                    if send_event(&mut socket, &Event::Token { text })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                Some(Err(e)) => {
                    break Event::Failed {
                        reason: e.to_string(),
                    };
                }
                None => break Event::Finished,
            }
        };

        if send_event(&mut socket, &outcome).await.is_err() {
            return;
        }
    }
}

async fn chat(State(llm): State<LlmClient>, Json(body): Json<ChatBody>) -> impl IntoResponse {
    let messages = vec![Message {
        role: Role::User,
        content: body.prompt,
    }];

    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        Body::from_stream(llm.stream_chat(messages)),
    )
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let llm = LlmClient::new("http://localhost:8080");

    let app = Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/chat", post(chat))
        .route("/ws", get(ws_handler))
        .with_state(llm);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app).await?;

    Ok(())
}
