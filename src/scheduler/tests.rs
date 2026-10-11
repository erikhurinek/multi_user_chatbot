use std::time::Duration;

use tokio::time::{sleep, timeout};

use super::*;
use crate::llm::fake::{FakeCall, fake_llm};

const WAIT: Duration = Duration::from_secs(2);

struct Harness {
    scheduler: SchedulerHandle,
    events: broadcast::Receiver<Event>,
    calls: mpsc::UnboundedReceiver<FakeCall>,
    pool: SqlitePool,
}

impl Harness {
    async fn new() -> Self {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();

        let pool = db::connect_with("sqlite::memory:", 1).await.unwrap();
        let (fake, calls) = fake_llm();
        let scheduler = spawn(fake, pool.clone(), 6000);
        let events = scheduler.subscribe();
        Self {
            scheduler,
            events,
            calls,
            pool,
        }
    }

    async fn event(&mut self) -> Event {
        timeout(WAIT, self.events.recv())
            .await
            .expect("timed out waiting for an event")
            .expect("event channel closed or lagged")
    }

    async fn call(&mut self) -> FakeCall {
        timeout(WAIT, self.calls.recv())
            .await
            .expect("timed out waiting for a model call")
            .expect("fake backend dropped")
    }

    async fn events_until(&mut self, done: impl Fn(&Event) -> bool) -> Vec<Event> {
        let mut seen = Vec::new();
        loop {
            let event = self.event().await;
            let finished = done(&event);
            seen.push(event);
            if finished {
                return seen;
            }
        }
    }

    async fn silence(&mut self) {
        if let Ok(event) = timeout(Duration::from_millis(150), self.events.recv()).await {
            panic!("expected silence, got {event:?}");
        }
    }

    async fn start(&mut self, owner: Uuid, author: &str, prompt: &str) -> (Uuid, FakeCall) {
        let id = self
            .scheduler
            .submit(owner, author.into(), prompt.into())
            .await
            .unwrap();
        assert!(matches!(self.event().await, Event::UserMessage { .. }));
        assert_eq!(
            self.event().await,
            Event::Queued {
                request_id: id,
                position: 0
            }
        );
        assert_eq!(self.event().await, Event::Started { request_id: id });
        (id, self.call().await)
    }

