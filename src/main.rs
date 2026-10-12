mod chat;
mod config;
mod db;
mod llm;
mod names;
mod scheduler;

use axum::{
    Router,
    extract::{
        State,
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use config::Config;
use llm::LlmClient;
use scheduler::SchedulerHandle;
use scheduler::{Joined, Snapshot};
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;
use tracing::Instrument;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    scheduler: SchedulerHandle,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    Submit { prompt: String },
    Cancel { request_id: Uuid },
}

#[derive(serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Greeting {
    Welcome { author: String },
    History(Snapshot),
}

async fn send_json<T: serde::Serialize>(
    socket: &mut WebSocket,
    value: &T,
) -> Result<(), axum::Error> {
    let json = serde_json::to_string(value).expect("value always serialises");
    socket.send(WsMessage::Text(json.into())).await
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let connection_id = Uuid::new_v4();
    let author = names::generate_name();
    let span = tracing::info_span!("connection", %connection_id, %author);

    async {
        tracing::info!("client connected");
        serve_connection(socket, &state, connection_id, &author).await;
        state.scheduler.disconnect(connection_id).await;
        tracing::info!("client disconnected");
    }
    .instrument(span)
    .await;
}

async fn serve_connection(mut socket: WebSocket, state: &AppState, me: Uuid, author: &str) {
    if send_json(
        &mut socket,
        &Greeting::Welcome {
            author: author.to_owned(),
        },
    )
    .await
    .is_err()
    {
        return;
    }

    let Some(Joined {
        mut events,
        snapshot,
    }) = state.scheduler.join().await
    else {
        return;
    };
    if send_json(&mut socket, &Greeting::History(snapshot))
        .await
        .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
                Some(Ok(WsMessage::Text(text))) => {
                    match serde_json::from_str::<ClientMessage>(text.as_str()) {
                        Ok(ClientMessage::Submit { prompt }) => {
                            if state.scheduler.submit(me, author.to_owned(), prompt).await.is_none() {
                                return;
                            }
                        }
                        Ok(ClientMessage::Cancel { request_id }) => {
                            state.scheduler.cancel(me, request_id).await;
                        }
                        Err(e) => tracing::warn!("failed to parse client message: {e}"),
                    }
                }
                Some(Ok(_)) => {}
                _ => return,
            },

            event = events.recv() => match event {
                Ok(event) => {
                    if send_json(&mut socket, &event).await.is_err() {
                        return;
                    }
                }
                Err(RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "client too slow, events dropped");
                }
                Err(RecvError::Closed) => return,
            },
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env()?;
    tracing::info!(llama_url = %config.llama_url, bind = %config.bind_addr, "starting");

    let pool = db::connect(&config.database_url).await?;
    let scheduler = scheduler::spawn(
        LlmClient::new(&config.llama_url),
        pool.clone(),
        config.prompt_char_budget,
    );
    let state = AppState { scheduler };

    let app = Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/ws", get(ws_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
