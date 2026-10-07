/*
 * Buzz review annotator.
 *
 * The only script that runs inside the sandboxed Review Canvas frame. The
 * frame document's CSP permits exactly this one nonce-bearing script; artifact
 * scripts never execute. It receives a single MessagePort from trusted parent
 * chrome (the one-time capability), makes each declared review block focusable,
 * and reports pointer, Enter, and Space selection back over that port. It never
 * reads or sends artifact text, and it exposes nothing on `window`.
 */
(() => {
  const MARKER = "data-synaxis-review-id";
  const INIT_TYPE = "synaxis-review:init";
  const MAX_BLOCKS = 200;

  function isSelectKey(event) {
    return (
      event.key === "Enter" || event.key === " " || event.key === "Spacebar"
    );
  }

  function hasModifier(event) {
    return event.shiftKey || event.ctrlKey || event.altKey || event.metaKey;
  }

  function createAnnotator(win, port, blocks) {
    const doc = win.document;
    const titles = new Map(blocks.map((block) => [block.id, block.title]));
    const elementsById = new Map();
    const idByElement = new Map();

    for (const element of doc.querySelectorAll(`[${MARKER}]`)) {
      const id = element.getAttribute(MARKER);
      if (!titles.has(id) || elementsById.has(id)) continue;
      elementsById.set(id, element);
      idByElement.set(element, id);
      element.setAttribute("tabindex", "0");
      if (
        !element.hasAttribute("role") &&
        (element.localName === "div" || element.localName === "span")
      ) {
        element.setAttribute("role", "group");
      }
      element.setAttribute(
        "aria-label",
        `${titles.get(id)} (review block, press Enter to comment)`,
      );
    }

    const select = (id) => port.postMessage({ type: "select", id });

    // Capture phase, so selection wins over anything inside the artifact.
    doc.addEventListener(
      "click",
      (event) => {
        // A link or area must never navigate the frame.
        for (let node = event.target; node; node = node.parentElement) {
          if (node.localName === "a" || node.localName === "area") {
            event.preventDefault();
          }
        }
        for (let node = event.target; node; node = node.parentElement) {
          const id = idByElement.get(node);
          if (id !== undefined) {
            event.preventDefault();
            select(id);
            return;
          }
        }
      },
      true,
    );

    doc.addEventListener(
      "keydown",
      (event) => {
        // Only the focused block itself activates; keys typed into a control
        // inside a block belong to that control.
        const id = idByElement.get(event.target);
        if (id === undefined || event.repeat || hasModifier(event)) return;
        if (!isSelectKey(event)) return;
        event.preventDefault();
        select(id);
      },
      true,
    );

    port.onmessage = (event) => {
      const message = event.data;
      if (!message || typeof message !== "object") return;
      if (message.type === "selection") {
        for (const [id, element] of elementsById) {
          if (id === message.id) {
            element.setAttribute("data-synaxis-selected", "true");
            element.setAttribute("aria-current", "true");
          } else {
            element.removeAttribute("data-synaxis-selected");
            element.removeAttribute("aria-current");
          }
        }
      } else if (message.type === "focus") {
        const element = elementsById.get(message.id);
        if (element) element.focus();
      }
    };

    port.postMessage({ type: "ready", blockIds: [...elementsById.keys()] });
  }

  function isBlock(value) {
    return (
      value &&
      typeof value.id === "string" &&
      typeof value.title === "string" &&
      value.id.length <= 128 &&
      value.title.length <= 400
    );
  }

  const onMessage = (event) => {
    // Only the embedding chrome may hand over the port, and only once.
    if (event.source !== window.parent) return;
    const data = event.data;
    const port = event.ports?.[0];
    if (
      !port ||
      !data ||
      data.type !== INIT_TYPE ||
      !Array.isArray(data.blocks) ||
      data.blocks.length > MAX_BLOCKS ||
      !data.blocks.every(isBlock)
    ) {
      return;
    }
    window.removeEventListener("message", onMessage);
    createAnnotator(window, port, data.blocks);
  };
  window.addEventListener("message", onMessage);
})();
