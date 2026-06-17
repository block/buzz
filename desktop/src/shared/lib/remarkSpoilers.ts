/**
 * Remark plugin that turns chat-style spoiler spans (`||secret||`) into a
 * custom HAST element rendered by `markdown.tsx`.
 */

type Node = {
  // biome-ignore lint/suspicious/noExplicitAny: building mdast-compatible nodes
  [key: string]: any;
};

type Part =
  | { type: "delimiter" }
  | {
      type: "node";
      node: Node;
    };

export default function remarkSpoilers() {
  return (
    // biome-ignore lint/suspicious/noExplicitAny: remark tree types are not available
    tree: any,
  ) => {
    transformNode(tree);
  };
}

function transformNode(node: Node) {
  if (
    !node?.children ||
    !Array.isArray(node.children) ||
    shouldSkipNode(node)
  ) {
    return;
  }

  for (const child of node.children) {
    transformNode(child);
  }

  node.children = groupBlockSpoilers(groupSpoilers(node.children));
}

function groupSpoilers(children: Node[]): Node[] {
  const output: Node[] = [];
  let spoilerBuffer: Node[] | null = null;

  for (const part of splitDelimiterParts(children)) {
    if (part.type === "delimiter") {
      if (spoilerBuffer) {
        output.push(buildSpoilerNode(spoilerBuffer));
        spoilerBuffer = null;
      } else {
        spoilerBuffer = [];
      }
      continue;
    }

    if (spoilerBuffer) {
      spoilerBuffer.push(part.node);
    } else {
      output.push(part.node);
    }
  }

  if (spoilerBuffer) {
    output.push({ type: "text", value: "||" }, ...spoilerBuffer);
  }

  return output;
}

function splitDelimiterParts(children: Node[]): Part[] {
  const parts: Part[] = [];

  for (const child of children) {
    if (child.type !== "text") {
      parts.push({ type: "node", node: child });
      continue;
    }

    const text = String(child.value ?? "");
    let cursor = 0;

    while (cursor < text.length) {
      const delimiterIndex = text.indexOf("||", cursor);
      if (delimiterIndex === -1) {
        const value = text.slice(cursor);
        if (value) parts.push({ type: "node", node: { ...child, value } });
        break;
      }

      const before = text.slice(cursor, delimiterIndex);
      if (before)
        parts.push({ type: "node", node: { ...child, value: before } });
      parts.push({ type: "delimiter" });
      cursor = delimiterIndex + 2;
    }
  }

  return parts;
}

function buildSpoilerNode(
  children: Node[],
  options: { block?: boolean } = {},
): Node {
  return {
    type: "spoiler",
    children,
    data: {
      hName: "spoiler",
      ...(options.block ? { hProperties: { "data-block-spoiler": "" } } : {}),
    },
  };
}

function groupBlockSpoilers(children: Node[]): Node[] {
  const output: Node[] = [];
  let spoilerBuffer: Node[] | null = null;
  let openingDelimiter: Node | null = null;

  for (const child of children) {
    if (isBlockDelimiter(child)) {
      if (spoilerBuffer) {
        output.push(buildSpoilerNode(spoilerBuffer, { block: true }));
        spoilerBuffer = null;
        openingDelimiter = null;
      } else {
        spoilerBuffer = [];
        openingDelimiter = child;
      }
      continue;
    }

    if (spoilerBuffer) {
      spoilerBuffer.push(child);
    } else {
      output.push(child);
    }
  }

  if (spoilerBuffer) {
    if (openingDelimiter) output.push(openingDelimiter);
    output.push(...spoilerBuffer);
  }

  return output;
}

function isBlockDelimiter(node: Node): boolean {
  return (
    node.type === "paragraph" &&
    Array.isArray(node.children) &&
    node.children.length === 1 &&
    node.children[0]?.type === "text" &&
    String(node.children[0].value ?? "").trim() === "||"
  );
}

function shouldSkipNode(node: Node): boolean {
  return node.type === "code" || node.type === "inlineCode";
}
