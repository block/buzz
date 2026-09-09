import { parseDocument } from "yaml";

export type TaskAssignee = {
  pubkey: string;
  notification?: { id: string; status: "pending" | "sent" };
};

function document(content: string) {
  const match = content.match(/^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/);
  if (!match) throw new Error("Task canvas has no frontmatter");
  const doc = parseDocument(match[1]);
  if (doc.errors.length || doc.get("buzz_schema") !== "channel-backed-task/v1")
    throw new Error("Invalid task canvas");
  return { doc, body: content.slice(match[0].length) };
}

export function taskAssignee(content: string): TaskAssignee | null {
  const value = document(content).doc.toJS().assignee;
  if (!value) return null;
  if (typeof value.pubkey !== "string" || !/^[a-f0-9]{64}$/i.test(value.pubkey))
    throw new Error(
      "Invalid task assignee; correct the canvas before assigning",
    );
  return value;
}

export function assignmentCanvas(
  content: string,
  pubkey: string,
  operationId: string,
  status: "pending" | "sent",
): string {
  if (!/^[a-f0-9]{64}$/i.test(pubkey)) throw new Error("Invalid agent pubkey");
  const existing = taskAssignee(content);
  const { doc, body } = document(content);
  if (!existing) doc.set("assignee", doc.createNode({}));
  doc.setIn(["assignee", "pubkey"], pubkey);
  doc.setIn(["assignee", "notification"], { id: operationId, status });
  return `---\n${doc.toString()}---\n${body}`;
}
