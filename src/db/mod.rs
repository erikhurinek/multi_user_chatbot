use std::str::FromStr;

use serde::Serialize;
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

use crate::llm::Role;

pub const CONVERSATION_ID: i64 = 1;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct StoredMessage {
    pub id: i64,
    pub role: String,
    pub author: String,
    pub content: String,
    pub created_at: String,
}

pub async fn connect(url: &str) -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str(url)?
        .journal_mode(SqliteJournalMode::Wal)
        .create_if_missing(true)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    sqlx::migrate!().run(&pool).await?;
    Ok(pool)
}

pub async fn insert_message(
    pool: &SqlitePool,
    conversation_id: i64,
    role: Role,
    author: Option<&str>,
    content: &str,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO messages (conversation_id, role, author, content) VALUES (?, ?, ?, ?)",
    )
    .bind(conversation_id)
    .bind(role.as_str())
    .bind(author)
    .bind(content)
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

pub async fn recent_messages(
    pool: &SqlitePool,
    conversation_id: i64,
    limit: i64,
) -> Result<Vec<StoredMessage>, sqlx::Error> {
    let mut rows = sqlx::query_as::<_, StoredMessage>(
        "SELECT id, role, author, content, created_at FROM messages \
         WHERE conversation_id = ? ORDER BY id DESC LIMIT ?",
    )
    .bind(conversation_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.reverse();
    Ok(rows)
}
