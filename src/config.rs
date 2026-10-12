use std::{env, fs, net::SocketAddr};

use anyhow::Context;

pub const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful assistant in a shared chat room \
with several participants. Each user message begins with the speaker's name followed by a \
colon. Use names to tell people apart, but do not begin your own replies with a name. \
Several people may write before you get a chance to reply. When that happens, answer all of \
them in a single message, addressing each person by name where it helps.";

fn load_system_prompt() -> anyhow::Result<String> {
    if let Ok(path) = env::var("SYSTEM_PROMPT_FILE") {
        let text = fs::read_to_string(&path)
            .with_context(|| format!("could not read SYSTEM_PROMPT_FILE ({path})"))?;
        return non_empty(text, "SYSTEM_PROMPT_FILE");
    }
    match env::var("SYSTEM_PROMPT") {
        Ok(text) => non_empty(text, "SYSTEM_PROMPT"),
        Err(_) => Ok(DEFAULT_SYSTEM_PROMPT.to_owned()),
    }
}

fn non_empty(text: String, source: &str) -> anyhow::Result<String> {
    let text = text.trim().to_owned();
    anyhow::ensure!(!text.is_empty(), "{source} is set but empty");
    Ok(text)
}

pub struct Config {
    pub llama_url: String,
    pub bind_addr: SocketAddr,
    pub database_url: String,
    pub prompt_char_budget: usize,
    pub static_dir: std::path::PathBuf,
    pub system_prompt: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let llama_url = var_or("LLAMA_URL", "http://localhost:8080");
        let bind_addr = var_or("BIND_ADDR", "127.0.0.1:3000")
            .parse()
            .context("BIND_ADDR must look like 127.0.0.1:3000")?;
        let database_url = var_or("DATABASE_URL", "sqlite://chat.db");
        let prompt_char_budget = var_or("PROMPT_CHAR_BUDGET", "6000")
            .parse()
            .context("PROMPT_CHAR_BUDGET must be a whole number")?;
        let static_dir = var_or("STATIC_DIR", "./frontend/dist").into();
        let system_prompt = load_system_prompt()?;

        Ok(Self {
            llama_url,
            bind_addr,
            database_url,
            prompt_char_budget,
            static_dir,
            system_prompt,
        })
    }
}

fn var_or(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn choose_system_prompt(file: Option<String>, inline: Option<String>) -> anyhow::Result<String> {
    match (file, inline) {
        (Some(text), _) => non_empty(text, "SYSTEM_PROMPT_FILE"),
        (None, Some(text)) => non_empty(text, "SYSTEM_PROMPT"),
        (None, None) => Ok(DEFAULT_SYSTEM_PROMPT.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_beats_inline_and_inline_beats_default() {
        assert_eq!(
            choose_system_prompt(Some("f".into()), Some("i".into())).unwrap(),
            "f"
        );
        assert_eq!(choose_system_prompt(None, Some("i".into())).unwrap(), "i");
        assert_eq!(
            choose_system_prompt(None, None).unwrap(),
            DEFAULT_SYSTEM_PROMPT
        );
    }

    #[test]
    fn blank_prompts_are_rejected() {
        assert!(choose_system_prompt(None, Some("  \n".into())).is_err());
    }
}
