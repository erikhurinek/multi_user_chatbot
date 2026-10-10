use async_stream::try_stream;
use futures_util::{StreamExt, stream::BoxStream};

use super::error::LlmError;
use super::sse::{LineBuffer, SseEvent, parse_line};
use super::types::{ChatRequest, Message, StreamChunk};

#[derive(Clone)]
pub struct LlmClient {
    http: reqwest::Client,
    chat_url: String,
}

impl LlmClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            chat_url: format!("{}/v1/chat/completions", base_url.trim_end_matches('/')),
        }
    }

    pub fn stream_chat(
        &self,
        messages: Vec<Message>,
    ) -> BoxStream<'static, Result<String, LlmError>> {
        let http = self.http.clone();
        let url = self.chat_url.clone();

        let stream = try_stream! {
            let response = http
                .post(url)
                .json(&ChatRequest { messages, stream: true })
                .send()
                .await?
                .error_for_status()?;

            let mut bytes = response.bytes_stream();
            let mut buffer = LineBuffer::default();

            'read: while let Some(chunk) = bytes.next().await {
                buffer.push(&chunk?);

                while let Some(line) = buffer.next_line() {
                    match parse_line(&line) {
                        Some (SseEvent::Done) => break 'read,
                        Some (SseEvent::Data(json)) => {
                            let parsed: StreamChunk = serde_json::from_str(&json)?;
                            if let Some(text) = parsed.into_content() {
                                yield text;
                            }
                        }
                        None => {}
                    }
                }
            }
        };

        stream.boxed()
    }
}
