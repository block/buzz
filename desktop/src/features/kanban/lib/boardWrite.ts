import { KIND_KANBAN_BOARD } from "@/shared/constants/kinds";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";

export type KanbanTemplateName = "kanban" | "sprint" | "sales" | "blank";

export type KanbanTemplateColumn = { name: string; wip: number | null };

/**
 * Column definitions per template, mirroring the CLI's `template_columns()`
 * (crates/buzz-cli/src/commands/boards.rs). `wip: null` = unbounded and must
 * **not** emit a `wip` tag pair (the same no-WIP contract the P1/P2 parsers
 * guard on read).
 */
export const KANBAN_TEMPLATES: Record<
  KanbanTemplateName,
  readonly KanbanTemplateColumn[]
> = {
  kanban: [
    { name: "Backlog", wip: 5 },
    { name: "In Progress", wip: 3 },
    { name: "Review", wip: null },
    { name: "Done", wip: null },
  ],
  sprint: [
    { name: "To Do", wip: 5 },
    { name: "In Progress", wip: 3 },
    { name: "Blocked", wip: null },
    { name: "Done", wip: null },
  ],
  sales: [
    { name: "Lead", wip: null },
    { name: "Qualified", wip: null },
    { name: "Proposal", wip: null },
    { name: "Won", wip: null },
    { name: "Lost", wip: null },
  ],
  blank: [{ name: "Backlog", wip: null }],
};

export type BoardDraft = {
  name: string;
  description?: string;
  template: KanbanTemplateName;
  /** Owner pubkey (lowercased) — the Desktop identity the board is created under. */
  owner: string;
};

export type BoardEventDraft = {
  kind: number;
  content: string;
  tags: string[][];
  boardId: string;
};

/** `col-` + the first 8 hex chars of a fresh UUID (mirrors the CLI `new_colid()`). */
function newColid(): string {
  // strip the UUID dashes, keep the first 8 hex chars
  return `col-${crypto.randomUUID().replaceAll("-", "").slice(0, 8)}`;
}

function boardContent(name: string, description?: string): string {
  const trimmedDescription = description?.trim();
  return trimmedDescription
    ? `## ${name}\n\n${trimmedDescription}`
    : `## ${name}`;
}

/**
 * Produce the canonical kind-31001 board event shape (tags + content) exactly
 * as the CLI/SDK `build_kanban_board` emits it: `d` / `name` / `p(owner)`
 * tags, one `column` tag per template column (`wip` omitted when unbounded,
 * `order` 0-based contiguous), and **no** `h` / `invite` tags (private by
 * default — matches the CLI). Pure: no signing or network here; callers sign
 * and publish the returned shape.
 */
export function buildBoardEvent(draft: BoardDraft): BoardEventDraft {
  const name = draft.name.trim();
  const boardId = crypto.randomUUID();
  const columns = KANBAN_TEMPLATES[draft.template];

  const tags: string[][] = [
    ["d", boardId],
    ["name", name],
    ["p", draft.owner.toLowerCase(), "owner"],
  ];
  columns.forEach((column, index) => {
    const parts: string[] = ["column", newColid(), "name", column.name];
    if (column.wip !== null) {
      parts.push("wip", String(column.wip));
    }
    parts.push("order", String(index));
    tags.push(parts);
  });

  return {
    kind: KIND_KANBAN_BOARD,
    content: boardContent(name, draft.description),
    tags,
    boardId,
  };
}

/**
 * Sign (under the Desktop identity) and publish a new board, returning its id
 * for navigation.
 *
 * A fresh random board id makes a NIP-33 collision effectively impossible, so
 * publish-to-OK is sufficient here. This helper is the template for inline
 * card create/move where LWW conflicts are real — that path must also inspect
 * the relay's `duplicate:`/dominated response the way the CLI's
 * `submit_lww_event` does (relay rejection is exit 5 in the CLI) rather than
 * assuming success.
 */
export async function createBoard(
  draft: BoardDraft,
): Promise<{ boardId: string }> {
  const built = buildBoardEvent(draft);
  const event = await signRelayEvent({
    kind: built.kind,
    content: built.content,
    tags: built.tags,
  });
  await relayClient.publishEvent(
    event,
    "Timed out while creating the board.",
    "Failed to create the board.",
  );
  return { boardId: built.boardId };
}
