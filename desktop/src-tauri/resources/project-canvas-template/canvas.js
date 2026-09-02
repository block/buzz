(() => {
  const PROTOCOL_VERSION = 1;
  const INTERACTIVE_SELECTOR =
    "a,button,input,label,select,textarea,video[controls],[role='button'],[data-no-drag]";
  const DEFAULT_PAN = { x: 24, y: 24 };
  const GRID = 24;
  const MIN_WIDGET_WIDTH = 192;
  const MIN_WIDGET_HEIGHT = 144;
  const runtime = window.buzzCanvas;
  const root = document.getElementById("canvas-root");
  const widgetModules = Object.values(window.buzzCanvasWidgets || {});
  const widgetRenderers = Object.assign(
    {},
    ...widgetModules.map((module) => module.renderers || {}),
  );
  const companionRenderers = Object.assign(
    {},
    ...widgetModules.map((module) => module.companions || {}),
  );

  if (!root) throw new Error("Canvas shell is missing #canvas-root");
  if (
    !runtime ||
    runtime.protocolVersion !== PROTOCOL_VERSION ||
    !runtime.port
  ) {
    throw new Error("Canvas shell did not provide a compatible MessagePort");
  }

  const state = {
    activeWidget: null,
    canvasId: null,
    dashboard: null,
    data: null,
    loadId: null,
    mode: "preview",
    nonce: null,
    positions: new Map(),
    project: null,
    sizes: new Map(),
    snapshots: null,
    translation: { ...DEFAULT_PAN },
  };

  const port = runtime.port;
  port.addEventListener("message", onHostMessage);
  port.start();

  function onHostMessage(event) {
    const message = event.data;
    if (!message || message.protocolVersion !== PROTOCOL_VERSION) return;
    if (message.type === "host.init") initialize(message);
    if (message.type === "host.mode" && matchesSession(message)) {
      setMode(message.mode);
    }
    if (message.type === "host.dataChanged" && matchesSession(message)) {
      state.snapshots = message.snapshots || {};
    }
    if (message.type === "host.widgetDataChanged" && matchesSession(message)) {
      applyWidgetDataUpdate(message.widgetId, message.data);
    }
  }

  function matchesSession(message) {
    return message.loadId === state.loadId && message.nonce === state.nonce;
  }

  function initialize(message) {
    if (!isInitMessage(message)) return;
    state.canvasId = message.canvasId;
    state.data = message.data;
    state.loadId = message.loadId;
    state.nonce = message.nonce;
    state.project = message.project;
    state.snapshots = message.snapshots || null;
    state.mode = normalizeMode(message.mode);
    state.dashboard = selectDashboard(message.data, message.project);
    const stored = storedLayout(message.layouts, state.dashboard.id);
    state.translation = sanitizePoint(stored?.pan) || { ...DEFAULT_PAN };
    state.positions.clear();
    state.sizes.clear();
    for (const widget of state.dashboard.widgets) {
      const override = sanitizePoint(stored?.widgets?.[widget.id]);
      state.positions.set(widget.id, override || { ...widget.position });
      const sizeOverride = sanitizeSize(stored?.sizes?.[widget.id]);
      state.sizes.set(widget.id, sizeOverride || { ...widget.size });
    }
    renderCanvas();
    port.postMessage({
      type: "canvas.rendered",
      protocolVersion: PROTOCOL_VERSION,
      loadId: state.loadId,
      nonce: state.nonce,
      dashboard: state.dashboard.id,
    });
  }

  function isInitMessage(message) {
    if (message.type !== "host.init" || !message.loadId || !message.nonce) {
      return false;
    }
    if (!message.project || typeof message.project.name !== "string") {
      return false;
    }
    return Boolean(message.data?.dashboards);
  }

  function normalizeMode(mode) {
    return mode === "full" ? "full" : "preview";
  }

  function storedLayout(layouts, dashboardId) {
    if (!layouts || typeof layouts !== "object") return null;
    const layout = layouts[dashboardId];
    return layout && typeof layout === "object" ? layout : null;
  }

  function sanitizePoint(point) {
    if (!point || typeof point !== "object") return null;
    if (!Number.isFinite(point.x) || !Number.isFinite(point.y)) return null;
    return { x: point.x, y: point.y };
  }

  function sanitizeSize(size) {
    if (!size || typeof size !== "object") return null;
    if (!Number.isFinite(size.width) || !Number.isFinite(size.height)) {
      return null;
    }
    if (size.width < MIN_WIDGET_WIDTH || size.height < MIN_WIDGET_HEIGHT) {
      return null;
    }
    return { width: size.width, height: size.height };
  }

  // Persist only what the user changed: widgets still sitting on their package
  // default are left out, so a later package revision can still move or
  // resize them. Position and size overrides are independent for the same
  // reason — resizing a widget must not pin where the package put it.
  function saveLayout() {
    if (!state.dashboard || !runtime.sdk?.layout) return;
    const widgets = {};
    const sizes = {};
    for (const widget of state.dashboard.widgets) {
      const position = state.positions.get(widget.id);
      const fallback = sanitizePoint(widget.position) || { x: 0, y: 0 };
      if (
        position &&
        (position.x !== fallback.x || position.y !== fallback.y)
      ) {
        widgets[widget.id] = { x: position.x, y: position.y };
      }
      const size = state.sizes.get(widget.id);
      if (
        size &&
        (size.width !== widget.size.width || size.height !== widget.size.height)
      ) {
        sizes[widget.id] = { width: size.width, height: size.height };
      }
    }
    const pan = {
      x: Math.round(state.translation.x),
      y: Math.round(state.translation.y),
    };
    runtime.sdk.layout.save({
      dashboard: state.dashboard.id,
      pan: pan.x === DEFAULT_PAN.x && pan.y === DEFAULT_PAN.y ? null : pan,
      sizes,
      widgets,
    });
  }

  function normalizeName(name) {
    return String(name || "")
      .trim()
      .replace(/^#/, "")
      .toLowerCase();
  }

  function selectDashboard(data, project) {
    const names = [project.name, project.displayName, ...(project.names || [])];
    let dashboardId = data.defaultDashboard;
    for (const name of names) {
      const match = data.selectors[normalizeName(name)];
      if (match) {
        dashboardId = match;
        break;
      }
    }
    const dashboard = data.dashboards[dashboardId] || data.dashboards.dev;
    return { ...dashboard, id: dashboardId };
  }

  function setMode(mode) {
    state.mode = normalizeMode(mode);
    const canvas = root.querySelector("[data-testid='project-widget-canvas']");
    if (canvas) canvas.dataset.canvasMode = state.mode;
  }

  function element(tag, className, attributes) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    for (const [name, value] of Object.entries(attributes || {})) {
      if (value === undefined || value === null) continue;
      if (name === "text") node.textContent = String(value);
      else if (name === "testId") node.dataset.testid = String(value);
      else if (name === "ariaLabel")
        node.setAttribute("aria-label", String(value));
      else node.setAttribute(name, String(value));
    }
    return node;
  }

  function icon(glyph, tone) {
    return element("span", `icon ${tone || ""}`, {
      "aria-hidden": "true",
      text: glyph,
    });
  }

  function resolveAsset(path) {
    try {
      return new URL(path, window.buzzCanvas.packageBaseUrl).href;
    } catch (_error) {
      return path;
    }
  }

  function renderCanvas() {
    root.replaceChildren();
    const canvas = element("section", `canvas tone-${state.dashboard.tone}`, {
      ariaLabel: "Project widget canvas",
      testId: "project-widget-canvas",
    });
    canvas.dataset.canvasMode = state.mode;
    updateTranslationData(canvas);
    canvas.addEventListener("pointerdown", startCanvasPan);

    const world = element("div", "canvas-world", {
      testId: "project-widget-canvas-world",
    });
    updateWorldTransform(world);
    for (const widget of state.dashboard.widgets) {
      world.append(renderWidgetGroup(widget));
    }
    canvas.append(world, renderResetButton());
    root.append(canvas, renderDialogLayer());
  }

  function renderWidgetGroup(widget) {
    const position = state.positions.get(widget.id);
    const size = state.sizes.get(widget.id);
    const group = element("div", "widget-group");
    group.dataset.widgetId = widget.id;
    moveWidgetGroup(group, position);

    const article = element("article", "widget", {
      ariaLabel: `${widget.title} widget`,
      testId: `project-canvas-widget-${widget.id}`,
      tabindex: "0",
    });
    article.setAttribute("aria-roledescription", "movable widget");
    article.dataset.worldX = String(position.x);
    article.dataset.worldY = String(position.y);
    article.addEventListener("pointerdown", (event) =>
      startWidgetDrag(event, widget),
    );
    article.addEventListener("keydown", (event) => nudgeWidget(event, widget));
    if (!widget.hideHeader) article.append(renderWidgetHeader(widget));
    article.append(renderWidgetContent(widget));
    group.append(article, renderResizeHandle(widget));
    applyWidgetSize(group, size);

    const companion = renderCompanion(widget);
    if (companion) group.append(companion);
    return group;
  }

  function renderResizeHandle(widget) {
    const handle = element("button", "resize-handle", {
      ariaLabel: `Resize ${widget.title} widget`,
      testId: `project-canvas-widget-${widget.id}-resize`,
      title: "Resize widget (drag or arrow keys)",
      type: "button",
    });
    handle.addEventListener("pointerdown", (event) =>
      startWidgetResize(event, widget),
    );
    handle.addEventListener("keydown", (event) => resizeWidget(event, widget));
    return handle;
  }

  function renderWidgetHeader(widget) {
    const header = element("header", "widget-header", {
      testId: `project-canvas-widget-${widget.id}-header`,
    });
    header.append(
      icon(widgetIcon(widget.type)),
      element("h2", "", { text: widget.title }),
    );
    return header;
  }

  function widgetIcon(type) {
    return (
      {
        activeChannels: "#",
        choreBoard: "✓",
        clientTime: "◷",
        meetings: "□",
        reviews: "↗",
        tasks: "☑",
      }[type] || "•"
    );
  }

  function renderWidgetContent(widget) {
    const renderer = widgetRenderers[widget.type];
    const content = element("div", "widget-content");
    if (renderer) content.append(renderWith(renderer, widget.data));
    else
      content.append(
        element("p", "empty-state", { text: "Widget unavailable" }),
      );
    return content;
  }

  function renderWith(renderer, data) {
    if (typeof renderer === "function") return renderer(data, widgetApi);
    if (renderer && typeof renderer.render === "function") {
      return renderer.render(data, widgetApi);
    }
    return element("p", "empty-state", { text: "Widget unavailable" });
  }

  function applyWidgetDataUpdate(widgetId, data) {
    const nextDashboard = selectDashboard(data, state.project);
    const currentWidget = state.dashboard.widgets.find(
      (widget) => widget.id === widgetId,
    );
    const nextWidget = nextDashboard.widgets.find(
      (widget) => widget.id === widgetId,
    );
    if (!currentWidget || !nextWidget) return;
    const group = [...root.querySelectorAll("[data-widget-id]")].find(
      (candidate) => candidate.dataset.widgetId === widgetId,
    );
    const content = group?.querySelector(".widget-content");
    if (!content) return;

    const previousData = currentWidget.data;
    state.data = data;
    currentWidget.data = nextWidget.data;
    const nextData = currentWidget.data;
    const renderer = widgetRenderers[currentWidget.type];
    if (
      renderer &&
      typeof renderer === "object" &&
      typeof renderer.update === "function"
    ) {
      const current = content.firstElementChild;
      const updated = renderer.update(
        current,
        nextData,
        previousData,
        widgetApi,
      );
      if (updated && updated !== current) content.replaceChildren(updated);
      return;
    }
    content.replaceChildren(renderWith(renderer, nextData));
  }

  function renderCompanion(widget) {
    const renderer = companionRenderers[widget.type];
    return renderer ? renderer(widget, widgetApi) : null;
  }

  function startCanvasPan(event) {
    if (event.button !== 0 || event.target !== event.currentTarget) return;
    const canvas = event.currentTarget;
    const start = { x: event.clientX, y: event.clientY };
    const origin = { ...state.translation };
    canvas.classList.add("dragging");
    trackPointer(
      event,
      (point) => {
        state.translation = {
          x: origin.x + point.x - start.x,
          y: origin.y + point.y - start.y,
        };
        updateTranslationData(canvas);
        updateWorldTransform(canvas.querySelector(".canvas-world"));
      },
      () => {
        canvas.classList.remove("dragging");
        saveLayout();
      },
    );
  }

  function startWidgetDrag(event, widget) {
    if (event.button !== 0 || event.target.closest(INTERACTIVE_SELECTOR))
      return;
    event.preventDefault();
    event.stopPropagation();
    const article = event.currentTarget;
    const group = article.parentElement;
    const start = { x: event.clientX, y: event.clientY };
    const origin = { ...state.positions.get(widget.id) };
    state.activeWidget = widget.id;
    group.classList.add("active", "dragging");
    trackPointer(
      event,
      (point) => {
        const next = {
          x: origin.x + point.x - start.x,
          y: origin.y + point.y - start.y,
        };
        state.positions.set(widget.id, next);
        moveWidgetGroup(group, next);
        article.dataset.worldX = String(Math.round(next.x));
        article.dataset.worldY = String(Math.round(next.y));
      },
      () => {
        const snapped = snapPoint(state.positions.get(widget.id));
        state.positions.set(widget.id, snapped);
        moveWidgetGroup(group, snapped);
        article.dataset.worldX = String(snapped.x);
        article.dataset.worldY = String(snapped.y);
        group.classList.remove("dragging");
        saveLayout();
      },
    );
  }

  function trackPointer(event, onMove, onEnd) {
    const pointerId = event.pointerId;
    const target = event.currentTarget;
    target.setPointerCapture(pointerId);
    const move = (nextEvent) => {
      if (nextEvent.pointerId === pointerId) onMove(nextEvent);
    };
    const end = (nextEvent) => {
      if (nextEvent.pointerId !== pointerId) return;
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      if (target.hasPointerCapture(pointerId))
        target.releasePointerCapture(pointerId);
      onEnd(nextEvent);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
  }

  function nudgeWidget(event, widget) {
    if (event.target !== event.currentTarget) return;
    const amount = event.shiftKey ? 48 : 24;
    const delta = {
      ArrowDown: { x: 0, y: amount },
      ArrowLeft: { x: -amount, y: 0 },
      ArrowRight: { x: amount, y: 0 },
      ArrowUp: { x: 0, y: -amount },
    }[event.key];
    if (!delta) return;
    event.preventDefault();
    const current = state.positions.get(widget.id);
    const next = snapPoint({ x: current.x + delta.x, y: current.y + delta.y });
    state.positions.set(widget.id, next);
    const group = event.currentTarget.parentElement;
    moveWidgetGroup(group, next);
    event.currentTarget.dataset.worldX = String(next.x);
    event.currentTarget.dataset.worldY = String(next.y);
    saveLayout();
  }

  function startWidgetResize(event, widget) {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    const group = event.currentTarget.parentElement;
    const start = { x: event.clientX, y: event.clientY };
    const origin = { ...state.sizes.get(widget.id) };
    group.classList.add("active", "resizing");
    trackPointer(
      event,
      (point) => {
        const next = clampSize({
          width: origin.width + point.x - start.x,
          height: origin.height + point.y - start.y,
        });
        state.sizes.set(widget.id, next);
        applyWidgetSize(group, next);
      },
      () => {
        const snapped = snapSize(state.sizes.get(widget.id));
        state.sizes.set(widget.id, snapped);
        applyWidgetSize(group, snapped);
        group.classList.remove("resizing");
        saveLayout();
      },
    );
  }

  function resizeWidget(event, widget) {
    const amount = event.shiftKey ? 48 : 24;
    const delta = {
      ArrowDown: { width: 0, height: amount },
      ArrowLeft: { width: -amount, height: 0 },
      ArrowRight: { width: amount, height: 0 },
      ArrowUp: { width: 0, height: -amount },
    }[event.key];
    if (!delta) return;
    event.preventDefault();
    const current = state.sizes.get(widget.id);
    const next = snapSize({
      width: current.width + delta.width,
      height: current.height + delta.height,
    });
    state.sizes.set(widget.id, next);
    applyWidgetSize(event.currentTarget.parentElement, next);
    saveLayout();
  }

  function snapPoint(point) {
    return {
      x: Math.round(point.x / GRID) * GRID,
      y: Math.round(point.y / GRID) * GRID,
    };
  }

  // The minimums are grid multiples, so snapping then clamping stays on grid.
  function clampSize(size) {
    return {
      width: Math.max(MIN_WIDGET_WIDTH, size.width),
      height: Math.max(MIN_WIDGET_HEIGHT, size.height),
    };
  }

  function snapSize(size) {
    return clampSize({
      width: Math.round(size.width / GRID) * GRID,
      height: Math.round(size.height / GRID) * GRID,
    });
  }

  function moveWidgetGroup(group, position) {
    group.style.transform = `translate3d(${position.x}px, ${position.y}px, 0)`;
  }

  function applyWidgetSize(group, size) {
    group.style.width = `${size.width}px`;
    group.style.height = `${size.height}px`;
    const article = group.querySelector(".widget");
    if (!article) return;
    article.dataset.worldWidth = String(Math.round(size.width));
    article.dataset.worldHeight = String(Math.round(size.height));
  }

  function updateWorldTransform(world) {
    world.style.transform = `translate3d(${state.translation.x}px, ${state.translation.y}px, 0)`;
  }

  function updateTranslationData(canvas) {
    canvas.dataset.panX = String(Math.round(state.translation.x));
    canvas.dataset.panY = String(Math.round(state.translation.y));
    canvas.dataset.projectDashboard = state.dashboard ? state.dashboard.id : "";
  }

  function renderResetButton() {
    const button = element("button", "reset-button", {
      ariaLabel: "Reset canvas layout",
      testId: "project-widget-canvas-reset",
      title: "Reset canvas layout",
      type: "button",
    });
    button.append(icon("⌖"));
    button.addEventListener("click", resetLayout);
    return button;
  }

  // Always reachable recovery: restores the package's pan and every widget
  // position and size, then clears the stored overrides. Widget elements are
  // moved in place rather than re-rendered so live subscriptions survive the
  // reset.
  function resetLayout() {
    state.translation = { ...DEFAULT_PAN };
    const groups = new Map(
      [...root.querySelectorAll("[data-widget-id]")].map((group) => [
        group.dataset.widgetId,
        group,
      ]),
    );
    for (const widget of state.dashboard.widgets) {
      const position = sanitizePoint(widget.position) || { x: 0, y: 0 };
      state.positions.set(widget.id, position);
      const size = { ...widget.size };
      state.sizes.set(widget.id, size);
      const group = groups.get(widget.id);
      if (!group) continue;
      moveWidgetGroup(group, position);
      applyWidgetSize(group, size);
      const article = group.querySelector(".widget");
      if (!article) continue;
      article.dataset.worldX = String(position.x);
      article.dataset.worldY = String(position.y);
    }
    const canvas = root.querySelector("[data-testid='project-widget-canvas']");
    if (canvas) {
      updateTranslationData(canvas);
      updateWorldTransform(canvas.querySelector(".canvas-world"));
    }
    saveLayout();
  }

  function renderDialogLayer() {
    return element("div", "dialog-layer", { testId: "canvas-dialog-layer" });
  }

  function showDialog(title, body, testId) {
    const layer = root.querySelector(".dialog-layer");
    const backdrop = element("div", "dialog-backdrop");
    const dialog = element("section", "dialog", { testId });
    dialog.setAttribute("role", "dialog");
    dialog.setAttribute("aria-modal", "true");
    dialog.append(element("h2", "dialog-title", { text: title }), body);
    const close = element("button", "dialog-close", {
      ariaLabel: "Close",
      text: "×",
      type: "button",
    });
    close.addEventListener("click", () => layer.replaceChildren());
    dialog.prepend(close);
    backdrop.append(dialog);
    layer.replaceChildren(backdrop);
    close.focus();
  }

  const widgetApi = Object.freeze({
    element,
    icon,
    resolveAsset,
    showDialog,
    state: () => state,
  });
})();
