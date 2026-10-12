import { expect, test } from "bun:test";
import { reduce, type Item } from "./reducer";
import { groupItems } from "./group";

test("tokens accumulate into one reply, even for a late joiner", () => {
  let items: Item[] = [];
  items = reduce(items, { type: "token", turn_id: "t1", text: "Hel" });
  items = reduce(items, { type: "token", turn_id: "t1", text: "lo" });
  expect(items).toEqual([
    { kind: "reply", key: "t1", content: "Hello", state: "streaming" },
  ]);
});

test("a turn answers every prompt in its batch", () => {
  let items: Item[] = [];
  items = reduce(items, {
    type: "user_message",
    request_id: "a",
    message_id: 1,
    author: "alice",
    content: "hi",
  });
  items = reduce(items, {
    type: "user_message",
    request_id: "b",
    message_id: 2,
    author: "bob",
    content: "yo",
  });
  items = reduce(items, {
    type: "started",
    turn_id: "t",
    request_ids: ["a", "b"],
  });
  items = reduce(items, { type: "finished", turn_id: "t" });
  expect(
    items.filter((i) => i.kind === "prompt" && i.state === "answered"),
  ).toHaveLength(2);
});

test("joining mid-turn restores queued prompts, the running turn and its text", () => {
  const stored = (id: number, author: string) =>
    ({
      id,
      role: "user",
      author,
      content: "x",
      status: "complete",
      created_at: "",
    }) as const;

  const items = reduce([], {
    type: "history",
    messages: [stored(1, "alice"), stored(2, "bob")],
    pending: [
      { request_id: "a", message_id: 1, turn_id: "t" },
      { request_id: "b", message_id: 2, turn_id: null },
    ],
    turn: { turn_id: "t", reply: "Hel" },
  });

  expect(items.map((i) => [i.key, i.state])).toEqual([
    ["a", "answering"],
    ["t", "streaming"],
    ["b", "queued"],
  ]);

  const next = reduce(items, { type: "token", turn_id: "t", text: "lo" });
  expect(next[1]).toMatchObject({ content: "Hello" });
});

test("a user_message already in the snapshot is ignored", () => {
  const msg = {
    type: "user_message",
    request_id: "a",
    message_id: 7,
    author: "al",
    content: "hi",
  } as const;
  const once = reduce([], msg);
  expect(reduce(once, msg)).toHaveLength(1);
});

test("consecutive messages by one speaker share a group", () => {
  let items: Item[] = [];
  for (const [id, who] of [
    ["a", "al"],
    ["b", "al"],
    ["c", "bo"],
  ] as const) {
    items = reduce(items, {
      type: "user_message",
      request_id: id,
      message_id: items.length + 1,
      author: who,
      content: id,
    });
  }
  const groups = groupItems(items, "al");
  expect(groups.map((g) => [g.label, g.items.length, g.own])).toEqual([
    ["al", 2, true],
    ["bo", 1, false],
  ]);
});
