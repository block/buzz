import * as React from "react";

import {
  loadLastComposerAgentKey,
  loadLastComposerModeKey,
  storeLastComposerAgentKey,
  storeLastComposerModeKey,
} from "@/features/dev-mode/lib/composerModePreference";
import type { DevComposerMode } from "@/features/dev-mode/lib/useDevComposerModes";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Stable identity for the cycled mode. Selection is keyed rather than
 * indexed so agent list refreshes cannot silently retarget the next prompt
 * at a different agent; a vanished agent falls back to the default agent.
 */
export function devComposerModeKey(mode: DevComposerMode): string {
  return mode.kind === "chat" ? "chat" : normalizePubkey(mode.target.pubkey);
}

/**
 * The composer's target selection: Tab toggles plain chat ↔ the last agent
 * the composer targeted, ⌃Tab / ⌘Tab cycles through the agents (chat
 * excluded). Both the selected mode and the last agent are persisted across
 * launches. Before any selection exists — or when the remembered agent
 * vanishes — the selection falls back to the default: the first managed
 * (local) agent, else the first agent; plain chat only when no agents exist.
 */
export function useComposerModeSelection(modes: DevComposerMode[]): {
  mode: DevComposerMode | undefined;
  /** Tab — toggle chat ↔ last agent. */
  toggleMode: () => void;
  /** ⌃Tab / ⌘Tab (+⇧ reverses) — cycle through the agents. */
  cycleAgent: (direction: 1 | -1) => void;
  /** Re-persist the mode a prompt was just sent with. */
  rememberMode: (mode: DevComposerMode) => void;
} {
  const [modeKey, setModeKey] = React.useState<string | null>(
    loadLastComposerModeKey,
  );
  // The last *agent* target (never "chat"), so Tab can toggle back to it
  // from plain chat.
  const [lastAgentKey, setLastAgentKey] = React.useState<string | null>(
    loadLastComposerAgentKey,
  );

  const defaultModeIndex = React.useMemo(() => {
    const managedIndex = modes.findIndex(
      (candidate) =>
        candidate.kind === "agent" && candidate.target.source === "managed",
    );
    if (managedIndex !== -1) return managedIndex;
    const agentIndex = modes.findIndex(
      (candidate) => candidate.kind === "agent",
    );
    return agentIndex === -1 ? 0 : agentIndex;
  }, [modes]);

  const foundModeIndex =
    modeKey === null
      ? -1
      : modes.findIndex(
          (candidate) => devComposerModeKey(candidate) === modeKey,
        );
  const modeIndex = foundModeIndex === -1 ? defaultModeIndex : foundModeIndex;
  const mode = modes[modeIndex];

  React.useEffect(() => {
    if (mode?.kind !== "agent") return;
    const key = devComposerModeKey(mode);
    setLastAgentKey((current) => (current === key ? current : key));
    storeLastComposerAgentKey(key);
  }, [mode]);

  const agentModes = React.useMemo(
    () => modes.filter((candidate) => candidate.kind === "agent"),
    [modes],
  );

  // The agent Tab returns to from chat: the remembered last agent when it
  // still exists, else the default (first managed, else first) agent.
  const resumeAgentMode = React.useMemo(() => {
    const remembered =
      lastAgentKey === null
        ? undefined
        : agentModes.find(
            (candidate) => devComposerModeKey(candidate) === lastAgentKey,
          );
    if (remembered) return remembered;
    const fallback = modes[defaultModeIndex];
    return fallback?.kind === "agent" ? fallback : (agentModes[0] ?? null);
  }, [agentModes, defaultModeIndex, lastAgentKey, modes]);

  const selectMode = React.useCallback((nextMode: DevComposerMode) => {
    const nextKey = devComposerModeKey(nextMode);
    setModeKey(nextKey);
    storeLastComposerModeKey(nextKey);
  }, []);

  const toggleMode = React.useCallback(() => {
    if (mode?.kind === "agent") {
      setModeKey("chat");
      storeLastComposerModeKey("chat");
      return;
    }
    if (resumeAgentMode) selectMode(resumeAgentMode);
  }, [mode, resumeAgentMode, selectMode]);

  const cycleAgent = React.useCallback(
    (direction: 1 | -1) => {
      if (agentModes.length === 0) return;
      const currentKey = mode ? devComposerModeKey(mode) : null;
      const currentIndex = agentModes.findIndex(
        (candidate) => devComposerModeKey(candidate) === currentKey,
      );
      if (currentIndex === -1) {
        // From chat, resume at the last agent instead of restarting the loop.
        if (resumeAgentMode) selectMode(resumeAgentMode);
        return;
      }
      const nextIndex =
        (currentIndex + direction + agentModes.length) % agentModes.length;
      selectMode(agentModes[nextIndex]);
    },
    [agentModes, mode, resumeAgentMode, selectMode],
  );

  const rememberMode = React.useCallback((sentMode: DevComposerMode) => {
    storeLastComposerModeKey(devComposerModeKey(sentMode));
  }, []);

  return { mode, toggleMode, cycleAgent, rememberMode };
}
