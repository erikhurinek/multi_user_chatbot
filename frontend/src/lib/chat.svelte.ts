import { connect, type Status } from "./socket";
import { reduce, type Item } from "./reducer";

export function createChat(url: string) {
  let items = $state<Item[]>([]);
  let status = $state<Status>("connecting");
  let me = $state<string | null>(null);

  const connection = connect(
    url,
    (msg) => {
      if (msg.type === "welcome") me = msg.author;
      items = reduce(items, msg);
    },
    (s) => {
      status = s;
    },
  );

  return {
    get items() {
      return items;
    },
    get status() {
      return status;
    },
    submit: (prompt: string) => connection.send({ type: "submit", prompt }),
    cancel: (id: string) => connection.send({ type: "cancel", id }),
    close: connection.close,
  };
}
