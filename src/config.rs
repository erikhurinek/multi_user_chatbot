use std::{env, fs, io, net::SocketAddr, path::PathBuf};

use anyhow::Context;
use serde::Deserialize;

pub const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful assistant in a shared chat room \
with several participants. Each user message begins with the speaker's name followed by a \
colon. Use names to tell people apart, but do not begin your own replies with a name. \
Several people may write before you get a chance to reply. When that happens, answer all of \
them in a single message, addressing each person by name where it helps.";

pub struct Config {
    pub llama_url: String,
    pub bind_addr: SocketAddr,
    pub database_url: String,
    pub static_dir: PathBuf,
    pub prompt_char_budget: usize,
    pub system_prompt: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    llama_url: Option<String>,
    bind_addr: Option<String>,
    database_url: Option<String>,
    static_dir: Option<PathBuf>,
    prompt_char_budget: Option<usize>,
    system_prompt: Option<String>,
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        Self::resolve(read_file()?, |name| env::var(name).ok())
    }

    fn resolve(file: FileConfig, env: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let llama_url = env("LLAMA_URL")
            .or(file.llama_url)
            .unwrap_or_else(|| "http://localhost:8080".to_owned());

        let bind_addr = env("BIND_ADDR")
            .or(file.bind_addr)
            .unwrap_or_else(|| "127.0.0.1:3000".to_owned())
            .parse()
            .context("bind_addr must look like 127.0.0.1:3000")?;

        let database_url = env("DATABASE_URL")
            .or(file.database_url)
            .unwrap_or_else(|| "sqlite://chat.db".to_owned());

        let static_dir = env("STATIC_DIR")
            .map(PathBuf::from)
            .or(file.static_dir)
            .unwrap_or_else(|| PathBuf::from("frontend/dist"));

        let prompt_char_budget = match env("PROMPT_CHAR_BUDGET") {
            Some(text) => text
                .parse()
                .context("PROMPT_CHAR_BUDGET must be a whole number")?,
            None => file.prompt_char_budget.unwrap_or(6000),
        };

        let system_prompt = env("SYSTEM_PROMPT")
            .or(file.system_prompt)
            .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_owned());
        let system_prompt = system_prompt.trim().to_owned();
        anyhow::ensure!(!system_prompt.is_empty(), "system_prompt must not be empty");

        Ok(Self {
            llama_url,
            bind_addr,
            database_url,
            static_dir,
            prompt_char_budget,
            system_prompt,
        })
    }
}

fn read_file() -> anyhow::Result<FileConfig> {
    let (path, explicit) = match env::var("CONFIG_FILE") {
        Ok(path) => (PathBuf::from(path), true),
        Err(_) => (PathBuf::from("config.toml"), false),
    };

    match fs::read_to_string(&path) {
        Ok(text) => {
            toml::from_str(&text).with_context(|| format!("invalid config file {}", path.display()))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound && !explicit => Ok(FileConfig::default()),
        Err(e) => Err(e).with_context(|| format!("could not read {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn defaults_apply_when_nothing_is_set() {
        let c = Config::resolve(FileConfig::default(), no_env).unwrap();
        assert_eq!(c.system_prompt, DEFAULT_SYSTEM_PROMPT);
        assert_eq!(c.prompt_char_budget, 6000);
    }

    #[test]
    fn file_beats_default_and_env_beats_file() {
        let file: FileConfig = toml::from_str("llama_url = 'http://file:1'").unwrap();
        let c = Config::resolve(file, no_env).unwrap();
        assert_eq!(c.llama_url, "http://file:1");

        let file: FileConfig = toml::from_str("llama_url = 'http://file:1'").unwrap();
        let c = Config::resolve(file, |name| {
            (name == "LLAMA_URL").then(|| "http://env:2".to_owned())
        })
        .unwrap();
        assert_eq!(c.llama_url, "http://env:2");
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(toml::from_str::<FileConfig>("llama_ulr = 'typo'").is_err());
    }

    #[test]
    fn blank_prompt_and_bad_address_are_rejected() {
        let blank: FileConfig = toml::from_str("system_prompt = '   '").unwrap();
        assert!(Config::resolve(blank, no_env).is_err());

        let bad: FileConfig = toml::from_str("bind_addr = 'nonsense'").unwrap();
        assert!(Config::resolve(bad, no_env).is_err());
    }
}
