mod backend;
mod client;
mod error;
mod sse;
mod types;

#[cfg(test)]
pub mod fake;

pub use backend::ChatBackend;
pub use client::LlmClient;
pub use error::LlmError;
pub use types::{Message, Role};
