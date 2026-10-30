import type { Project } from "@/features/projects/hooks";

export function groupProjectWorkItemsByProject<T extends { project: Project }>(
  rows: T[],
) {
  const groups = new Map<string, { project: Project; rows: T[] }>();
  for (const row of rows) {
    const existing = groups.get(row.project.id);
    if (existing) {
      existing.rows.push(row);
    } else {
      groups.set(row.project.id, { project: row.project, rows: [row] });
    }
  }
  return [...groups.values()];
}
