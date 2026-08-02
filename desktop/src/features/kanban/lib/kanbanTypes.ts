import { KIND_KANBAN_BOARD, KIND_KANBAN_CARD } from "@/shared/constants/kinds";
import type { RelayEvent } from "@/shared/api/types";

/**
 * Kanban read-model types + tag parsers.
 *
 * Tag parsing is always **keyed by tag name**, never by fixed index — the
 * same requirement as the P1 no-WIP CLI fix. Tags arrive in arbitrary order,
 * and optional sub-fields (`wip`, `rank`, `due`, `invite`…) may be absent, so
 * a parser that assumes "the 4th element is the WIP" silently misreads boards
 * without a WIP limit. Every field here is located by name.
 */

export type KanbanColumn = {
  /** Stable column id (colid). Renaming a lane never renumbers cards. */
  id: string;
  /** Display name. */
  name: string;
  /** Advisory WIP limit; `null` = unbounded. */
  wip: number | null;
  /** Position in the board (0-based, contiguous). */
  order: number;
};

export type KanbanBoard = {
  /** `d` tag — the board id and the `:id` in the `#a` card ref. */
  id: string;
  /** Owner pubkey (from the `p` tag with role "owner"). */
  owner: string;
  /** `name` tag — the board title shown in rail / list / view. */
  name: string;
  /** Markdown description (event `content`). */
  description: string;
  /** Ordered column definitions. */
  columns: KanbanColumn[];
  /** `h` tags — channels the owner shared the board to. */
  channels: string[];
  /** `invite` tags — pubkeys directly shared on the board. */
  invites: string[];
  createdAt: number;
};

export type KanbanCard = {
  /** `d` tag — the card id. */
  id: string;
  /** Event id (useful for optimistic/audit tie-ins later). */
  eventId: string;
  /** Raw `a` board ref, `31001:<owner>:<boardId>`. */
  board: string;
  /** `31001:<owner>:<boardId>` */
  boardOwner: string;
  /** The board id portion of the `a` ref. */
  boardId: string;
  /** `column` tag — the colid the card currently sits in. */
  column: string;
  /** Base-36 order-preserving rank string; `null` = unsorted (goes last). */
  rank: string | null;
  /** Title derived from the first markdown heading in `content`. */
  title: string;
  /** Full markdown body. */
  content: string;
  /** `p` tags — assignee pubkeys. */
  assignees: string[];
  /** `l` tags (namespaced `["l", <label>, "kanban"]`). */
  labels: string[];
  /** `due` tag — `YYYY-MM-DD`, or `null`. */
  due: string | null;
  createdAt: number;
};

function firstTagValue(event: RelayEvent, name: string): string | null {
  for (const tag of event.tags) {
    if (tag.length >= 2 && tag[0] === name && typeof tag[1] === "string") {
      return tag[1];
    }
  }
  return null;
}

function allTagValues(
  event: RelayEvent,
  name: string,
  predicate?: (tag: string[]) => boolean,
): string[] {
  const values: string[] = [];
  for (const tag of event.tags) {
    if (tag[0] !== name || tag.length < 2) continue;
    if (predicate && !predicate(tag)) continue;
    if (typeof tag[1] === "string" && tag[1].trim().length > 0) {
      values.push(tag[1]);
    }
  }
  return values;
}

/** Parse a repeatable `["column", id, "name", name, ("wip", w), "order", n]` tag. */
export function parseColumnTag(tag: string[]): KanbanColumn | null {
  if (tag[0] !== "column" || tag.length < 5 || typeof tag[1] !== "string") {
    return null;
  }
  const id = tag[1];
  if (id.trim().length === 0) return null;

  // Walk the key/value pairs that follow the tag name + id. Locating by name
  // means a column without a `wip` pair still parses correctly.
  const fields: Record<string, string> = {};
  for (let i = 2; i + 1 < tag.length; i += 2) {
    const key = tag[i];
    const value = tag[i + 1];
    if (typeof key === "string" && typeof value === "string") {
      fields[key] = value;
    }
  }
  if (fields.name === undefined) return null;

  const order = Number(fields.order);
  const wip = fields.wip === undefined ? null : Number(fields.wip);
  return {
    id,
    name: fields.name,
    order: Number.isFinite(order) ? order : Number.MAX_SAFE_INTEGER,
    wip: wip === null || !Number.isFinite(wip) ? null : wip,
  };
}

function boardOwnerFromEvent(event: RelayEvent): string {
  const ownerTag = event.tags.find(
    (tag) => tag[0] === "p" && tag.length >= 3 && tag[2] === "owner",
  );
  const owner = ownerTag?.[1] ?? firstTagValue(event, "p");
  return (owner ?? event.pubkey).toLowerCase();
}

function samePubkey(left: string, right: string): boolean {
  return left.toLowerCase() === right.toLowerCase();
}

/**
 * A board is visible to the reader when they own it, are named in an
 * `invite` tag, OR are a member of any channel the board was shared to via an
 * `h` tag.
 *
 * Kanban kinds are stored globally (`channel_id = NULL`) and are in no
 * relay-gated set (see the P3 entry-gate finding in WORK_LOGS), so a bare
 * `{kinds:[31001]}` REQ returns *every* community board. This client-side
 * check — own + `invite` + channel-shared (`h`) membership — is the only
 * access boundary keeping private boards out of a reader's list, so it must
 * coincide with the P3 write gate (owner + invited + channel members).
 */
