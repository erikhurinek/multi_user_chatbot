use thiserror::Error;

#[derive(Error, Debug)]
pub enum LlmError {
    #[error("request to llama-server failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("could not parse a streamed chunk: {0}")]
    Parse(#[from] serde_json::Error),
}
