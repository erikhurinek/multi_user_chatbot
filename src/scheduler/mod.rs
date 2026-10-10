use std::collections::VecDeque;

use futures_util::{StreamExt, stream::BoxStream};
use serde::Serialize;
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

use crate::llm::{LlmClient, LlmError, Message, Role};

pub enum Command {
    Submit {
        request_id: Uuid,
        owner: Uuid,
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

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Queued { request_id: Uuid, position: usize },
    PositionChanged { request_id: Uuid, position: usize },
    Started { request_id: Uuid },
    Token { request_id: Uuid, text: String },
    Finished { request_id: Uuid },
    Cancelled { request_id: Uuid },
    Failed { request_id: Uuid, reason: String },
}

#[derive(Clone)]
pub struct SchedulerHandle {
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<Event>,
}

impl SchedulerHandle {
    pub async fn submit(&self, owner: Uuid, prompt: String) -> Option<Uuid> {
        let request_id = Uuid::new_v4();
        self.commands
            .send(Command::Submit {
                request_id,
                owner,
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

pub fn spawn(llm: LlmClient) -> SchedulerHandle {
    let (commands_tx, commands_rx) = mpsc::channel(32);
    let (events_tx, _) = broadcast::channel(256);

    tokio::spawn(run(llm, commands_rx, events_tx.clone()));

    SchedulerHandle {
        commands: commands_tx,
        events: events_tx,
    }
}

struct Request {
    id: Uuid,
    owner: Uuid,
    prompt: String,
}

struct Active {
    id: Uuid,
    owner: Uuid,
    stream: BoxStream<'static, Result<String, LlmError>>,
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
) -> usize {
    let mut cancelled = 0;
    let mut index = 0;
    let mut first_removed: Option<usize> = None;

    queue.retain(|r| {
        let i = index;
        index += 1;

        let hit = matches(r.id, r.owner);
        if hit {
            first_removed.get_or_insert(i);
            cancelled += 1;
            let _ = events.send(Event::Cancelled { request_id: r.id });
        }
        !hit
    });

    if let Some(from) = first_removed {
        send_positions(queue, events, from);
    }

    if let Some(a) = active.take_if(|a| matches(a.id, a.owner)) {
        cancelled += 1;
        let _ = events.send(Event::Cancelled { request_id: a.id });
    }

    cancelled
}

fn send_positions(queue: &VecDeque<Request>, events: &broadcast::Sender<Event>, from: usize) {
    for (position, req) in queue.iter().enumerate().skip(from) {
        let _ = events.send(Event::PositionChanged {
            request_id: req.id,
            position,
        });
    }
}

async fn run(
    llm: LlmClient,
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
            let messages = vec![Message {
                role: Role::User,
                content: req.prompt,
            }];
            let _ = events.send(Event::Started { request_id: req.id });
            active = Some(Active {
                id: req.id,
                owner: req.owner,
                stream: llm.stream_chat(messages),
            })
        }

        tokio::select! {
            command = commands.recv() => match command {
                Some(Command::Submit { request_id, owner, prompt }) => {
                    let position = queue.len();
                    queue.push_back(Request { id: request_id, owner, prompt });
                    tracing::info!(%request_id, %owner, position, "request queued");
                    let _ = events.send(Event::Queued { request_id, position });
                }
                Some(Command::Cancel { request_id, requester }) => {
                    let n = cancel_where(&mut queue, &mut active, &events, |id, owner| {
                        id == request_id && owner == requester
                    });
                    if n == 0 {
                        tracing::info!(%request_id, %requester, "cancel matched nothing");
                    }
                }
                Some(Command::Disconnect { owner: gone }) => {
                    let n = cancel_where(&mut queue, &mut active, &events, |_, owner| owner == gone);
                    tracing::debug!(owner = %gone, cancelled = n, "connection cleaned up");
                }
                None => break
            },

            item = next_token(&mut active) => {
                let Some(id) = active.as_ref().map(|a| a.id) else { continue };
                match item {
                    Some(Ok(text)) => {
                        let _ = events.send(Event::Token { request_id: id, text });
                    }
                    Some(Err(e)) => {
                        let _ = events.send(Event::Failed { request_id: id, reason: e.to_string() });
                        active=None;
                    }
                    None => {
                        let _ = events.send(Event::Finished { request_id: id });
                        active = None;
                    }
                }
            }
        }
    }
}
