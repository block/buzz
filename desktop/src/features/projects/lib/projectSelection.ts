/** Clustered overview selection that drives the context pod. */

export type ProjectSelectionKind =
  | "channel"
  | "commit"
  | "project"
  | "repository"
  | "review"
  | "task";

export type ProjectSelectionItem = {
  channelId?: string | null;
  id: string;
  kind: ProjectSelectionKind;
  people?: string[];
  shareLink?: string | null;
  title: string;
};

export type ProjectSelectionState = {
  anchorId: string | null;
  items: ProjectSelectionItem[];
};

export const EMPTY_PROJECT_SELECTION: ProjectSelectionState = {
  anchorId: null,
  items: [],
};

export type ProjectSelectionActionId =
  | "chat-agent"
  | "copy"
  | "create-review"
  | "discuss";

export type ProjectSelectionAction = {
  id: ProjectSelectionActionId;
  label: string;
  testId: string;
};

export type ProjectSelectionPresentation = {
  actions: ProjectSelectionAction[];
  people: string[];
  title: string;
};

const KIND_NOUNS: Record<ProjectSelectionKind, [string, string]> = {
  channel: ["channel", "channels"],
  commit: ["commit", "commits"],
  project: ["project", "projects"],
  repository: ["repository", "repositories"],
  review: ["review", "reviews"],
  task: ["task", "tasks"],
};

export function projectSelectionNoun(
  kind: ProjectSelectionKind,
  count: number,
) {
  const [singular, plural] = KIND_NOUNS[kind];
  return count === 1 ? singular : plural;
}

export function projectSelectionTitle(items: ProjectSelectionItem[]) {
  if (items.length === 0) return "";
  const kind = items[0]?.kind;
  if (!kind) return "";
  return `${items.length} ${projectSelectionNoun(kind, items.length)}`;
}

export function nextProjectSelection(
  current: ProjectSelectionState,
  item: ProjectSelectionItem,
  options?: {
    rangeItems?: ProjectSelectionItem[];
    shiftKey?: boolean;
  },
): ProjectSelectionState {
  const currentKind = current.items[0]?.kind;
  if (currentKind && currentKind !== item.kind) {
    return { anchorId: item.id, items: [item] };
  }

  if (options?.shiftKey && current.anchorId && options.rangeItems) {
    const ordered = options.rangeItems.filter(
      (candidate) => candidate.kind === item.kind,
    );
    const from = ordered.findIndex(
      (candidate) => candidate.id === current.anchorId,
    );
    const to = ordered.findIndex((candidate) => candidate.id === item.id);
    if (from >= 0 && to >= 0) {
      const start = Math.min(from, to);
      const end = Math.max(from, to);
      return {
        anchorId: current.anchorId,
        items: ordered.slice(start, end + 1),
      };
    }
  }

  const exists = current.items.some((candidate) => candidate.id === item.id);
  if (exists) {
    const items = current.items.filter((candidate) => candidate.id !== item.id);
    return {
      anchorId: items.at(-1)?.id ?? null,
      items,
    };
  }

  return {
    anchorId: item.id,
    items: [...current.items, item],
  };
}

export function nextProjectGroupSelection(
  current: ProjectSelectionState,
  groupItems: ProjectSelectionItem[],
): ProjectSelectionState {
  const kind = groupItems[0]?.kind;
  if (!kind) return current;
  const items = groupItems.filter((item) => item.kind === kind);
  if (items.length === 0) return current;
  const currentItems = current.items[0]?.kind === kind ? current.items : [];
  const groupIds = new Set(items.map((item) => item.id));
  const allSelected = items.every((item) =>
    currentItems.some((candidate) => candidate.id === item.id),
  );
  const nextItems = allSelected
    ? currentItems.filter((item) => !groupIds.has(item.id))
    : [
        ...currentItems,
        ...items.filter(
          (item) => !currentItems.some((candidate) => candidate.id === item.id),
        ),
      ];
  return {
    anchorId: nextItems.at(-1)?.id ?? null,
    items: nextItems,
  };
}

export function projectSelectionPeople(items: ProjectSelectionItem[]) {
  return [
    ...new Set(
      items.flatMap((item) =>
        (item.people ?? []).filter((pubkey) => pubkey.trim().length > 0),
      ),
    ),
  ];
}

export function projectSelectionShareLinks(items: ProjectSelectionItem[]) {
  return [
    ...new Set(
      items
        .map((item) => item.shareLink?.trim() ?? "")
        .filter((link) => link.length > 0),
    ),
  ];
}

export function projectSelectionChannelId(items: ProjectSelectionItem[]) {
  return projectSelectionChannelCandidates(items)[0]?.channelId ?? null;
}

