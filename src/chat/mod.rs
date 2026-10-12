use crate::{
    db::StoredMessage,
    llm::{Message, Role},
};

pub fn build_context(
    system_prompt: &str,
    history: Vec<StoredMessage>,
    char_budget: usize,
) -> Vec<Message> {
    let mut chosen: Vec<Message> = Vec::new();
    let mut used = system_prompt.chars().count();

    for stored in history.into_iter().rev() {
        let Some(message) = to_message(stored) else {
            continue;
        };
        used += message.content.chars().count();
        if used > char_budget && !chosen.is_empty() {
            break;
        }
        chosen.push(message);
    }
    chosen.reverse();

    let mut messages = vec![Message {
        role: Role::System,
        content: system_prompt.to_owned(),
    }];
    messages.extend(chosen);
    messages
}

fn to_message(stored: StoredMessage) -> Option<Message> {
    let role = Role::parse(&stored.role)?;
    let content = match (role, stored.author) {
        (Role::User, Some(author)) => format!("{author}: {}", stored.content),
        _ => stored.content,
    };
    Some(Message { role, content })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_SYSTEM_PROMPT;

    fn msg(id: i64, role: &str, author: Option<&str>, content: &str) -> StoredMessage {
        StoredMessage {
            id,
            role: role.into(),
            author: author.map(Into::into),
            content: content.into(),
            status: "complete".into(),
            created_at: String::new(),
        }
    }

    #[test]
    fn drops_oldest_when_over_budget() {
        let history = vec![
            msg(1, "user", Some("a"), "first message here"),
            msg(2, "assistant", None, "second"),
            msg(3, "user", Some("b"), "third"),
        ];
        let out = build_context("", history, 20);
        let contents: Vec<_> = out.iter().map(|m| m.content.as_str()).collect();
        assert_eq!(contents[1..], ["second", "b: third"]);
    }

    #[test]
    fn always_keeps_the_newest_message() {
        let out = build_context(
            DEFAULT_SYSTEM_PROMPT,
            vec![msg(1, "user", Some("a"), "far too long for this budget")],
            3,
        );
        assert_eq!(out.len(), 2); // system prompt + the message
    }

    #[test]
    fn labels_users_but_not_the_assistant() {
        let out = build_context(
            DEFAULT_SYSTEM_PROMPT,
            vec![msg(1, "assistant", None, "hi")],
            100,
        );
        assert_eq!(out[1].content, "hi");
    }
}
