use futures_util::stream::BoxStream;

use super::{error::LlmError, types::Message};

pub trait ChatBackend: Send + Sync + 'static {
    fn stream_chat(&self, messages: Vec<Message>) -> BoxStream<'static, Result<String, LlmError>>;
}
