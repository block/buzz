/**
 * Factory for remark plugins that detect prefix-based patterns (e.g. @mention,
 * #channel) in text nodes and replace them with custom HAST elements.
 *
 * Both `remarkMentions` and `remarkChannelLinks` share identical tree-walking
 * and text-splitting logic — this factory captures that once.
 */

type Node = {
  // biome-ignore lint/suspicious/noExplicitAny: building mdast-compatible nodes
  [key: string]: any;
};

type NodeBuilderResult = Node | { node: Node; trailing?: string };

type NodeBuilder = (matchText: string) => NodeBuilderResult;

/**
 * `leadGroup` names a capture group holding a leading boundary character that
 * the pattern had to consume to assert one (WebKit before Safari 16.4 fails to
 * *parse* lookbehind, blanking the whole app, so patterns capture instead).
 * Its text is emitted back as plain text and is not part of the built node.
 */
type PrefixPluginOptions = { leadGroup?: number };

/**
 * Create a remark plugin that walks the tree, finds regex matches in text
 * nodes, and replaces each match with a node produced by `buildNode`.
 */
export function createRemarkPrefixPlugin(
  pattern: RegExp,
  buildNode: NodeBuilder,
  options?: PrefixPluginOptions,
) {
  const leadGroup = options?.leadGroup;
  return (
    // biome-ignore lint/suspicious/noExplicitAny: remark tree types are not available
    tree: any,
  ) => {
    walkChildren(tree, pattern, buildNode, leadGroup);
  };
}

function walkChildren(
  node: Node,
  pattern: RegExp,
  buildNode: NodeBuilder,
  leadGroup?: number,
) {
  if (
    !node?.children ||
    !Array.isArray(node.children) ||
    shouldSkipNode(node)
  ) {
    return;
  }

  for (let i = node.children.length - 1; i >= 0; i--) {
    const child = node.children[i];

    if (child.type === "text") {
      const parts = splitByPattern(child.value, pattern, buildNode, leadGroup);
      if (
        parts.length > 1 ||
        (parts.length === 1 && parts[0].type !== "text")
      ) {
        node.children.splice(i, 1, ...parts);
      }
    } else {
      walkChildren(child, pattern, buildNode, leadGroup);
    }
  }
}

function shouldSkipNode(node: Node): boolean {
  return (
    node.type === "link" || node.type === "code" || node.type === "inlineCode"
  );
}

function splitByPattern(
  text: string,
  pattern: RegExp,
  buildNode: NodeBuilder,
  leadGroup?: number,
) {
  // Reset lastIndex — the pattern is reused across text nodes with the `g` flag
  pattern.lastIndex = 0;
  // biome-ignore lint/suspicious/noExplicitAny: building mdast-compatible nodes
  const parts: any[] = [];
  let lastIndex = 0;
  let match: RegExpExecArray | null = null;

  while (true) {
    match = pattern.exec(text);
    if (!match) {
      break;
    }

    const lead = leadGroup === undefined ? "" : (match[leadGroup] ?? "");
    const matchStart = match.index + lead.length;
    if (matchStart > lastIndex) {
      parts.push({ type: "text", value: text.slice(lastIndex, matchStart) });
    }

    const result = normalizeBuildNodeResult(
      buildNode(match[0].slice(lead.length)),
    );
    parts.push(result.node);
    if (result.trailing) {
      parts.push({ type: "text", value: result.trailing });
    }

    lastIndex = match.index + match[0].length;
  }

  if (parts.length === 0) {
    return [{ type: "text", value: text }];
  }

  if (lastIndex < text.length) {
    parts.push({ type: "text", value: text.slice(lastIndex) });
  }

  return parts;
}

function normalizeBuildNodeResult(result: NodeBuilderResult): {
  node: Node;
  trailing?: string;
} {
  if (isBuildNodeWithTrailing(result)) {
    return result;
  }

  return { node: result };
}

function isBuildNodeWithTrailing(
  result: NodeBuilderResult,
): result is { node: Node; trailing?: string } {
  return "node" in result;
}
