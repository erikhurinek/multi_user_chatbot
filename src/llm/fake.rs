use futures_util::{StreamExt, stream::BoxStream};
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

use super::{ChatBackend, LlmError, Message};

type Chunk = Result<String, LlmError>;

pub struct FakeLlm {
    calls: mpsc::UnboundedSender<FakeCall>,
}

pub struct FakeCall {
    pub messages: Vec<Message>,
    tx: mpsc::UnboundedSender<Chunk>,
}

pub fn fake_llm() -> (FakeLlm, mpsc::UnboundedReceiver<FakeCall>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (FakeLlm { calls: tx }, rx)
}

impl ChatBackend for FakeLlm {
    fn stream_chat(&self, messages: Vec<Message>) -> BoxStream<'static, Chunk> {
        let (tx, rx) = mpsc::unbounded_channel();
        let _ = self.calls.send(FakeCall { messages, tx });
        UnboundedReceiverStream::new(rx).boxed()
    }
}

impl FakeCall {
    pub fn send_token(&self, text: &str) {
        self.tx
            .send(Ok(text.to_owned()))
            .expect("the scheduler dropped the stream");
    }

    pub fn fail(&self) {
        let error = serde_json::from_str::<()>("not json").unwrap_err();
        self.tx
            .send(Err(LlmError::Parse(error)))
            .expect("the scheduler dropped the stream");
    }

    pub fn finish(self) {}

    pub fn is_cancelled(&self) -> bool {
        self.tx.is_closed()
    }

    pub async fn cancelled(&self) {
        self.tx.closed().await
    }
}