    async fn wait_for_rows(&self, count: usize) -> Vec<db::StoredMessage> {
        timeout(WAIT, async {
            loop {
                let rows = db::recent_messages(&self.pool, db::CONVERSATION_ID, 50)
                    .await
                    .unwrap();
                if rows.len() >= count {
                    return rows;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("timed out waiting for database rows")
    }
}

#[tokio::test]
async fn reply_streams_then_is_saved() {
    let mut h = Harness::new().await;
    let alice = Uuid::new_v4();

    let (id, call) = h.start(alice, "alice", "Hello").await;
    assert_eq!(call.messages.last().unwrap().content, "alice: Hello");

    call.send_token("Hi");
    call.send_token(" there");
    assert_eq!(
        h.event().await,
        Event::Token {
            request_id: id,
            text: "Hi".into()
        }
    );
    assert_eq!(
        h.event().await,
        Event::Token {
            request_id: id,
            text: " there".into()
        }
    );

    call.finish();
    assert_eq!(h.event().await, Event::Finished { request_id: id });

    let rows = db::recent_messages(&h.pool, db::CONVERSATION_ID, 10)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].author.as_deref(), Some("alice"));
    assert_eq!(rows[1].role, "assistant");
    assert_eq!(rows[1].content, "Hi there");
    assert_eq!(rows[1].status, "complete");
}

#[tokio::test]
async fn requests_run_one_at_a_time_and_later_ones_see_earlier_replies() {
    let mut h = Harness::new().await;
    let (alice, bob) = (Uuid::new_v4(), Uuid::new_v4());

    let (a_id, a_call) = h.start(alice, "alice", "First").await;

    let b_id = h
        .scheduler
        .submit(bob, "bob".into(), "Second".into())
        .await
        .unwrap();
    assert!(matches!(h.event().await, Event::UserMessage { .. }));
    assert_eq!(
        h.event().await,
        Event::Queued {
            request_id: b_id,
            position: 0
        }
    );

    h.silence().await;
    assert!(h.calls.try_recv().is_err(), "the second request must wait");

    a_call.send_token("ok");
    assert_eq!(
        h.event().await,
        Event::Token {
            request_id: a_id,
            text: "ok".into()
        }
    );
    a_call.finish();
    assert_eq!(h.event().await, Event::Finished { request_id: a_id });
    assert_eq!(h.event().await, Event::Started { request_id: b_id });

    let b_call = h.call().await;
    let contents: Vec<&str> = b_call.messages.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(contents[1..], ["alice: First", "ok", "bob: Second"]);
}

#[tokio::test]
async fn cancelling_stops_the_model_and_keeps_the_partial_reply() {
    let mut h = Harness::new().await;
    let alice = Uuid::new_v4();
    let (id, call) = h.start(alice, "alice", "Tell me a story").await;

    call.send_token("Once upon");
    h.event().await;

    h.scheduler.cancel(alice, id).await;
    assert_eq!(h.event().await, Event::Cancelled { request_id: id });
    timeout(WAIT, call.cancelled())
        .await
        .expect("the stream was never dropped");

    let rows = h.wait_for_rows(2).await;
    assert_eq!(rows[1].content, "Once upon");
    assert_eq!(rows[1].status, "cancelled");
}

#[tokio::test]
async fn only_the_owner_can_cancel() {
    let mut h = Harness::new().await;
    let (alice, bob) = (Uuid::new_v4(), Uuid::new_v4());
    let (id, call) = h.start(alice, "alice", "Hi").await;

    h.scheduler.cancel(bob, id).await;

    h.scheduler
        .submit(bob, "bob".into(), "Hello?".into())
        .await
        .unwrap();
    assert!(matches!(h.event().await, Event::UserMessage { .. }));

    assert!(!call.is_cancelled());
}

#[tokio::test]
async fn disconnect_cancels_everything_the_connection_owns() {
    let mut h = Harness::new().await;
    let (alice, bob) = (Uuid::new_v4(), Uuid::new_v4());

    let (a1, a1_call) = h.start(alice, "alice", "one").await;
    let a2 = h
        .scheduler
        .submit(alice, "alice".into(), "two".into())
        .await
        .unwrap();
    let b1 = h
        .scheduler
        .submit(bob, "bob".into(), "three".into())
        .await
        .unwrap();

    h.scheduler.disconnect(alice).await;

    let seen = h.events_until(|e| matches!(e, Event::Started { .. })).await;
    assert!(seen.contains(&Event::Cancelled { request_id: a1 }));
    assert!(seen.contains(&Event::Cancelled { request_id: a2 }));
    assert!(seen.contains(&Event::PositionChanged {
        request_id: b1,
        position: 0
    }));
    assert_eq!(seen.last(), Some(&Event::Started { request_id: b1 }));

    timeout(WAIT, a1_call.cancelled())
        .await
        .expect("the stream was never dropped");
}

#[tokio::test]
async fn a_failed_generation_is_saved_as_failed_and_the_queue_moves_on() {
    let mut h = Harness::new().await;
    let (alice, bob) = (Uuid::new_v4(), Uuid::new_v4());

    let (a_id, a_call) = h.start(alice, "alice", "one").await;
    let b_id = h
        .scheduler
        .submit(bob, "bob".into(), "two".into())
        .await
        .unwrap();
    h.event().await;
    h.event().await;

    a_call.send_token("par");
    a_call.fail();
    assert_eq!(
        h.event().await,
        Event::Token {
            request_id: a_id,
            text: "par".into()
        }
    );
    assert!(matches!(
        h.event().await,
        Event::Failed { request_id, .. } if request_id == a_id
    ));
    assert_eq!(h.event().await, Event::Started { request_id: b_id });

    let rows = h.wait_for_rows(3).await;
    let failed = rows.iter().find(|r| r.role == "assistant").unwrap();
    assert_eq!(failed.content, "par");
    assert_eq!(failed.status, "failed");

    let b_call = h.call().await;
    assert!(b_call.messages.iter().all(|m| m.content != "par"));
}
