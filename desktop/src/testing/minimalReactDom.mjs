export function installMinimalReactDom() {
  class MinimalEventTarget {
    constructor() {
      this._listeners = {};
    }
    addEventListener(type, fn) {
      this._listeners[type] ??= [];
      this._listeners[type].push(fn);
    }
    removeEventListener(type, fn) {
      this._listeners[type] = (this._listeners[type] ?? []).filter(
        (listener) => listener !== fn,
      );
    }
    dispatchEvent(event) {
      for (const fn of this._listeners[event.type] ?? []) fn(event);
      return true;
    }
  }

  class MinimalNode extends MinimalEventTarget {
    constructor(tagName) {
      super();
      this.tagName = tagName;
      this.children = [];
      this.childNodes = [];
      this.style = {};
      this.nodeType = tagName === "#text" ? 3 : 1;
      this.parentNode = null;
    }
    get ownerDocument() {
      return globalThis.document;
    }
    get firstChild() {
      return this.children[0] ?? null;
    }
    appendChild(child) {
      this.children.push(child);
      this.childNodes.push(child);
      child.parentNode = this;
      return child;
    }
    removeChild(child) {
      this.children = this.children.filter((item) => item !== child);
      this.childNodes = this.childNodes.filter((item) => item !== child);
      child.parentNode = null;
      return child;
    }
    insertBefore(newNode, refNode) {
      if (!refNode) return this.appendChild(newNode);
      const index = this.children.indexOf(refNode);
      if (index < 0) return this.appendChild(newNode);
      this.children.splice(index, 0, newNode);
      this.childNodes.splice(index, 0, newNode);
      newNode.parentNode = this;
      return newNode;
    }
    contains(node) {
      return node != null && (node === this || this.children.includes(node));
    }
  }

  class MinimalDocument extends MinimalEventTarget {
    constructor() {
      super();
      this.nodeType = 9;
      this._body = new MinimalNode("body");
    }
    createElement(tagName) {
      return new MinimalNode(tagName);
    }
    createTextNode(value) {
      const node = new MinimalNode("#text");
      node.nodeValue = value;
      return node;
    }
    createComment(value) {
      const node = new MinimalNode("#comment");
      node.nodeValue = value;
      node.nodeType = 8;
      return node;
    }
    get body() {
      return this._body;
    }
    get activeElement() {
      return null;
    }
  }

  globalThis.document = new MinimalDocument();
  globalThis.HTMLElement = MinimalNode;
  globalThis.HTMLIFrameElement = MinimalNode;
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  process.env.IS_REACT_ACT_ENVIRONMENT = "true";

  Object.defineProperty(globalThis, "window", {
    value: globalThis,
    configurable: true,
  });
  Object.defineProperty(globalThis, "navigator", {
    value: { userAgent: "node" },
    configurable: true,
  });

  globalThis.MutationObserver = class {
    observe() {}
    disconnect() {}
    takeRecords() {
      return [];
    }
  };
  globalThis.requestAnimationFrame = (fn) => setTimeout(fn, 0);
}