export function boardIsAccessible(
  board: KanbanBoard,
  me: string | undefined,
  memberChannelIds: readonly string[],
): boolean {
  if (!me) return false;
  if (samePubkey(board.owner, me)) return true;
  if (board.invites.some((invite) => samePubkey(invite, me))) return true;
  // Channel-shared: visible iff the reader belongs to one of the shared `h` channels.
  return board.channels.some((channelId) =>
    memberChannelIds.some((memberId) => samePubkey(memberId, channelId)),
  );
}

/** Parse a board (31001) event into the read model. */
export function parseBoard(event: RelayEvent): KanbanBoard | null {
  if (event.kind !== KIND_KANBAN_BOARD) return null;
  const id = firstTagValue(event, "d");
  const name = firstTagValue(event, "name") ?? "Untitled board";
  if (!id) return null;

  const columns: KanbanColumn[] = [];
  for (const tag of event.tags) {
    const column = parseColumnTag(tag);
    if (column) columns.push(column);
  }
  columns.sort((left, right) => left.order - right.order);

  return {
    id,
    owner: boardOwnerFromEvent(event),
    name,
    description: event.content,
    columns,
    channels: allTagValues(event, "h"),
    invites: allTagValues(event, "invite"),
    createdAt: event.created_at,
  };
}

/** Parse a card (31002) event into the read model. */
export function parseCard(event: RelayEvent): KanbanCard | null {
  if (event.kind !== KIND_KANBAN_CARD) return null;
  const id = firstTagValue(event, "d");
  const board = firstTagValue(event, "a");
  const column = firstTagValue(event, "column");
  if (!id || !board || !column) return null;

  const boardParts = board.split(":");
  const boardOwner = boardParts[1]?.toLowerCase() ?? event.pubkey.toLowerCase();
  const boardId = boardParts.slice(2).join(":");
  const rank = firstTagValue(event, "rank");
  const due = firstTagValue(event, "due");

  return {
    id,
    eventId: event.id,
    board,
    boardOwner,
    boardId,
    column,
    rank,
    title: cardTitleFromContent(event.content, id),
    content: event.content,
    assignees: allTagValues(event, "p"),
    labels: allTagValues(event, "l", (tag) => tag[2] === "kanban"),
    due,
    createdAt: event.created_at,
  };
}

function cardTitleFromContent(content: string, fallback: string): string {
  const heading = content
    .split(/\r?\n/)
    .find((line) => /^\s{0,3}#{1,6}\s+\S/u.test(line));
  if (heading) {
    const title = heading.replace(/^\s{0,3}#{1,6}\s+/u, "").trim();
    if (title.length > 0) return title;
  }
  return fallback;
}

/**
 * Collapse replaceable events to the canonical NIP-33 head per
 * `owner:d` coordinate (latest `created_at` wins, id as tiebreak). Defense in
 * depth — the relay normally returns one head; older/edge relays may not.
 */
function collapseByCoordinate<T>(
  events: readonly RelayEvent[],
  parse: (event: RelayEvent) => T | null,
  coordinate: (event: RelayEvent, value: T) => string,
): T[] {
  const sorted = [...events].sort(
    (left, right) =>
      right.created_at - left.created_at || left.id.localeCompare(right.id),
  );
  const seen = new Set<string>();
  const result: T[] = [];
  for (const event of sorted) {
    const value = parse(event);
    if (!value) continue;
    const key = coordinate(event, value);
    if (seen.has(key)) continue;
    seen.add(key);
    result.push(value);
  }
  return result;
}

export function collapseBoards(events: readonly RelayEvent[]): KanbanBoard[] {
  return collapseByCoordinate(
    events,
    parseBoard,
    (_event, board) => `${board.owner}:${board.id}`,
  );
}

export function collapseCards(events: readonly RelayEvent[]): KanbanCard[] {
  return collapseByCoordinate(
    events,
    parseCard,
    (event, card) => `${event.pubkey.toLowerCase()}:${card.id}`,
  );
}

/**
 * Sort cards by their base-36 order-preserving rank, then by id as a stable
 * tiebreak. Cardinal trumps ordinal: a card with no rank goes last.
 * Comparing the *canonical* rank strings with `<` maps directly to numeric
 * order (the P1 codec guarantees plain lexicographic ordering), so no numeric
 * decode or custom comparator is needed.
 */
export function sortCardsByRank(cards: readonly KanbanCard[]): KanbanCard[] {
  return [...cards].sort((left, right) => {
    if (left.rank === null && right.rank === null) {
      return left.id.localeCompare(right.id);
    }
    if (left.rank === null) return 1;
    if (right.rank === null) return -1;
    if (left.rank === right.rank) return left.id.localeCompare(right.id);
    return left.rank < right.rank ? -1 : 1;
  });
}

/** Group cards by their single `column` colid, each bucket rank-sorted. */
export function groupCardsByColumn(
  cards: readonly KanbanCard[],
): Map<string, KanbanCard[]> {
  const grouped = new Map<string, KanbanCard[]>();
  for (const card of cards) {
    const bucket = grouped.get(card.column);
    if (bucket) {
      bucket.push(card);
    } else {
      grouped.set(card.column, [card]);
    }
  }
  for (const bucket of grouped.values()) {
    sortCardsByRank(bucket);
  }
  return grouped;
}
