import {
  type Component,
  Editor,
  matchesKey,
  type OverlayHandle,
  parseKey,
  ScrollView,
  type Terminal,
  TuiAltScreen,
  VStack,
  visibleWidth,
} from "@earendil-works/pi-tui";
import type { Launch } from "./launch.ts";
import { Picker, type PickerItem } from "./picker.ts";
import { activityPlugin, type Command, Plugins } from "./plugins.ts";
import { Session } from "./session.ts";
import { type Conversation, Store } from "./store.ts";
import { editorTheme, fit, singleLine, style } from "./theme.ts";
import { Transcript, type TranscriptEntry } from "./transcript.ts";
import type { Transport } from "./transport.ts";

/** Editor-centered terminal shell. Remote execution is owned by Session's relay. */
export class TerminalApp {
  readonly store = new Store();
  readonly tui: TuiAltScreen;
  readonly editor: Editor;
  readonly session: Session;
  readonly plugins: Plugins;
  private readonly transcript = new Transcript();
  private readonly scrolls = new Map<string, ScrollView>();
  private scroll: ScrollView;
  private overlay?: OverlayHandle;
  private pluginView?: string;
  private editorKey?: string;
  private updating = false;
  private stopped = false;
  private readonly dispose: () => void;
  private readonly onExit: () => void;
  private commandDraft = "";
  private startupDraft = "";
  private quitArmedAt = 0;

  constructor(
    terminal: Terminal,
    transport: Transport,
    onExit: () => void,
    launch?: Launch,
  ) {
    this.onExit = onExit;
    this.tui = new TuiAltScreen(terminal, true, undefined, {
      scrollToEndIndicator: () => style.accent(" ↓ Latest · Ctrl+L "),
      copyOnSelect: false,
    });
    this.scroll = new ScrollView(this.transcript, {
      follow: "end",
      primary: true,
      scrollbar: "auto",
    });
    this.editor = new Editor(this.tui, editorTheme, { paddingX: 1 });
    this.editor.disableSubmit = true;
    this.editor.onChange = () => {
      if (this.updating) return;
      const view = this.store.current;
      if (view && !this.pluginView) view.draft = this.editor.getExpandedText();
      else if (this.session.startup && !this.pluginView)
        this.startupDraft = this.editor.getExpandedText();
      else this.commandDraft = this.editor.getExpandedText();
    };
    this.session = new Session(this.store, transport, launch);
    this.plugins = new Plugins((listener) => this.store.subscribe(listener));
    this.plugins.load({
      id: "buzz.navigation",
      activate: (context) => {
        for (const command of this.commands()) context.command(command);
      },
    });
    this.plugins.load(
      activityPlugin(this.store, (id) => {
        this.pluginView = id;
        this.refresh();
      }),
    );
    this.dispose = this.store.subscribe(() => this.refresh());
    this.tui.addInputListener((data) => {
      const key = parseKey(data);
      if (key === "ctrl+c" || key === "ctrl+q") {
        this.requestQuit();
        return { consume: true };
      }
      if (this.tui.hasOverlay()) return;
      if (matchesKey(data, "ctrl+j") || matchesKey(data, "shift+enter")) return;
      const action =
        key === "ctrl+k"
          ? () => this.contexts()
          : key === "ctrl+r"
            ? () => this.recipients()
            : key === "ctrl+g"
              ? () => this.commandPalette()
              : key === "ctrl+t"
                ? () => this.threads()
                : key === "alt+a"
                  ? () => {
                      this.pluginView = this.pluginView
                        ? undefined
                        : "activity";
                      this.refresh();
                    }
                  : key === "ctrl+l"
                    ? () => this.scroll.scrollToEnd()
                    : key === "escape" && this.pluginView
                      ? () => {
                          this.pluginView = undefined;
                          this.refresh();
                        }
                      : key === "enter"
                        ? () => this.submit()
                        : undefined;
      if (action) {
        try {
          action();
        } catch (error) {
          this.report(error);
        }
        return { consume: true };
      }
    });
    this.refresh();
  }

  /** Enter alternate-screen mode and connect the selected transport. */
  start(): void {
    this.tui.start();
    this.tui.setFocus(this.editor);
    this.session.start();
  }

