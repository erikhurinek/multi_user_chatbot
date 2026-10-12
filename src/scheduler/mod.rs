use sqlx::SqlitePool;
use std::collections::VecDeque;
use tokio::sync::oneshot;

use futures_util::{StreamExt, stream::BoxStream};
use serde::Serialize;
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

use crate::chat;
use crate::db;
use crate::db::StoredMessage;
use crate::llm::{ChatBackend, LlmError, Role};

const HISTORY_LIMIT: i64 = 50;
const MAX_BATCH: usize = 8;

pub enum Command {
    Submit {
        request_id: Uuid,
        owner: Uuid,
        author: String,
        prompt: String,
    },
    Cancel {
        id: Uuid,
        requester: Uuid,
    },
    Disconnect {
        owner: Uuid,
    },
    Join {
        reply: oneshot::Sender<Joined>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    UserMessage {
        request_id: Uuid,
        message_id: i64,
        author: String,
        content: String,
    },
    Queued {
        request_id: Uuid,
    },
    Withdrawn {
        request_id: Uuid,
    },
    Rejected {
        request_id: Uuid,
        reason: String,
    },
    Started {
        turn_id: Uuid,
        request_ids: Vec<Uuid>,
    },
    Token {
        turn_id: Uuid,
        text: String,
    },
    Finished {
        turn_id: Uuid,
    },
    Cancelled {
        turn_id: Uuid,
    },
    Failed {
        turn_id: Uuid,
        reason: String,
    },
}

#[derive(Serialize)]
pub struct Pending {
    pub request_id: Uuid,
    pub message_id: i64,
    pub turn_id: Option<Uuid>,
}

#[derive(Serialize)]
pub struct RunningTurn {
    pub turn_id: Uuid,
    pub reply: String,
}

#[derive(Serialize)]
pub struct Snapshot {
    pub messages: Vec<StoredMessage>,
    pub pending: Vec<Pending>,
    pub turn: Option<RunningTurn>,
}

pub struct Joined {
    pub events: broadcast::Receiver<Event>,
    pub snapshot: Snapshot,
}

#[derive(Clone)]
pub struct SchedulerHandle {
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<Event>,
}

impl SchedulerHandle {
    pub async fn join(&self) -> Option<Joined> {
        let (reply, answer) = oneshot::channel();
        self.commands.send(Command::Join { reply }).await.ok()?;
        answer.await.ok()
    }

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

    pub async fn cancel(&self, requester: Uuid, turn_id: Uuid) {
        let _ = self
            .commands
            .send(Command::Cancel {
                id: turn_id,
                requester,
            })
            .await;
    }

    pub async fn disconnect(&self, owner: Uuid) {
        let _ = self.commands.send(Command::Disconnect { owner }).await;
    }
}

struct PartialReply {
    turn_id: Uuid,
    text: String,
}

struct PromptRef {
    request_id: Uuid,
    message_id: i64,
}

struct Request {
    id: Uuid,
    owner: Uuid,
    user_message_id: i64,
}

struct Active {
    id: Uuid,
    owners: Vec<Uuid>,
    prompts: Vec<PromptRef>,
    reply: String,
    stream: BoxStream<'static, Result<String, LlmError>>,
}

impl Active {
    fn into_partial(self) -> Option<PartialReply> {
        (!self.reply.is_empty()).then(|| PartialReply {
            turn_id: self.id,
            text: self.reply,
        })
    }
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
        tracing::error!(request_id = %p.turn_id, error = %e, "could not save partial reply");
    }
}

fn withdraw_where(
    queue: &mut VecDeque<Request>,
    events: &broadcast::Sender<Event>,
    matches: impl Fn(&Request) -> bool,
) -> usize {
    let mut count = 0;
    queue.retain(|r| {
        let hit = matches(r);
        if hit {
            count += 1;
            let _ = events.send(Event::Withdrawn { request_id: r.id });
        }
        !hit
    });
    count
}

