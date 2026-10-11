use std::{env, net::SocketAddr};

use anyhow::Context;

pub struct Config {
    pub llama_url: String,
    pub bind_addr: SocketAddr,
    pub database_url: String,
    pub prompt_char_budget: usize,
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

        Ok(Self {
            llama_url,
            bind_addr,
            database_url,
            prompt_char_budget,
        })
    }
}

fn var_or(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_owned())
}