  private commands(): Command[] {
    return [
      {
        name: "switch",
        description: "Switch conversation · Ctrl+K",
        run: () => this.contexts(),
      },
      {
        name: "to",
        description: "Choose an exact recipient · Ctrl+R",
        run: () => this.recipients(),
      },
      {
        name: "threads",
        description: "Open a thread from this conversation · Ctrl+T",
        run: () => this.threads(),
      },
      {
        name: "chat",
        description: "Return to the conversation · Esc",
        run: () => {
          this.pluginView = undefined;
          this.refresh();
        },
      },
      {
        name: "latest",
        description: "Jump to latest output · Ctrl+L",
        run: () => this.scroll.scrollToEnd(),
      },
      {
        name: "retry",
        description: "Retry the identical unresolved message",
        run: async () => {
          if (!this.store.current)
            throw new Error("Choose a conversation first.");
          await this.store.send(
            this.store.current,
            this.session.transport,
            true,
          );
        },
      },
      {
        name: "discard",
        description:
          "Forget an unresolved send after inspecting the conversation",
        run: () => {
          const view = this.store.current;
          if (!view?.pending || view.sending)
            throw new Error("No unresolved send is available to discard.");
          this.pick(
            "Delivery may have succeeded. Forget this send?",
            [
              {
                id: "keep",
                label: "Keep the pending message",
                detail: "Recommended until you have checked the conversation",
              },
              {
                id: "discard",
                label: "Forget delivery tracking",
                detail:
                  "Does not undo a message. Sending again may duplicate it.",
              },
            ],
            (item) => {
              if (item.id === "discard") {
                view.pending = undefined;
                view.error = undefined;
                this.refresh();
              }
            },
          );
        },
      },
      {
        name: "retry-setup",
        description:
          "Retry setup of this launch's channel without creating another",
        run: async () => {
          await this.session.startup?.retry();
        },
      },
      {
        name: "reconnect",
        description: "Retry the relay connection and refresh recent history",
        run: async () => {
          await this.session.transport.request("reconnect", {});
        },
      },
      {
        name: "close",
        description: "Close this view without cancelling the agent",
        run: () => {
          const view = this.store.current;
          if (!view) return;
          if (view.draft || view.pending || view.sending)
            throw new Error(
              "This conversation has a draft or pending send. Resolve it before closing.",
            );
          this.store.views.delete(view.key);
          this.store.messages.delete(view.key);
          this.scrolls.delete(view.key);
          this.store.activeKey = this.store.views.keys().next().value;
          this.store.changed();
        },
      },
      {
        name: "help",
        description: "Keyboard shortcuts and client boundaries",
        run: () => this.help(),
      },
      {
        name: "quit",
        description: "Detach from Buzz · Ctrl+Q",
        run: () => this.requestQuit(),
      },
    ];
  }

  private submit(): void {
    const text = this.editor.getExpandedText();
    if (text.startsWith("/") && !text.startsWith("//")) {
      const command = this.plugins.commands.get(text.trim().slice(1));
      if (!command) {
        this.report(
          new Error(
            "Unknown command. Ctrl+G lists commands; start with // to send a literal slash.",
          ),
        );
        return;
      }
      this.editor.setText("");
      void Promise.resolve()
        .then(() => command.run())
        .catch((error) => this.report(error));
      return;
    }
    const view = this.store.current;
    if (!view || this.pluginView) {
      this.contexts();
      return;
    }
    view.draft = text.startsWith("//") ? text.slice(1) : text;
    void this.store
      .send(view, this.session.transport)
      .catch((error) => this.report(error));
  }

  private pick(
    title: string,
    items: PickerItem[],
    select: (item: PickerItem) => void,
  ): void {
    this.overlay?.hide();
    const close = () => {
      this.overlay?.hide();
      this.overlay = undefined;
      this.tui.setFocus(this.editor);
    };
    const picker = new Picker({
      title,
      items,
      onCancel: close,
      availableRows: () =>
        Math.max(3, Math.floor(this.tui.terminal.rows * 0.9) - 5),
      requestRender: () => this.tui.requestRender(),
      onSelect: (item) => {
        close();
        try {
          select(item);
        } catch (error) {
          this.report(error);
        }
      },
    });
    const framed: Component = {
      render: (width) => {
        const innerWidth = Math.max(1, width - 4);
        const lines = picker.render(innerWidth);
        const border = style.muted("─".repeat(Math.max(0, width - 2)));
        return [
          style.muted("┌") + border + style.muted("┐"),
          ...lines.map(
            (line) =>
              style.muted("│") +
              " " +
              line +
              " ".repeat(Math.max(0, width - visibleWidth(line) - 3)) +
              style.muted("│"),
          ),
          style.muted("│") +
            fit(
              " ↑↓ select · Enter open · Esc back",
              Math.max(1, width - 2),
            ).padEnd(Math.max(1, width - 2)) +
            style.muted("│"),
          style.muted("└") + border + style.muted("┘"),
        ];
      },
      invalidate: () => picker.invalidate(),
      handleInput: (data) => picker.handleInput(data),
    };
    picker.focused = true;
    this.overlay = this.tui.showOverlay(framed, {
      width: "85%",
      maxHeight: "90%",
      anchor: "center",
      margin: 1,
    });
  }

