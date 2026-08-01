import type { Channel } from "@/shared/api/types";

/**
 * Sub-channels are a pure naming convention — no relay/schema support: a
 * channel named `parent--sub` is a sub-channel of the channel named
 * `parent`. One nesting level only. The separator can never appear in
 * generated names (sanitizeChannelName collapses hyphen runs), so ordinary
 * channels cannot accidentally become subs.
 */
export const SUB_CHANNEL_SEPARATOR = "--";

export type SubChannelRef = {
  parentName: string;
  subSlug: string;
};

/** Parse `parent--sub`; null when the name is not sub-channel-shaped. */
export function parseSubChannelName(name: string): SubChannelRef | null {
  const index = name.indexOf(SUB_CHANNEL_SEPARATOR);
  if (index <= 0) return null;
  const subSlug = name.slice(index + SUB_CHANNEL_SEPARATOR.length);
  if (subSlug.length === 0) return null;
  return { parentName: name.slice(0, index), subSlug };
}

export function subChannelName(parentName: string, subSlug: string): string {
  return `${parentName}${SUB_CHANNEL_SEPARATOR}${subSlug}`;
}

export type SubChannelIndex = {
  /** Channels that render in the left list: everything except paired subs. */
  mains: Channel[];
  /** Sub-channels per parent id, newest activity first. */
  subsByParentId: ReadonlyMap<string, Channel[]>;
  parentIdByChildId: ReadonlyMap<string, string>;
};

function lastMessageTime(channel: Channel): number {
  if (!channel.lastMessageAt) return 0;
  const timestamp = Date.parse(channel.lastMessageAt);
  return Number.isFinite(timestamp) ? timestamp : 0;
}

/**
 * Pair subs with their parents in one O(n) pass — parents can have hundreds
 * of subs, so no per-channel scans. A `--` name whose parent is not in the
 * list stays a main (nothing silently disappears).
 */
export function indexSubChannels(channels: Channel[]): SubChannelIndex {
  const byName = new Map<string, Channel>();
  for (const channel of channels) {
    byName.set(channel.name, channel);
  }

  const mains: Channel[] = [];
  const subsByParentId = new Map<string, Channel[]>();
  const parentIdByChildId = new Map<string, string>();
  for (const channel of channels) {
    const parsed = parseSubChannelName(channel.name);
    const parent = parsed ? byName.get(parsed.parentName) : undefined;
    if (!parsed || !parent || parent.id === channel.id) {
      mains.push(channel);
      continue;
    }
    parentIdByChildId.set(channel.id, parent.id);
    const siblings = subsByParentId.get(parent.id);
    if (siblings) {
      siblings.push(channel);
    } else {
      subsByParentId.set(parent.id, [channel]);
    }
  }
  for (const siblings of subsByParentId.values()) {
    siblings.sort((left, right) => {
      const activityOrder = lastMessageTime(right) - lastMessageTime(left);
      return activityOrder || left.name.localeCompare(right.name);
    });
  }
  return { mains, subsByParentId, parentIdByChildId };
}

/**
 * Unread mains for the left list: a main is unread when it or any of its
 * subs is unread — subs have no row of their own to carry the dot.
 */
export function aggregateUnreadMains(
  index: SubChannelIndex,
  unreadChannelIds: ReadonlySet<string>,
): ReadonlySet<string> {
  const unread = new Set<string>();
  for (const main of index.mains) {
    if (unreadChannelIds.has(main.id)) {
      unread.add(main.id);
      continue;
    }
    const subs = index.subsByParentId.get(main.id);
    if (subs?.some((sub) => unreadChannelIds.has(sub.id))) {
      unread.add(main.id);
    }
  }
  return unread;
}

/**
 * Working mains for the left list: a main waves when work is active in the
 * main itself or in any sub hidden behind its tab strip.
 */
export function aggregateWorkingMains(
  index: SubChannelIndex,
  workingChannelIds: ReadonlySet<string>,
): ReadonlySet<string> {
  const working = new Set<string>();
  for (const main of index.mains) {
    if (workingChannelIds.has(main.id)) {
      working.add(main.id);
      continue;
    }
    const subs = index.subsByParentId.get(main.id);
    if (subs?.some((sub) => workingChannelIds.has(sub.id))) {
      working.add(main.id);
    }
  }
  return working;
}

/**
 * Last-activity overrides for list ordering: a main's activity is the max
 * of its own and all its subs', so working sub-channels float their parent.
 */
