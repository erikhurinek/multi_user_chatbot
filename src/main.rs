mod config;
mod llm;
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
use scheduler::{Event, SchedulerHandle};
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;
use tracing::Instrument;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    Submit { prompt: String },
    Cancel { request_id: Uuid },
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(scheduler): State<SchedulerHandle>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, scheduler))
}

async fn send_event(socket: &mut WebSocket, event: &Event) -> Result<(), axum::Error> {
    let json = serde_json::to_string(event).expect("Event always serialises");
    socket.send(WsMessage::Text(json.into())).await
}

async fn handle_socket(socket: WebSocket, scheduler: SchedulerHandle) {
    let connection_id = Uuid::new_v4();
    let span = tracing::info_span!("connection", %connection_id);

    async {
        tracing::info!("client connected");
        serve_connection(socket, &scheduler, connection_id).await;
        scheduler.disconnect(connection_id).await;
        tracing::info!("client disconnected");
    }
    .instrument(span)
    .await;
}

async fn serve_connection(mut socket: WebSocket, scheduler: &SchedulerHandle, me: Uuid) {
    let mut events = scheduler.subscribe();

    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
                Some(Ok(WsMessage::Text(text))) => {
                    match serde_json::from_str::<ClientMessage>(text.as_str()) {
                        Ok(ClientMessage::Submit { prompt }) => {
                            if scheduler.submit(me, prompt).await.is_none() {
                                return;
                            }
                        }
                        Ok(ClientMessage::Cancel { request_id }) => {
                            scheduler.cancel(me, request_id).await;
                        }
                        Err(e) => tracing::warn!("failed to parse client message: {e}"),
                    }
                }
                Some(Ok(_)) => {}
                _ => return,
            },

            event = events.recv() => match event {
                Ok(event) => {
                    if send_event(&mut socket, &event).await.is_err() {
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

    let scheduler = scheduler::spawn(LlmClient::new(&config.llama_url));

    let app = Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/ws", get(ws_handler))
        .with_state(scheduler);

    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