  private contexts(): void {
    const items: PickerItem[] = [];
    for (const view of this.store.views.values()) {
      items.push({
        id: view.key,
        label: `#${this.store.channels.get(view.channelId)?.name ?? view.channelId}${view.rootEventId ? ` / thread ${view.rootEventId.slice(0, 8)}` : ""}`,
        detail: view.recipient
          ? `To ${this.store.name(view.recipient)} · ${view.recipient.slice(0, 12)}`
          : "Choose a recipient with Ctrl+R",
        badge: [
          view.key === this.store.activeKey ? "current" : "",
          view.unread ? `${view.unread} new` : "",
          view.draft ? "draft" : "",
        ]
          .filter(Boolean)
          .join(" · "),
      });
    }
    for (const channel of this.store.channels.values()) {
      if (channel.joined && !this.store.views.has(`${channel.id}:`))
        items.push({
          id: channel.id,
          label: `#${channel.name}`,
          detail: `${channel.members.length} members · ${channel.id}`,
        });
    }
    this.pick("Conversations", items, (item) => {
      const existing = this.store.views.get(item.id);
      this.pluginView = undefined;
      this.store.open(existing?.channelId ?? item.id, existing?.rootEventId);
    });
  }

  private recipients(): void {
    const view = this.store.current;
    if (!view) {
      this.contexts();
      return;
    }
    const members = this.store.channels.get(view.channelId)?.members ?? [];
    const startup = this.session.startup;
    const canInvite =
      startup?.launch.mode === "new" &&
      startup.launch.channelId === view.channelId;
    const candidates = [
      ...new Set([
        ...members,
        ...(canInvite
          ? [...this.store.profiles.values()]
              .filter((profile) => profile.owner === this.store.pubkey)
              .map((profile) => profile.pubkey)
          : []),
      ]),
    ];
    this.pick(
      "Send as you · choose one recipient",
      candidates
        .filter((key) => key !== this.store.pubkey)
        .map((key) => ({
          id: key,
          label: this.store.name(key),
          detail: key,
          badge: !members.includes(key)
            ? "invite your agent"
            : this.store.profiles.get(key)?.owner === this.store.pubkey
              ? "your agent"
              : "member",
        })),
      (item) => {
        if (startup?.launch.channelId === view.channelId && !view.rootEventId)
          void startup
            .selectAgent(view, item.id)
            .catch((error) => this.report(error));
        else this.store.selectRecipient(view, item.id);
      },
    );
  }

  private threads(): void {
    const view = this.store.current;
    if (!view) {
      this.contexts();
      return;
    }
    const channelView = {
      ...view,
      key: `${view.channelId}:`,
      rootEventId: undefined,
    };
    this.pick(
      "Open a thread · recent messages",
      this.store
        .events(channelView)
        .slice()
        .reverse()
        .map((event) => ({
          id: event.id,
          label: singleLine(event.content).slice(0, 120),
          detail: `${this.store.name(event.pubkey)} · ${event.id.slice(0, 16)}`,
        })),
      (item) => {
        this.pluginView = undefined;
        this.store.open(view.channelId, item.id);
      },
    );
  }

  private commandPalette(): void {
    this.pick(
      "Commands",
      [...this.plugins.commands.values()].map((command) => ({
        id: command.name,
        label: `/${command.name}`,
        detail: command.description,
      })),
      (item) => {
        void Promise.resolve()
          .then(() => this.plugins.commands.get(item.id)?.run())
          .catch((error) => this.report(error));
      },
    );
  }

