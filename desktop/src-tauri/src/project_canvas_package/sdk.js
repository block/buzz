// Host-owned Project Canvas SDK. Served from /__buzz/sdk.js and loaded before
// every package script, so packages can rely on `window.buzzCanvas.sdk`.
//
// The SDK never starts the MessagePort: the package entry (canvas.js) owns
// starting it, and listeners registered before the port starts lose nothing.
// RPC calls issued before host.init arrives are queued and flushed with the
// session envelope once the host binds the load.
(() => {
  const PROTOCOL_VERSION = 1;
  const runtime = window.buzzCanvas;
  if (
    !runtime ||
    runtime.protocolVersion !== PROTOCOL_VERSION ||
    !runtime.port ||
    !runtime.sdk
  ) {
    return;
  }
  const port = runtime.port;
  const session = {
    capabilities: [],
    loadId: null,
    nonce: null,
    ready: false,
  };
  let requestCounter = 0;
  const pending = new Map();
  const subscriptions = new Map();
  const queued = [];

  function nextId(prefix) {
    requestCounter += 1;
    return `${prefix}-${requestCounter}`;
  }

  function envelope() {
    return {
      loadId: session.loadId,
      nonce: session.nonce,
      protocolVersion: PROTOCOL_VERSION,
    };
  }

  function send(message) {
    if (session.ready) port.postMessage(Object.assign(envelope(), message));
    else queued.push(message);
  }

  function rpcFailure(error) {
    const failure = new Error(
      error && typeof error.message === "string"
        ? error.message
        : "Canvas request failed",
    );
    failure.code =
      error && typeof error.code === "string" ? error.code : "failed";
    return failure;
  }

  function settle(id, message) {
    const entry = pending.get(id);
    if (!entry) return;
    pending.delete(id);
    if (message.error) entry.reject(rpcFailure(message.error));
    else
      entry.resolve(
        message.result !== undefined ? message.result : { ok: true },
      );
  }

  port.addEventListener("message", (event) => {
    const message = event.data;
    if (!message || message.protocolVersion !== PROTOCOL_VERSION) return;
    if (message.type === "host.init") {
      if (session.ready) return;
      session.ready = true;
      session.loadId = message.loadId;
      session.nonce = message.nonce;
      session.capabilities = Array.isArray(message.capabilities)
        ? message.capabilities.slice()
        : [];
      for (const queuedMessage of queued.splice(0)) {
        port.postMessage(Object.assign(envelope(), queuedMessage));
      }
      return;
    }
    if (message.loadId !== session.loadId || message.nonce !== session.nonce) {
      return;
    }
    if (message.type === "host.queryResult") settle(message.queryId, message);
    else if (message.type === "host.commandResult") {
      settle(message.commandId, message);
    } else if (message.type === "host.openResult") {
      settle(message.openId, message);
    } else if (message.type === "host.subscriptionUpdate") {
      const subscription = subscriptions.get(message.subscriptionId);
      if (subscription) subscription(message.result);
    } else if (message.type === "host.subscriptionEnded") {
      const subscription = subscriptions.get(message.subscriptionId);
      subscriptions.delete(message.subscriptionId);
      if (subscription && message.error) {
        subscription({ data: null, error: message.error, status: "error" });
      }
    }
  });

  function request(prefix, build) {
    return new Promise((resolve, reject) => {
      const id = nextId(prefix);
      pending.set(id, { reject, resolve });
      send(build(id));
    });
  }

  const data = Object.freeze({
    query(name, params) {
      return request("q", (queryId) => ({
        query: { name, params: params || {} },
        queryId,
        type: "canvas.query",
      }));
    },
    liveQuery(name, params, onUpdate) {
      const subscriptionId = nextId("s");
      subscriptions.set(subscriptionId, onUpdate);
      send({
        query: { name, params: params || {} },
        subscriptionId,
        type: "canvas.subscribe",
      });
      return () => {
        if (!subscriptions.delete(subscriptionId)) return;
        send({ subscriptionId, type: "canvas.unsubscribe" });
      };
    },
    command(name, params) {
      return request("c", (commandId) => ({
        command: { name, params: params || {} },
        commandId,
        type: "canvas.command",
      }));
    },
  });

  const app = Object.freeze({
    open(target) {
      return request("o", (openId) => ({
        openId,
        target,
        type: "canvas.open",
      }));
    },
  });

  // --- Layout persistence --------------------------------------------------
  // The host persists widget placement per dashboard; no capability is needed
  // because it only records direct user manipulation of host-rendered chrome.
  // Sends are debounced here so a held arrow key cannot trip the host's port
  // rate limit, which tears the frame down rather than failing one message.

  const LAYOUT_DEBOUNCE_MS = 300;
  const LAYOUT_COORDINATE_LIMIT = 100000;
  const LAYOUT_MIN_WIDGET_SIZE = 16;
  const LAYOUT_MAX_WIDGETS = 256;
  let layoutTimer = 0;
  let layoutPending = null;

  function layoutCoordinate(value) {
    const numeric = Number(value);
    if (!Number.isFinite(numeric)) return null;
    return Math.max(
      -LAYOUT_COORDINATE_LIMIT,
      Math.min(LAYOUT_COORDINATE_LIMIT, numeric),
    );
  }

  function layoutPoint(value) {
    if (!value || typeof value !== "object") return null;
    const x = layoutCoordinate(value.x);
    const y = layoutCoordinate(value.y);
    return x === null || y === null ? null : { x, y };
  }

  // Clamped into the host's accepted range so a save can never produce an
  // invalid port message (those count toward frame teardown).
  function layoutDimension(value) {
    const numeric = Number(value);
    if (!Number.isFinite(numeric)) return null;
    return Math.max(
      LAYOUT_MIN_WIDGET_SIZE,
      Math.min(LAYOUT_COORDINATE_LIMIT, numeric),
    );
  }

  function layoutSize(value) {
    if (!value || typeof value !== "object") return null;
    const width = layoutDimension(value.width);
    const height = layoutDimension(value.height);
    return width === null || height === null ? null : { height, width };
  }

  function layoutOverrides(value, sanitize) {
    const overrides = {};
    if (!value || typeof value !== "object") return overrides;
    let count = 0;
    for (const [widgetId, entry] of Object.entries(value)) {
      if (count >= LAYOUT_MAX_WIDGETS) break;
      if (!/^[A-Za-z0-9._-]{1,128}$/.test(widgetId)) continue;
      const sanitized = sanitize(entry);
      if (!sanitized) continue;
      Object.defineProperty(overrides, widgetId, {
        configurable: true,
        enumerable: true,
        value: sanitized,
        writable: true,
      });
      count += 1;
    }
    return overrides;
  }

  function flushLayout() {
    layoutTimer = 0;
    const next = layoutPending;
    layoutPending = null;
    if (next) send(next);
  }

  const layout = Object.freeze({
    save(next) {
      const options = next || {};
      const dashboard = String(options.dashboard || "");
      if (!dashboard || dashboard.length > 128) return;
      // Last write wins: only the final arrangement of a burst is sent.
      layoutPending = {
        dashboard,
        pan: layoutPoint(options.pan),
        sizes: layoutOverrides(options.sizes, layoutSize),
        type: "canvas.layout",
        widgets: layoutOverrides(options.widgets, layoutPoint),
      };
      clearTimeout(layoutTimer);
      layoutTimer = setTimeout(flushLayout, LAYOUT_DEBOUNCE_MS);
    },
  });

  // --- Standard components -------------------------------------------------
  // Identity semantics mirror the app's UserAvatar: same initials derivation
  // and the same 7-tone hash, so a person renders identically inside and
  // outside the canvas.

  function initialsFor(name) {
    return String(name || "")
      .replace(/[^\p{L}\p{N}\s]/gu, " ")
      .trim()
      .split(/\s+/)
      .map((part) => part[0] || "")
      .join("")
      .slice(0, 2)
      .toUpperCase();
  }

  function toneFor(name) {
    let hash = 0;
    for (const character of String(name || "")
      .trim()
      .toLowerCase()) {
      hash = (hash * 31 + (character.codePointAt(0) || 0)) >>> 0;
    }
    return hash % 7;
  }

  function el(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined && text !== null) node.textContent = String(text);
    return node;
  }

  const PUBKEY_PATTERN = /^[0-9a-f]{64}$/;

  /**
   * Resolves where an avatar's picture comes from, if anywhere.
   *
   * A `pubkey` uses the host's avatar route, which the frame fetches like any
   * ordinary image — so the picture costs nothing in the RPC payload that
   * carried the person's row. `avatarUrl` stays supported for data URLs the
   * host inlined directly, and is what a widget written before the route
   * existed keeps using.
   *
   * The path is relative, so it resolves against the frame's document URL;
   * `base-uri 'none'` in the canvas CSP means a package cannot repoint it.
   */
  function avatarImageSrc(options) {
    const pubkey = String(options.pubkey || "").toLowerCase();
    if (PUBKEY_PATTERN.test(pubkey)) return `./__buzz/avatar/${pubkey}`;
    return typeof options.avatarUrl === "string" &&
      options.avatarUrl.startsWith("data:image/")
      ? options.avatarUrl
      : null;
  }

  function avatar(props) {
    const options = props || {};
    const name = String(options.name || "");
    const size = ["xs", "sm", "md"].includes(options.size)
      ? options.size
      : "md";
    const node = el("span", "buzz-avatar");
    node.dataset.buzzComponent = "avatar";
    node.dataset.size = size;
    node.dataset.shape = options.agent ? "squircle" : "circle";
    node.setAttribute("role", "img");
    node.setAttribute("aria-label", name || "Unknown person");
    node.dataset.tone = String(toneFor(name));
    node.append(el("span", "buzz-avatar-fallback", initialsFor(name) || "?"));
    const src = avatarImageSrc(options);
    if (!src) return node;
    // The picture is stacked over the initials (see `.buzz-avatar-image`) and
    // dropped if it fails. The route 404s for anyone the host has no avatar
    // for, which is ordinary rather than an error, so that case has to leave
    // the initials showing instead of a broken-image glyph.
    const image = el("img", "buzz-avatar-image");
    image.alt = "";
    image.decoding = "async";
    image.addEventListener(
      "error",
      () => {
        image.remove();
      },
      { once: true },
    );
    image.src = src;
    node.append(image);
    return node;
  }

  function openable(row, target, onOpen) {
    const handler =
      typeof onOpen === "function"
        ? onOpen
        : target
          ? () => {
              app.open(target).catch(() => {});
            }
          : null;
    if (!handler) return row;
    const button = el("button", row.className);
    button.type = "button";
    for (const [key, value] of Object.entries(row.dataset)) {
      button.dataset[key] = value;
    }
    button.append(...row.childNodes);
    button.addEventListener("click", handler);
    return button;
  }

  function reviewRow(props) {
    const options = props || {};
    const review = options.review || {};
    const row = el("div", "buzz-review-row");
    row.dataset.buzzComponent = "review-row";
    const summary = el("span", "buzz-review-summary");
    summary.append(
      el("span", "buzz-review-id", review.displayId || ""),
      el("strong", "buzz-review-title", review.title || "Untitled review"),
    );
    if (review.branch) {
      summary.append(el("code", "buzz-review-branch", review.branch));
    }
    const status = String(review.status || "Open");
    const pill = el("span", "buzz-status-pill", status);
    pill.dataset.status = status.toLowerCase().replaceAll(" ", "-");
    const trailing = el("span", "buzz-review-status");
    if (review.authorName) {
      trailing.append(
        avatar({
          agent: Boolean(review.authorIsAgent),
          avatarUrl: review.authorAvatarUrl || null,
          pubkey: review.authorPubkey || null,
          name: review.authorName,
          size: "xs",
        }),
      );
    }
    trailing.append(pill);
    row.append(summary, trailing);
    return openable(
      row,
      review.id ? { id: review.id, type: "review" } : null,
      options.onOpen,
    );
  }

  function channelRow(props) {
    const options = props || {};
    const channel = options.channel || {};
    const row = el("div", "buzz-channel-row");
    row.dataset.buzzComponent = "channel-row";
    const details = el("span", "buzz-channel-details");
    details.append(
      el("strong", "buzz-channel-name", `# ${channel.name || "channel"}`),
    );
    const meta = channel.topic || channel.description || "";
    if (meta) details.append(el("span", "buzz-channel-meta", meta));
    row.append(details);
    const people = Array.isArray(channel.people)
      ? channel.people.slice(0, 5)
      : [];
    if (people.length > 0) {
      const cluster = el("span", "buzz-channel-people");
      for (const person of people) {
        cluster.append(
          avatar({
            agent: Boolean(person.isAgent),
            avatarUrl: person.avatarDataUrl || null,
            name: person.displayName || person.pubkey || "",
            pubkey: person.pubkey || null,
            size: "xs",
          }),
        );
      }
      row.append(cluster);
    }
    return openable(
      row,
      channel.id ? { id: channel.id, type: "channel" } : null,
      options.onOpen,
    );
  }

  Object.assign(runtime.sdk, {
    app,
    capabilities: () => session.capabilities.slice(),
    data,
    layout,
    ui: Object.freeze({ avatar, channelRow, reviewRow }),
    version: 1,
  });
  Object.freeze(runtime.sdk);
})();