async fn stop_turn(pool: &SqlitePool, events: &broadcast::Sender<Event>, turn: Active) {
    let turn_id = turn.id;
    save_partial(pool, turn.into_partial(), db::Status::Cancelled).await;
    let _ = events.send(Event::Cancelled { turn_id });
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
        if active.is_none() && !queue.is_empty() {
            let take = queue.len().min(MAX_BATCH);
            let batch: Vec<Request> = queue.drain(..take).collect();
            let first_id = batch[0].user_message_id;

            let loaded = async {
                let mut history =
                    db::context_messages(&pool, db::CONVERSATION_ID, first_id, HISTORY_LIMIT)
                        .await?;
                for r in &batch {
                    history.push(db::get_message(&pool, r.user_message_id).await?);
                }
                Ok::<_, sqlx::Error>(history)
            }
            .await;

            match loaded {
                Ok(history) => {
                    let turn_id = Uuid::new_v4();
                    let messages = chat::build_context(history, prompt_char_budget);
                    tracing::info!(%turn_id, prompts = batch.len(), "generation started");
                    let _ = events.send(Event::Started {
                        turn_id,
                        request_ids: batch.iter().map(|r| r.id).collect(),
                    });
                    active = Some(Active {
                        id: turn_id,
                        owners: batch.iter().map(|r| r.owner).collect(),
                        prompts: batch
                            .iter()
                            .map(|r| PromptRef {
                                request_id: r.id,
                                message_id: r.user_message_id,
                            })
                            .collect(),
                        reply: String::new(),
                        stream: backend.stream_chat(messages),
                    });
                }
                Err(e) => {
                    tracing::error!(error = %e, "could not load history");
                    for r in &batch {
                        let _ = events.send(Event::Rejected {
                            request_id: r.id,
                            reason: "could not load conversation history".into(),
                        });
                    }
                    continue;
                }
            }
        }

        tokio::select! {
            command = commands.recv() => match command {
                Some(Command::Submit { request_id, owner, author, prompt }) => {
                    let user_message_id = match db::insert_message(
                        &pool, db::CONVERSATION_ID, Role::User, Some(&author), &prompt, db::Status::Complete,
                    ).await {
                        Ok(id) => id,
                        Err(e) => {
                            tracing::error!(%request_id, error = %e, "could not save user message");
                            let _ = events.send(Event::Rejected { request_id, reason: "could not save message".into() });
                            continue;
                        }
                    };
                    let _ = events.send(Event::UserMessage {
                        request_id,
                        message_id: user_message_id,
                        author,
                        content: prompt,
                    });
                    queue.push_back(Request { id: request_id, owner, user_message_id });
                    tracing::info!(%request_id, %owner, "request queued");
                    let _ = events.send(Event::Queued { request_id });
                }

                Some(Command::Cancel { id, requester }) => {
                    let mut count = withdraw_where(&mut queue, &events, |r| r.id == id && r.owner == requester);
                    if let Some(turn) = active.take_if(|a| a.id == id && a.owners.contains(&requester)) {
                        count += 1;
                        stop_turn(&pool, &events, turn).await;
                    }
                    if count == 0 {
                        tracing::warn!(%id, %requester, "cancel matched nothing");
                    }
                }

                Some(Command::Disconnect { owner: gone }) => {
                    let withdrawn = withdraw_where(&mut queue, &events, |r| r.owner == gone);
                    if let Some(a) = active.as_mut() {
                        a.owners.retain(|o| *o != gone);
                    }
                    if let Some(turn) = active.take_if(|a| a.owners.is_empty()) {
                        stop_turn(&pool, &events, turn).await;
                    }
                    tracing::debug!(owner = %gone, withdrawn, "connection cleaned up");
                }

                Some(Command::Join { reply }) => {
                    let receiver = events.subscribe();

                    match db::recent_messages(&pool, db::CONVERSATION_ID, HISTORY_LIMIT).await {
                        Ok(messages) => {
                            let mut pending: Vec<Pending> = queue
                                .iter()
                                .map(|r| Pending { request_id: r.id, message_id: r.user_message_id, turn_id: None })
                                .collect();

                            let turn = active.as_ref().map(|a| {
                                pending.extend(a.prompts.iter().map(|p| Pending {
                                    request_id: p.request_id,
                                    message_id: p.message_id,
                                    turn_id: Some(a.id),
                                }));
                                RunningTurn { turn_id: a.id, reply: a.reply.clone() }
                            });

                            let _ = reply.send(Joined { events: receiver, snapshot: Snapshot { messages, pending, turn } });
                        }
                        Err(e) => tracing::error!(error = %e, "could not build snapshot"),
                    }
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
                        let _ = events.send(Event::Token { turn_id: id, text });
                    }
                    Some(Err(e)) => {
                        tracing::error!(request_id = %id, error = %e, "generation failed");
                        let _ = events.send(Event::Failed { turn_id: id, reason: e.to_string() });
                        let partial = (!a.reply.is_empty()).then(|| PartialReply {
                            turn_id: id,
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
                        let _ = events.send(Event::Finished { turn_id: id });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
pub mod tests;