  private help(): void {
    this.pick(
      "Buzz · keyboard guide",
      [
        {
          id: "switch",
          label: "Ctrl+K  Conversations",
          detail: "Each keeps its own draft, recipient, and scroll position",
        },
        {
          id: "to",
          label: "Ctrl+R  Recipient",
          detail: "Exact identity, not a display-name match",
        },
        {
          id: "activity",
          label: "Alt+A  Agent activity",
          detail: "Owner-only, live telemetry. Esc returns to chat.",
        },
        {
          id: "threads",
          label: "Ctrl+T  Threads",
          detail: "Open a thread without moving other agent sessions",
        },
        {
          id: "chat",
          label: "Enter sends · Shift+Enter / Ctrl+J adds a line",
          detail: "PgUp/PgDn scroll · Ctrl+L latest · Ctrl+Shift+F search",
        },
        {
          id: "chat",
          label: "Drafts are in memory for this terminal session",
          detail:
            "No shell tools or agent permissions are granted by the terminal",
        },
      ],
      () => {},
    );
  }

  private refresh(): void {
    if (this.stopped) return;
    const view = this.store.current;
    const startupView = this.session.startup
      ? this.store.views.get(`${this.session.startup.launch.channelId}:`)
      : undefined;
    if (startupView && this.startupDraft && !startupView.draft) {
      startupView.draft = this.startupDraft;
      this.startupDraft = "";
    }
    const key = this.pluginView
      ? `plugin:${this.pluginView}`
      : (view?.key ?? "welcome");
    if (this.editorKey !== key) {
      this.editorKey = key;
      this.updating = true;
      this.editor.setText(
        this.pluginView
          ? this.commandDraft
          : (view?.draft ?? this.startupDraft),
      );
      this.updating = false;
      let scroll = this.scrolls.get(key);
      if (!scroll) {
        scroll = new ScrollView(this.transcript, {
          follow: "end",
          primary: true,
          scrollbar: "auto",
        });
        this.scrolls.set(key, scroll);
      }
      this.scroll = scroll;
    } else if (
      view &&
      !this.pluginView &&
      this.editor.getExpandedText() !== view.draft
    ) {
      this.updating = true;
      this.editor.setText(view.draft);
      this.updating = false;
    }
    this.transcript.setEntries(this.entries(view));
    this.tui.setLayoutRoot(
      new VStack([
        this.chrome((width) => this.header(width)),
        { component: this.scroll, basis: 0, grow: 1, minSize: 1 },
        this.chrome((width) => this.recipientLine(width)),
        {
          component: this.editor,
          basis: "auto",
          minSize: 3,
          maxSize: 9,
          shrink: 1,
        },
        this.chrome((width) => this.footer(width)),
      ]),
    );
    this.tui.requestRender();
  }

  private chrome(render: (width: number) => string[]): Component {
    return { render, invalidate() {} };
  }

  private entries(view?: Conversation): TranscriptEntry[] {
    if (this.pluginView)
      return this.plugins.views.get(this.pluginView)?.entries() ?? [];
    if (!view)
      return [
        {
          id: "welcome",
          author: "Buzz",
          time: "your agents, one terminal",
          content: this.session.startup
            ? `${this.session.startup.launch.mode === "new" ? "Creating a private channel" : "Opening your channel"}…\n\nStart typing below while Buzz connects. Your draft stays here.\n\nResume this channel with:\n\nbuzz join ${this.session.startup.launch.channelId}\n\nIf setup fails, use **/retry-setup** to retry the same channel.`
            : "Your agents keep working.\n\n**Ctrl+K** opens a conversation. **Ctrl+R** chooses who receives your message.\n\nMove between contexts without losing your place. Drafts and recipients stay with their conversation; agents keep running when you leave.",
          detail: "Ctrl+G commands · Alt+A activity · /help",
        },
      ];
    const entries: TranscriptEntry[] = this.store.events(view).map((event) => ({
      id: event.id,
      author: this.store.name(event.pubkey),
      self: event.pubkey === this.store.pubkey,
      time: new Date(event.created_at * 1000).toLocaleTimeString([], {
        hour: "2-digit",
        minute: "2-digit",
      }),
      content: this.plugins.renderers.get(event.kind)?.(event) ?? event.content,
    }));
    for (const activity of this.store.activity.values()) {
      if (activity.channelId === view.channelId && activity.state === "working")
        entries.push({
          id: activity.key,
          author: this.store.name(activity.agent),
          time: "",
          kind: "activity",
          content: `${this.store.name(activity.agent)} · ${activity.detail}`,
          detail: "channel activity · Alt+A details",
        });
    }
    if (view.pending)
      entries.push({
        id: "pending",
        author: "You",
        time: view.sending ? "sending…" : "delivery unresolved",
        content: view.pending.content,
        detail: view.sending
          ? "Waiting for the relay acknowledgement"
          : "Use /retry for the identical event, or /discard after checking history.",
      });
    if (view.error)
      entries.push({
        id: "error",
        author: "Delivery / connection",
        time: "",
        content: view.error,
        kind: "error",
      });
    if (!entries.length && view.history === "ready")
      entries.push({
        id: "empty-channel",
        author: "Ready for your first prompt",
        time: "",
        content: view.recipient
          ? `Messages here go to **${singleLine(this.store.name(view.recipient))}**. Type below to begin.`
          : "Type your prompt below. **Ctrl+R** chooses an agent before you send.",
        detail: `Resume with buzz join ${view.channelId}`,
      });
    return entries;
  }

