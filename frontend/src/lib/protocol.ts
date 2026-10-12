export interface StoredMessage {
  id: number;
  role: "user" | "assistant" | "system";
  author: string | null;
  content: string;
  status: "complete" | "cancelled" | "failed";
  created_at: string;
}

export interface Pending {
  request_id: string;
  message_id: number;
  turn_id: string | null;
}

export interface RunningTurn {
  turn_id: string;
  reply: string;
}

export type ServerMessage =
  | { type: "welcome"; author: string }
  | {
      type: "history";
      messages: StoredMessage[];
      pending: Pending[];
      turn: RunningTurn | null;
    }
  | {
      type: "user_message";
      request_id: string;
      message_id: number;
      author: string;
      content: string;
    }
  | { type: "queued"; request_id: string }
  | { type: "withdrawn"; request_id: string }
  | { type: "rejected"; request_id: string; reason: string }
  | { type: "started"; turn_id: string; request_ids: string[] }
  | { type: "token"; turn_id: string; text: string }
  | { type: "finished"; turn_id: string }
  | { type: "cancelled"; turn_id: string }
  | { type: "failed"; turn_id: string; reason: string };

export type ClientMessage =
  { type: "submit"; prompt: string } | { type: "cancel"; id: string };
