use std::time::Duration;

use tokio::time::{sleep, timeout};

use super::*;
use crate::config::DEFAULT_SYSTEM_PROMPT;
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
        let scheduler = spawn(fake, pool.clone(), DEFAULT_SYSTEM_PROMPT.to_owned(), 6000);
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
        assert_eq!(self.event().await, Event::Queued { request_id: id });
        let Event::Started {
            turn_id,
            request_ids,
        } = self.event().await
        else {
            panic!("expected Started");
        };
        assert_eq!(request_ids, vec![id]);
        (turn_id, self.call().await)
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
            turn_id: id,
            text: "Hi".into()
        }
    );
    assert_eq!(
        h.event().await,
        Event::Token {
            turn_id: id,
            text: " there".into()
        }
    );

    call.finish();
    assert_eq!(h.event().await, Event::Finished { turn_id: id });

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
    assert_eq!(h.event().await, Event::Queued { request_id: b_id });

    h.silence().await;
    assert!(h.calls.try_recv().is_err(), "the second request must wait");

    a_call.send_token("ok");
    assert_eq!(
        h.event().await,
        Event::Token {
            turn_id: a_id,
            text: "ok".into()
        }
    );
    a_call.finish();
    assert_eq!(h.event().await, Event::Finished { turn_id: a_id });
    let Event::Started { request_ids, .. } = h.event().await else {
        panic!("expected Started");
    };
    assert_eq!(request_ids, vec![b_id]);

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
    assert_eq!(h.event().await, Event::Cancelled { turn_id: id });
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
async fn prompts_queued_meanwhile_are_answered_in_one_turn() {
    let mut h = Harness::new().await;
    let (alice, bob, carol) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());

    let (turn1, call1) = h.start(alice, "alice", "First").await;

    let b = h
        .scheduler
        .submit(bob, "bob".into(), "Second".into())
        .await
        .unwrap();
    h.event().await;
    h.event().await;
    let c = h
        .scheduler
        .submit(carol, "carol".into(), "Third".into())
        .await
        .unwrap();
    h.event().await;
    h.event().await;

    call1.send_token("ok");
    h.event().await;
    call1.finish();
    assert_eq!(h.event().await, Event::Finished { turn_id: turn1 });

    let Event::Started { request_ids, .. } = h.event().await else {
        panic!("expected Started");
    };
    assert_eq!(request_ids, vec![b, c]);

    let call2 = h.call().await;
    let contents: Vec<&str> = call2.messages.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(
        contents[1..],
        ["alice: First", "ok", "bob: Second", "carol: Third"]
    );
    assert!(
        h.calls.try_recv().is_err(),
        "one model call for both prompts"
    );
}

#[tokio::test]
async fn joining_mid_turn_gives_a_consistent_snapshot() {
    let mut h = Harness::new().await;
    let alice = Uuid::new_v4();
    let (turn, call) = h.start(alice, "alice", "Hi").await;

    call.send_token("Hel");
    h.event().await;

    let mut joined = h.scheduler.join().await.unwrap();
    let running = joined.snapshot.turn.as_ref().unwrap();
    assert_eq!(running.turn_id, turn);
    assert_eq!(running.reply, "Hel");
    assert_eq!(joined.snapshot.pending.len(), 1);
    assert_eq!(joined.snapshot.pending[0].turn_id, Some(turn));
    assert_eq!(joined.snapshot.messages.len(), 1);

    call.send_token("lo");
    let next = timeout(WAIT, joined.events.recv()).await.unwrap().unwrap();
    assert_eq!(
        next,
        Event::Token {
            turn_id: turn,
            text: "lo".into()
        }
    );
}