export function projectSelectionChannelCandidates(
  items: ProjectSelectionItem[],
) {
  const counts = new Map<string, number>();
  for (const item of items) {
    const channelId = item.channelId?.trim() ?? "";
    if (!channelId) continue;
    counts.set(channelId, (counts.get(channelId) ?? 0) + 1);
  }
  return [...counts.entries()]
    .map(([channelId, count]) => ({ channelId, count }))
    .sort(
      (left, right) =>
        right.count - left.count ||
        left.channelId.localeCompare(right.channelId),
    );
}

export function projectSelectionDiscussContent(items: ProjectSelectionItem[]) {
  if (items.length === 0) return "";
  const kind = items[0]?.kind;
  if (!kind) return "";
  const noun = projectSelectionNoun(kind, items.length);
  const heading =
    items.length === 1
      ? `Let's talk about this ${noun}:`
      : `Let's talk about these ${noun}:`;
  const links = projectSelectionShareLinks(items);
  const lines = links.length > 0 ? links : items.map((item) => item.title);
  return `${heading}\n\n${lines.join("\n")}`;
}

export function mergeSelectionDiscussDraft(
  existing: string | undefined,
  next: string,
) {
  const current = existing?.trim() ?? "";
  if (!current) return next;
  if (current.includes(next.trim())) return existing ?? next;
  return `${current}\n\n${next}`;
}

export function projectSelectionPresentation(
  items: ProjectSelectionItem[],
): ProjectSelectionPresentation | null {
  if (items.length === 0) return null;
  const kind = items[0]?.kind;
  if (!kind) return null;
  const actions: ProjectSelectionAction[] = [
    {
      id: "chat-agent",
      label: "Chat with an agent",
      testId: "projects-selection-chat-agent",
    },
  ];
  actions.push({
    id: "discuss",
    label: "Discuss in a channel",
    testId: "projects-selection-discuss",
  });
  if (kind === "commit") {
    actions.push({
      id: "create-review",
      label: items.length === 1 ? "Create review" : "Create reviews",
      testId: "projects-selection-create-review",
    });
  }
  if (projectSelectionShareLinks(items).length > 0) {
    actions.push({
      id: "copy",
      label: items.length === 1 ? "Copy link" : "Copy links",
      testId: "projects-selection-copy",
    });
  }
  return {
    actions,
    people: projectSelectionPeople(items),
    title: projectSelectionTitle(items),
  };
}

export function selectionItemFromProject(input: {
  channelId?: string | null;
  id: string;
  owner?: string | null;
  shareLink?: string | null;
  title: string;
}): ProjectSelectionItem {
  return {
    channelId: input.channelId ?? null,
    id: `project:${input.id}`,
    kind: "project",
    people: input.owner ? [input.owner] : [],
    shareLink: input.shareLink ?? null,
    title: input.title,
  };
}

export function selectionItemFromRepository(input: {
  channelId?: string | null;
  id: string;
  owner?: string | null;
  shareLink?: string | null;
  title: string;
}): ProjectSelectionItem {
  return {
    channelId: input.channelId ?? null,
    id: `repository:${input.id}`,
    kind: "repository",
    people: input.owner ? [input.owner] : [],
    shareLink: input.shareLink ?? null,
    title: input.title,
  };
}

export function selectionItemFromChannel(input: {
  channelId: string;
  people?: string[];
  title: string;
}): ProjectSelectionItem {
  return {
    channelId: input.channelId,
    id: `channel:${input.channelId}`,
    kind: "channel",
    people: input.people ?? [],
    shareLink: null,
    title: input.title,
  };
}

export function selectionItemFromTask(input: {
  author?: string | null;
  channelId?: string | null;
  id: string;
  shareLink?: string | null;
  title: string;
}): ProjectSelectionItem {
  return {
    channelId: input.channelId ?? null,
    id: `task:${input.id}`,
    kind: "task",
    people: input.author ? [input.author] : [],
    shareLink: input.shareLink ?? null,
    title: input.title,
  };
}

export function selectionItemFromReview(input: {
  author?: string | null;
  channelId?: string | null;
  id: string;
  shareLink?: string | null;
  title: string;
}): ProjectSelectionItem {
  return {
    channelId: input.channelId ?? null,
    id: `review:${input.id}`,
    kind: "review",
    people: input.author ? [input.author] : [],
    shareLink: input.shareLink ?? null,
    title: input.title,
  };
}

export function selectionItemFromCommit(input: {
  author?: string | null;
  channelId?: string | null;
  commitHash: string;
  projectId: string;
  shareLink?: string | null;
  title: string;
}): ProjectSelectionItem {
  return {
    channelId: input.channelId ?? null,
    id: `commit:${input.projectId}:${input.commitHash}`,
    kind: "commit",
    people: input.author ? [input.author] : [],
    shareLink: input.shareLink ?? null,
    title: input.title,
  };
}
