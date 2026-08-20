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
 * Create a remark plugin that walks the tree, finds regex matches in text
 * nodes, and replaces each match with a node produced by `buildNode`.
 */
export function createRemarkPrefixPlugin(
  pattern: RegExp,
  buildNode: NodeBuilder,
) {
  return (
    // biome-ignore lint/suspicious/noExplicitAny: remark tree types are not available
    tree: any,
  ) => {
    walkChildren(tree, pattern, buildNode);
  };
}

// biome-ignore lint/suspicious/noExplicitAny: remark tree types are not available
function walkChildren(node: any, pattern: RegExp, buildNode: NodeBuilder) {
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
      const parts = splitByPattern(child.value, pattern, buildNode);
      if (
        parts.length > 1 ||
        (parts.length === 1 && parts[0].type !== "text")
      ) {
        node.children.splice(i, 1, ...parts);
      }
    } else {
      walkChildren(child, pattern, buildNode);
    }
  }
}

// biome-ignore lint/suspicious/noExplicitAny: remark tree types are not available
function shouldSkipNode(node: any): boolean {
  return (
    node.type === "link" || node.type === "code" || node.type === "inlineCode"
  );
}

function splitByPattern(text: string, pattern: RegExp, buildNode: NodeBuilder) {
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

    if (match.index > lastIndex) {
      parts.push({ type: "text", value: text.slice(lastIndex, match.index) });
    }

    const result = normalizeBuildNodeResult(buildNode(match[0]));
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