  private header(width: number): string[] {
    const view = this.store.current;
    const community = this.store.relayUrl
      ? new URL(this.store.relayUrl).host
      : "connecting";
    const title = this.pluginView
      ? this.plugins.views.get(this.pluginView)?.title
      : view
        ? `#${this.store.channels.get(view.channelId)?.name ?? view.channelId}${view.rootEventId ? ` / thread ${view.rootEventId.slice(0, 8)}` : ""}`
        : "Conversations";
    const status = this.store.relayUrl.startsWith("demo:")
      ? "DEMO / OFFLINE"
      : this.store.connection;
    const left = ` ${style.bold(style.accent("buzz"))} ${style.muted("/ ")} ${singleLine(community)}  ${style.muted("/")}  ${singleLine(title ?? "")}`;
    const right = ` ${this.store.connection === "connected" ? style.success("●") : style.error("○")} ${status} `;
    const room = Math.max(0, width - visibleWidth(right));
    const clipped = fit(left, room);
    return [
      clipped +
        " ".repeat(Math.max(0, room - visibleWidth(clipped))) +
        fit(right, width),
      style.muted("─".repeat(width)),
      "",
    ];
  }

  private recipientLine(width: number): string[] {
    const view = this.store.current;
    const label = this.pluginView
      ? "Read-only view · Esc returns to your draft"
      : view?.recipient
        ? `To ${this.store.name(view.recipient)} · ${view.recipient.slice(0, 12)}  /  as you`
        : "Choose a recipient · Ctrl+R";
    return ["", fit(` ${style.accent(singleLine(label))}`, width)];
  }

  private footer(width: number): string[] {
    const working = [...this.store.activity.values()].filter(
      (item) => item.state === "working",
    ).length;
    const unread = [...this.store.views.values()].reduce(
      (sum, view) => sum + view.unread,
      0,
    );
    const keys =
      width >= 90
        ? " ^K contexts  ^R recipient  ^G commands  ⌥A activity  Enter send  ^J newline"
        : " ^K switch  ^R to  ^G commands";
    const activity = `${working ? `${working} working` : ""}${unread ? ` · ${unread} new` : ""}`;
    return [
      fit(
        style.muted(keys) + (activity ? `  ${style.accent(activity)}` : ""),
        width,
      ),
      fit(` ${style.muted(singleLine(this.store.notice))}`, width),
    ];
  }

  private report(error: unknown): void {
    this.store.notice =
      error instanceof Error ? error.message : "Action failed";
    this.refresh();
  }

  private requestQuit(): void {
    const hasDraft =
      this.commandDraft ||
      this.startupDraft ||
      [...this.store.views.values()].some((view) => view.draft || view.pending);
    if (hasDraft && Date.now() - this.quitArmedAt > 3000) {
      this.quitArmedAt = Date.now();
      this.store.notice =
        "Unsaved drafts / unresolved sends. Press Ctrl+Q again within 3 seconds to discard local state and detach.";
      this.refresh();
      return;
    }
    this.stop();
    this.onExit();
  }

  /** Restore terminal modes and release all subscriptions and plugin resources. */
  stop(): void {
    if (this.stopped) return;
    this.stopped = true;
    this.dispose();
    this.session.close();
    try {
      this.plugins.dispose();
    } finally {
      this.tui.stop({ preserveScreen: true });
    }
  }
}
