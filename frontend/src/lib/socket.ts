import type { ClientMessage, ServerMessage } from "./protocol";

export type Status = "connecting" | "open" | "closed";

export function connect(
  url: string,
  onMessage: (msg: ServerMessage) => void,
  onStatus: (status: Status) => void,
) {
  let socket: WebSocket | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let attempt = 0;
  let stopped = false;

  const open = () => {
    onStatus("connecting");
    const ws = new WebSocket(url);
    socket = ws;

    ws.onopen = () => {
      attempt = 0;
      onStatus("open");
    };
    ws.onmessage = (event) => {
      try {
        onMessage(JSON.parse(event.data) as ServerMessage);
      } catch (error) {
        console.warn("ingoring unreadable server message", error);
      }
    };
    ws.onclose = () => {
      onStatus("closed");
      if (stopped) return;
      const delay = Math.min(1000 * 2 ** attempt, 10_000);
      attempt++;
      timer = setTimeout(open, delay);
    };
  };

  open();

  return {
    send(msg: ClientMessage): boolean {
      if (socket?.readyState !== WebSocket.OPEN) return false;
      socket.send(JSON.stringify(msg));
      return true;
    },
    close() {
      stopped = true;
      clearTimeout(timer);
      socket?.close();
    },
  };
}
