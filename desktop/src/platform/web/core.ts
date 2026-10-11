import { handlers, nativeOnly } from "./commands";
import { wsClose, wsCloseAll, wsConnect, wsSend } from "./websocket";

export class Channel<T = unknown> {
  id = Math.floor(Math.random() * 2 ** 31);
  onmessage: (m: T) => void;
  constructor(cb?: (m: T) => void) {
    this.onmessage = cb ?? (() => {});
  }
}
export const isTauri = () => false;
export const convertFileSrc = (p: string) => p;

export async function invoke<T = unknown>(
  cmd: string,
  args?: Record<string, any>,
  options?: { headers?: Record<string, string> },
): Promise<T> {
  if (import.meta.env.DEV) console.info(`[invoke] ${cmd}`);
  switch (cmd) {
    case "plugin:websocket|connect":
      return (await wsConnect(args!.url, args!.onMessage)) as T;
    case "plugin:websocket|send":
      return wsSend(args!.id, args!.message) as T;
    case "plugin:websocket|disconnect":
      return wsClose(args!.id) as T;
    case "plugin:websocket|disconnect_all":
      return wsCloseAll() as T;
  }
  const h = handlers[cmd];
  if (!h) {
    console.debug(`[web] native-only command: ${cmd}`);
    throw nativeOnly(cmd);
  }
  try {
    return (await h(args, options)) as T;
  } catch (e) {
    if (import.meta.env.DEV)
      console.warn(
        `[invoke-error] ${cmd}: ${e instanceof Error ? e.message : String(e)}`,
      );
    throw e;
  }
}

// DEV-only test hook (stripped from production builds) so e2e can exercise registry commands directly.
if (import.meta.env.DEV) {
  const w = window as unknown as {
    __buzzWebInvoke?: typeof invoke;
    __buzzWebHandlers?: string[];
  };
  w.__buzzWebInvoke = invoke;
  w.__buzzWebHandlers = Object.keys(handlers);
}
