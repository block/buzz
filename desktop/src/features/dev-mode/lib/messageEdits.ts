import { applyEditTagOverlay } from "@/features/messages/lib/applyEditTagOverlay.mjs";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_STREAM_MESSAGE_EDIT } from "@/shared/constants/kinds";

function editTargetId(tags: string[][]): string | null {
  for (const tag of tags) {
    if (tag[0] === "e" && tag[1]) return tag[1];
  }
  return null;
}

/**
 * Latest kind:40003 edit per target message: targetId → edit event. The
 * standard UI resolves edits in formatTimelineMessages; developer mode
 * renders raw relay events, so it applies the same overlay via these
 * helpers. Edit events ride along in both the channel window flatten and
 * the thread-subtree fetch (structural aux), so callers can pass either
 * query's data unfiltered.
 */
export function collectMessageEdits(
  events: readonly RelayEvent[] | undefined,
): Map<string, RelayEvent> {
  const edits = new Map<string, RelayEvent>();
  for (const event of events ?? []) {
    if (event.kind !== KIND_STREAM_MESSAGE_EDIT) continue;
    const targetId = editTargetId(event.tags);
    if (!targetId) continue;
    const existing = edits.get(targetId);
    if (!existing || event.created_at > existing.created_at) {
      edits.set(targetId, event);
    }
  }
  return edits;
}

/**
 * Overlay each edited event with its latest edit: the edit's content
 * replaces the original's, and imeta tags come exclusively from the edit
 * (applyEditTagOverlay — same merge the standard timeline uses). Events
 * without edits pass through unchanged, as does the whole array when no
 * edits exist.
 */
export function applyMessageEdits(
  events: readonly RelayEvent[] | undefined,
): RelayEvent[] {
  const source = events ?? [];
  const edits = collectMessageEdits(source);
  if (edits.size === 0) return [...source];
  return source.map((event) => {
    const edit = edits.get(event.id);
    if (!edit) return event;
    return {
      ...event,
      content: edit.content,
      tags: applyEditTagOverlay(event.tags, edit.tags),
    };
  });
}
