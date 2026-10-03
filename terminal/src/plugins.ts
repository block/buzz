import type { RelayEvent } from "./protocol.ts";
import type { Store } from "./store.ts";
import type { TranscriptEntry } from "./transcript.ts";

/** A local terminal action; sending remains in the host-owned delivery path. */
export interface Command {
  name: string;
  description: string;
  run: () => void | Promise<void>;
}

/** A read-only transcript view supplied by a trusted bundled plugin. */
export interface PluginView {
  id: string;
  title: string;
  entries: () => TranscriptEntry[];
}

/** Scoped extension API. Plugins are trusted code, not a security sandbox. */
export interface PluginContext {
  command(command: Command): void;
  view(view: PluginView): void;
  renderer(kind: number, render: (event: RelayEvent) => string): void;
  onChange(listener: () => void): void;
  onDispose(dispose: () => void): void;
}

/** Explicitly loaded extension with deterministic cleanup. */
export interface Plugin {
  id: string;
  activate(context: PluginContext): void;
}

/** Registry for commands, read-only views, and event renderers. */
export class Plugins {
  commands = new Map<string, Command>();
  views = new Map<string, PluginView>();
  renderers = new Map<number, (event: RelayEvent) => string>();
  private active = new Map<string, () => void>();
  private readonly subscribe: (listener: () => void) => () => void;

  constructor(subscribe: (listener: () => void) => () => void) {
    this.subscribe = subscribe;
  }

  /** Activate once; partial registration is rolled back if activation fails. */
  load(plugin: Plugin): void {
    if (this.active.has(plugin.id))
      throw new Error(`Plugin already active: ${plugin.id}`);
    const disposers: (() => void)[] = [];
    const register = <K, V>(map: Map<K, V>, key: K, value: V) => {
      if (map.has(key))
        throw new Error(`Duplicate plugin registration: ${key}`);
      map.set(key, value);
      disposers.push(() => {
        map.delete(key);
      });
    };
    const dispose = () => {
      const errors: unknown[] = [];
      for (const callback of disposers.splice(0).reverse()) {
        try {
          callback();
        } catch (error) {
          errors.push(error);
        }
      }
      this.active.delete(plugin.id);
      if (errors.length)
        throw new AggregateError(errors, `Plugin cleanup failed: ${plugin.id}`);
    };
    try {
      plugin.activate({
        command: (command) => register(this.commands, command.name, command),
        view: (view) => register(this.views, view.id, view),
        renderer: (kind, render) => register(this.renderers, kind, render),
        onChange: (listener) => disposers.push(this.subscribe(listener)),
        onDispose: (callback) => disposers.push(callback),
      });
      this.active.set(plugin.id, dispose);
    } catch (error) {
      dispose();
      throw error;
    }
  }

  /** Remove all contributions and release resources for one plugin. */
  unload(id: string): void {
    this.active.get(id)?.();
  }

  /** Release every plugin on terminal exit. */
  dispose(): void {
    const errors: unknown[] = [];
    for (const dispose of [...this.active.values()].reverse()) {
      try {
        dispose();
      } catch (error) {
        errors.push(error);
      }
    }
    if (errors.length)
      throw new AggregateError(errors, "Plugin cleanup failed");
  }
}

/** First-party activity and diff presentation using the same extension seams. */
export function activityPlugin(
  store: Store,
  show: (id: string) => void,
): Plugin {
  return {
    id: "buzz.activity",
    activate(context) {
      context.command({
        name: "activity",
        description: "Agent activity · Alt+A",
        run: () => show("activity"),
      });
      context.view({
        id: "activity",
        title: "Agent activity · live, owner-only",
        entries: () => {
          const entries: TranscriptEntry[] = [
            {
              id: "help",
              author: "Activity",
              time: "",
              content:
                "These are remote executions, not terminal sessions. Switching views or closing Buzz does not cancel them. Telemetry missed while disconnected cannot be recovered.",
            },
          ];
          for (const item of [...store.activity.values()].reverse()) {
            entries.push({
              id: item.key,
              author: store.name(item.agent),
              authorPubkey: item.agent,
              time: item.state,
              content: `${item.detail}\n\n#${store.channels.get(item.channelId)?.name ?? item.channelId}`,
              detail: `session ${item.sessionId} · turn ${item.turnId}`,
              kind: item.state === "failed" ? "error" : "message",
            });
          }
          if (entries.length === 1)
            entries.push({
              id: "empty",
              author: "Waiting for telemetry",
              time: "",
              content:
                "Activity appears when an owned agent emits a new turn. Other members’ private agent activity is not available.",
            });
          return entries;
        },
      });
      context.renderer(
        40008,
        (event) =>
          `\`\`\`diff\n${event.content.replaceAll("```", "''' ")}\n\`\`\``,
      );
    },
  };
}
