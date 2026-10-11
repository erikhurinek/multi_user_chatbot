use sqlx::SqlitePool;
use std::collections::VecDeque;

use futures_util::{StreamExt, stream::BoxStream};
use serde::Serialize;
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

use crate::chat;
use crate::db;
use crate::llm::{ChatBackend, LlmClient, LlmError, Message, Role};

const HISTORY_FETCH_LIMIT: i64 = 100;

pub enum Command {
    Submit {
        request_id: Uuid,
        owner: Uuid,
        author: String,
        prompt: String,
    },
    Cancel {
        request_id: Uuid,
        requester: Uuid,
    },
    Disconnect {
        owner: Uuid,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Queued {
        request_id: Uuid,
        position: usize,
    },
    PositionChanged {
        request_id: Uuid,
        position: usize,
    },
    Started {
        request_id: Uuid,
    },
    Token {
        request_id: Uuid,
        text: String,
    },
    Finished {
        request_id: Uuid,
    },
    Cancelled {
        request_id: Uuid,
    },
    Failed {
        request_id: Uuid,
        reason: String,
    },
    UserMessage {
        request_id: Uuid,
        author: String,
        content: String,
    },
}

#[derive(Clone)]
pub struct SchedulerHandle {
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<Event>,
}

impl SchedulerHandle {
    pub async fn submit(&self, owner: Uuid, author: String, prompt: String) -> Option<Uuid> {
        let request_id = Uuid::new_v4();
        self.commands
            .send(Command::Submit {
                request_id,
                owner,
                author,
                prompt,
            })
            .await
            .ok()?;
        Some(request_id)
    }

    pub async fn cancel(&self, requester: Uuid, request_id: Uuid) {
        let _ = self
            .commands
            .send(Command::Cancel {
                request_id,
                requester,
            })
            .await;
    }

