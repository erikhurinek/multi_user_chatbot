import type { ServerMessage, StoredMessage } from "./protocol";

export type PromptState =
  "queued" | "answering" | "answered" | "withdrawn" | "rejected";
export type ReplyState = "streaming" | "complete" | "cancelled" | "failed";

export interface Prompt {
  kind: "prompt";
  key: string;
  messageId: number;
  author: string;
  content: string;
  state: PromptState;
  turnId?: string;
}

export interface Reply {
  kind: "reply";
  key: string;
  content: string;
  state: ReplyState;
}

export type Item = Prompt | Reply;

function fromStored(m: StoredMessage): Item {
  const key = `db-${m.id}`;
  if (m.role === "assistant") {
    return { kind: "reply", key, content: m.content, state: m.status };
  }
  return {
    kind: "prompt",
    key,
    messageId: m.id,
    author: m.author ?? m.role,
    content: m.content,
    state: "answered",
  };
}

function setPrompt(items: Item[], key: string, state: PromptState): Item[] {
  return items.map((i): Item =>
    i.kind === "prompt" && i.key === key ? { ...i, state } : i,
  );
}

function ensureReply(items: Item[], turnId: string): Item[] {
  const exists = items.some((i) => i.kind === "reply" && i.key === turnId);
  return exists
    ? items
    : [
        ...items,
        { kind: "reply", key: turnId, content: "", state: "streaming" },
      ];
}

function settle(items: Item[], turnId: string, state: ReplyState): Item[] {
  return items.map((i): Item => {
    if (i.kind === "reply" && i.key === turnId) return { ...i, state };
    if (i.kind === "prompt" && i.turnId === turnId)
      return { ...i, state: "answered" };
    return i;
  });
}

export function reduce(items: Item[], msg: ServerMessage): Item[] {
  switch (msg.type) {
    case "welcome":
      return items;

    case "history": {
      const pending = new Map(msg.pending.map((p) => [p.message_id, p]));
      const restored = msg.messages.map((m): Item => {
        const item = fromStored(m);
        const p = pending.get(m.id);
        if (item.kind !== "prompt" || !p) return item;
        return {
          ...item,
          key: p.request_id,
          state: p.turn_id ? "answering" : "queued",
          turnId: p.turn_id ?? undefined,
        };
      });

      const turn = msg.turn;
      if (!turn) return restored;

      let at = restored.length;
      restored.forEach((i, n) => {
        if (i.kind === "prompt" && i.turnId === turn.turn_id) at = n + 1;
      });
      const reply: Item = {
        kind: "reply",
        key: turn.turn_id,
        content: turn.reply,
        state: "streaming",
      };
      return [...restored.slice(0, at), reply, ...restored.slice(at)];
    }

    case "user_message":
      if (
        items.some((i) => i.kind === "prompt" && i.messageId === msg.message_id)
      )
        return items;
      return [
        ...items,
        {
          kind: "prompt",
          key: msg.request_id,
          messageId: msg.message_id,
          author: msg.author,
          content: msg.content,
          state: "queued",
        },
      ];
    case "queued":
      return items;
    case "withdrawn":
      return setPrompt(items, msg.request_id, "withdrawn");
    case "rejected":
      return setPrompt(items, msg.request_id, "rejected");
    case "started": {
      const ids = new Set(msg.request_ids);
      const marked = items.map((i): Item =>
        i.kind === "prompt" && ids.has(i.key)
          ? { ...i, state: "answering", turnId: msg.turn_id }
          : i,
      );
      return ensureReply(marked, msg.turn_id);
    }
    case "token":
      return ensureReply(items, msg.turn_id).map((i): Item =>
        i.kind === "reply" && i.key === msg.turn_id
          ? { ...i, content: i.content + msg.text }
          : i,
      );
    case "finished":
      return settle(items, msg.turn_id, "complete");
    case "cancelled":
      return settle(items, msg.turn_id, "cancelled");
    case "failed":
      return settle(items, msg.turn_id, "failed");
    default: {
      const unhandled: never = msg;
      return items;
    }
  }
}
