use std::str::FromStr;

use serde::Serialize;
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

use crate::llm::Role;

pub const CONVERSATION_ID: i64 = 1;

#[derive(Debug, Clone, Copy)]
pub enum Status {
    Complete,
    Cancelled,
    Failed,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Complete => "complete",
            Status::Cancelled => "cancelled",
            Status::Failed => "failed",
        }
    }
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct StoredMessage {
    pub id: i64,
    pub role: String,
    pub author: Option<String>,
    pub content: String,
    pub status: String,
    pub created_at: String,
}

pub async fn connect_with(url: &str, max_connections: u32) -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str(url)?
        .journal_mode(SqliteJournalMode::Wal)
        .create_if_missing(true)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect_with(options)
        .await?;

    sqlx::migrate!().run(&pool).await?;
    Ok(pool)
}

pub async fn connect(url: &str) -> anyhow::Result<SqlitePool> {
    connect_with(url, 5).await
}

pub async fn insert_message(
    pool: &SqlitePool,
    conversation_id: i64,
    role: Role,
    author: Option<&str>,
    content: &str,
    status: Status,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO messages (conversation_id, role, author, content, status) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(conversation_id)
    .bind(role.as_str())
    .bind(author)
    .bind(content)
    .bind(status.as_str())
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
        "SELECT id, role, author, content, status, created_at FROM messages \
         WHERE conversation_id = ? ORDER BY id DESC LIMIT ?",
    )
    .bind(conversation_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.reverse();
    Ok(rows)
}

pub async fn context_messages(
    pool: &SqlitePool,
    conversation_id: i64,
    current_id: i64,
    limit: i64,
) -> Result<Vec<StoredMessage>, sqlx::Error> {
    let mut rows = sqlx::query_as::<_, StoredMessage>(
        "SELECT id, role, author, content, status, created_at FROM messages \
         WHERE conversation_id = ? AND status = 'complete' \
           AND id != ? AND (role != 'user' OR id < ?) \
         ORDER BY id DESC LIMIT ?",
    )
    .bind(conversation_id)
    .bind(current_id)
    .bind(current_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.reverse();
    Ok(rows)
}

pub async fn get_message(pool: &SqlitePool, id: i64) -> Result<StoredMessage, sqlx::Error> {
    sqlx::query_as::<_, StoredMessage>(
        "SELECT id, role, author, content, status, created_at FROM messages WHERE id = ?",
    )
    .bind(id)
    .fetch_one(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_insert_and_retrieve_messages() {
        let pool = connect("sqlite::memory:").await.unwrap();
        insert_message(
            &pool,
            CONVERSATION_ID,
            Role::User,
            Some("User 1"),
            "Message 1",
            Status::Complete,
        )
        .await
        .unwrap();
        insert_message(
            &pool,
            CONVERSATION_ID,
            Role::Assistant,
            Some("Bot"),
            "Message 2",
            Status::Complete,
        )
        .await
        .unwrap();
        insert_message(
            &pool,
            CONVERSATION_ID,
            Role::User,
            Some("User 1"),
            "Message 3",
            Status::Complete,
        )
        .await
        .unwrap();

        let messages = recent_messages(&pool, CONVERSATION_ID, 2).await.unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "Message 2");
        assert_eq!(messages[1].content, "Message 3");
    }
}
