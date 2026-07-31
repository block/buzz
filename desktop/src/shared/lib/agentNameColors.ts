import type { CSSProperties } from "react";

export const AGENT_NAME_COLOR_IDS = [
  "red",
  "orange",
  "amber",
  "yellow",
  "lime",
  "green",
  "emerald",
  "teal",
  "cyan",
  "sky",
  "blue",
  "indigo",
  "violet",
  "purple",
  "fuchsia",
  "pink",
] as const;

export type AgentNameColorId = (typeof AGENT_NAME_COLOR_IDS)[number];

function isAgentNameColorId(value: string): value is AgentNameColorId {
  return (AGENT_NAME_COLOR_IDS as readonly string[]).includes(value);
}

/**
 * Style to apply to an agent's name text. Unset/unknown colors return `{}`,
 * preserving whatever theme-derived color the element already has.
 */
export function getAgentNameColorStyle(
  nameColor?: string | null,
): CSSProperties {
  if (!nameColor || !isAgentNameColorId(nameColor)) {
    return {};
  }
  return { color: `var(--agent-color-${nameColor})` };
}

/**
 * The CSS `var(...)` reference for a palette color id, or `undefined` if the
 * id is missing/unknown. Validates against the same 16-id palette as
 * `getAgentNameColorStyle` so callers building raw style strings (e.g. the
 * mention-highlight ProseMirror decorations) can't interpolate an
 * unvalidated id straight into CSS.
 */
export function colorIdToCssVarValue(
  colorId?: string | null,
): string | undefined {
  if (!colorId || !isAgentNameColorId(colorId)) {
    return undefined;
  }
  return `var(--agent-color-${colorId})`;
}