    pub async fn disconnect(&self, owner: Uuid) {
        let _ = self.commands.send(Command::Disconnect { owner }).await;
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }
}

struct PartialReply {
    request_id: Uuid,
    text: String,
}

struct Cancellation {
    count: usize,
    partial: Option<PartialReply>,
}

struct Request {
    id: Uuid,
    owner: Uuid,
    user_message_id: i64,
}

struct Active {
    id: Uuid,
    owner: Uuid,
    reply: String,
    stream: BoxStream<'static, Result<String, LlmError>>,
}

pub fn spawn<B: ChatBackend>(
    backend: B,
    pool: SqlitePool,
    prompt_char_budget: usize,
) -> SchedulerHandle {
    let (commands_tx, commands_rx) = mpsc::channel(32);
    let (events_tx, _) = broadcast::channel(256);

    tokio::spawn(run(
        backend,
        pool,
        prompt_char_budget,
        commands_rx,
        events_tx.clone(),
    ));

    SchedulerHandle {
        commands: commands_tx,
        events: events_tx,
    }
}

async fn next_token(active: &mut Option<Active>) -> Option<Result<String, LlmError>> {
    match active {
        Some(a) => a.stream.next().await,
        None => std::future::pending().await,
    }
}

fn cancel_where(
    queue: &mut VecDeque<Request>,
    active: &mut Option<Active>,
    events: &broadcast::Sender<Event>,
    matches: impl Fn(Uuid, Uuid) -> bool,
) -> Cancellation {
    let mut count = 0;
    let mut index = 0;
    let mut first_removed: Option<usize> = None;

    queue.retain(|r| {
        let i = index;
        index += 1;

        let hit = matches(r.id, r.owner);
        if hit {
            first_removed.get_or_insert(i);
            count += 1;
            let _ = events.send(Event::Cancelled { request_id: r.id });
        }
        !hit
    });

    if let Some(from) = first_removed {
        send_positions(queue, events, from);
    }

    let mut partial = None;
    if let Some(a) = active.take_if(|a| matches(a.id, a.owner)) {
        count += 1;
        let _ = events.send(Event::Cancelled { request_id: a.id });
        if !a.reply.is_empty() {
            partial = Some(PartialReply {
                request_id: a.id,
                text: a.reply,
            })
        }
    }

    Cancellation { count, partial }
}

async fn save_partial(pool: &SqlitePool, partial: Option<PartialReply>, status: db::Status) {
    let Some(p) = partial else { return };
    if let Err(e) = db::insert_message(
        pool,
        db::CONVERSATION_ID,
        Role::Assistant,
        None,
        &p.text,
        status,
    )
    .await
    {
        tracing::error!(request_id = %p.request_id, error = %e, "could not save partial reply");
    }
}

fn send_positions(queue: &VecDeque<Request>, events: &broadcast::Sender<Event>, from: usize) {
    for (position, req) in queue.iter().enumerate().skip(from) {
        let _ = events.send(Event::PositionChanged {
            request_id: req.id,
            position,
        });
    }
}

async fn run<B: ChatBackend>(
    backend: B,
    pool: SqlitePool,
    prompt_char_budget: usize,
    mut commands: mpsc::Receiver<Command>,
    events: broadcast::Sender<Event>,
) {
    let mut queue: VecDeque<Request> = VecDeque::new();
    let mut active: Option<Active> = None;

    loop {
        if active.is_none()
            && let Some(req) = queue.pop_front()
        {
            send_positions(&queue, &events, 0);

            let loaded = async {
                let mut history = db::context_messages(
                    &pool,
                    db::CONVERSATION_ID,
                    req.user_message_id,
                    HISTORY_FETCH_LIMIT,
                )
                .await?;
                history.push(db::get_message(&pool, req.user_message_id).await?);
                Ok::<_, sqlx::Error>(history)
            }
            .await;

            match loaded {
                Ok(history) => {
                    let messages = chat::build_context(history, prompt_char_budget);
                    tracing::info!(
                        request_id = %req.id, context_messages = messages.len(), "generation started"
                    );
                    tracing::debug!(?messages, "context sent to model");
                    let _ = events.send(Event::Started { request_id: req.id });
                    active = Some(Active {
                        id: req.id,
                        owner: req.owner,
                        reply: String::new(),
                        stream: backend.stream_chat(messages),
                    });
                }
                Err(e) => {
                    tracing::error!(request_id = %req.id, error = %e, "could not load history");
                    let _ = events.send(Event::Failed {
                        request_id: req.id,
                        reason: "could not load conversation history".into(),
                    });
                    continue;
                }
            }
        }

        tokio::select! {
            command = commands.recv() => match command {
                Some(Command::Submit { request_id, owner, author, prompt }) => {
                    let user_message_id = match db::insert_message(
                        &pool, db::CONVERSATION_ID, Role::User, Some(&author), &prompt,
                        db::Status::Complete,
                    ).await {
                        Ok(id) => id,
                        Err(e) => {
                            tracing::error!(%request_id, error = %e, "could not save user message");
                            let _ = events.send(Event::Failed {
                                request_id,
                                reason: "could not save message".into(),
                            });
                            continue;
                        }
                    };

                    let _ = events.send(Event::UserMessage { request_id, author, content: prompt });

                    let position = queue.len();
                    queue.push_back(Request { id: request_id, owner, user_message_id });
                    tracing::info!(%request_id, %owner, position, "request queued");
                    let _ = events.send(Event::Queued { request_id, position });
                }

                Some(Command::Cancel { request_id, requester }) => {
                    let result = cancel_where(&mut queue, &mut active, &events, |id, owner| {
                        id == request_id && owner == requester
                    });
                    if result.count == 0 {
                        tracing::warn!(%request_id, %requester, "cancel matched nothing");
                    }
                    save_partial(&pool, result.partial, db::Status::Cancelled).await;
                }

                Some(Command::Disconnect { owner: gone }) => {
                    let result = cancel_where(&mut queue, &mut active, &events, |_, owner| owner == gone);
                    tracing::debug!(owner = %gone, cancelled = result.count, "connection cleaned up");
                    save_partial(&pool, result.partial, db::Status::Cancelled).await;
                }

                None => break,
            },

            item = next_token(&mut active) => {
                let Some(a) = active.as_mut() else { continue };
                let id = a.id;

                match item {
                    Some(Ok(text)) => {
                        a.reply.push_str(&text);
                        tracing::trace!(%id, "token");
                        let _ = events.send(Event::Token { request_id: id, text });
                    }
                    Some(Err(e)) => {
                        tracing::error!(request_id = %id, error = %e, "generation failed");
                        let _ = events.send(Event::Failed { request_id: id, reason: e.to_string() });
                        let partial = (!a.reply.is_empty()).then(|| PartialReply {
                            request_id: id,
                            text: std::mem::take(&mut a.reply),
                        });
                        active = None;
                        save_partial(&pool, partial, db::Status::Failed).await;
                    }
                    None => {
                        let reply = std::mem::take(&mut a.reply);
                        active = None;
                        if let Err(e) = db::insert_message(
                            &pool, db::CONVERSATION_ID, Role::Assistant, None, &reply,
                            db::Status::Complete,
                        ).await {
                            tracing::error!(request_id = %id, error = %e, "could not save reply");
                        }
                        tracing::info!(request_id = %id, "generation finished");
                        let _ = events.send(Event::Finished { request_id: id });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
pub mod tests;