export function aggregateLastActivity(
  index: SubChannelIndex,
): ReadonlyMap<string, string> {
  const overrides = new Map<string, string>();
  for (const [parentId, subs] of index.subsByParentId) {
    let latest = "";
    for (const sub of subs) {
      if ((sub.lastMessageAt ?? "") > latest) {
        latest = sub.lastMessageAt ?? "";
      }
    }
    if (latest.length > 0) {
      overrides.set(parentId, latest);
    }
  }
  return overrides;
}

/** First line of the prompt, bounded — used in announcements and canvases. */
export function subChannelTaskLine(task: string): string {
  const firstLine = task.split("\n", 1)[0].trim();
  return firstLine.length > 140 ? `${firstLine.slice(0, 139)}…` : firstLine;
}

export function subChannelAnnouncement(subName: string): string {
  return `→ spawned #${subName}`;
}

/**
 * The sub-channel's canvas records its relationship and the report-back
 * contract. Format is shared with `buzz channels create --parent` — keep
 * the two in sync (crates/buzz-cli/src/commands/channels.rs).
 */
export function subChannelCanvasDoc(input: {
  parentName: string;
  parentId: string;
  announcementEventId: string;
  task: string;
}): string {
  const task = subChannelTaskLine(input.task);
  return [
    `# Sub-channel of #${input.parentName}`,
    "",
    `- parent: #${input.parentName} (${input.parentId})`,
    `- spawned-from: ${input.announcementEventId}`,
    `- task: ${task}`,
    "",
    `When the work here is complete, post a summary to #${input.parentName} ` +
      `as a thread reply to the spawn announcement (event ${input.announcementEventId}). ` +
      `Every member of this sub-channel must be a member of #${input.parentName}.`,
  ].join("\n");
}

const SUB_CHANNELS_HEADING = "## Sub-channels";

/**
 * Append a sub entry to the parent canvas's `## Sub-channels` section,
 * creating the section (and canvas) when absent. Single read + single write
 * even with hundreds of entries. Same format as the CLI's --parent path.
 */
export function appendSubChannelToParentCanvas(
  existing: string | null,
  subName: string,
  task: string,
): string {
  const bullet = `- #${subName} — ${subChannelTaskLine(task)}`;
  const canvas = existing ?? "";
  const lines = canvas.split("\n");
  const headingAt = lines.findIndex(
    (line) => line.trim() === SUB_CHANNELS_HEADING,
  );
  if (headingAt === -1) {
    const base = canvas.trimEnd();
    return base.length === 0
      ? `${SUB_CHANNELS_HEADING}\n${bullet}\n`
      : `${base}\n\n${SUB_CHANNELS_HEADING}\n${bullet}\n`;
  }
  let insertAt = headingAt + 1;
  for (let index = headingAt + 1; index < lines.length; index += 1) {
    const line = lines[index];
    if (line.trim().length === 0) continue;
    if (!line.startsWith("- ")) break;
    insertAt = index + 1;
  }
  lines.splice(insertAt, 0, bullet);
  return lines.join("\n");
}

export type SubChannelRename = {
  channelId: string;
  newName: string;
};

/**
 * Renaming a parent must rename every sub (the name IS the link). Pure
 * planner: single pass over the channel list; the caller applies the
 * updates with bounded concurrency.
 */
export function planSubChannelRenames(
  channels: readonly Pick<Channel, "id" | "name">[],
  oldParentName: string,
  newParentName: string,
): SubChannelRename[] {
  if (oldParentName === newParentName) return [];
  const prefix = `${oldParentName}${SUB_CHANNEL_SEPARATOR}`;
  const renames: SubChannelRename[] = [];
  for (const channel of channels) {
    if (!channel.name.startsWith(prefix)) continue;
    const subSlug = channel.name.slice(prefix.length);
    if (subSlug.length === 0) continue;
    renames.push({
      channelId: channel.id,
      newName: subChannelName(newParentName, subSlug),
    });
  }
  return renames;
}

/** Apply rename plans with bounded concurrency; failures are collected. */
export async function applySubChannelRenames(
  renames: readonly SubChannelRename[],
  rename: (channelId: string, newName: string) => Promise<void>,
  concurrency = 8,
): Promise<{ failed: SubChannelRename[] }> {
  const failed: SubChannelRename[] = [];
  let next = 0;
  const workers = Array.from(
    { length: Math.min(concurrency, renames.length) },
    async () => {
      while (next < renames.length) {
        const plan = renames[next];
        next += 1;
        try {
          await rename(plan.channelId, plan.newName);
        } catch {
          failed.push(plan);
        }
      }
    },
  );
  await Promise.all(workers);
  return { failed };
}
