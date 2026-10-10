use std::{env, net::SocketAddr};

use anyhow::Context;

pub struct Config {
    pub llama_url: String,
    pub bind_addr: SocketAddr,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let llama_url = var_or("LLAMA_URL", "http://localhost:8080");
        let bind_addr = var_or("BIND_ADDR", "127.0.0.1:3000")
            .parse()
            .context("BIND_ADDR must look like 127.0.0.1:3000")?;

        Ok(Self {
            llama_url,
            bind_addr,
        })
    }
}

fn var_or(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_owned())
}
