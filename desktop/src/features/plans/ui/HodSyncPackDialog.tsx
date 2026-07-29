import * as React from "react";
import { FileText } from "lucide-react";
import { openPath } from "@tauri-apps/plugin-opener";
import {
  generateHodSyncPack,
  type ArtifactWriteResult,
} from "@/shared/api/tauriProjectExecution";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import type { HodGroup, HodSyncItem, HodSyncPack } from "../domain/hodSyncPack";

const GROUPS = ["XO", "MEO", "WEEO", "SO", "other"] as const;

function groupBody(items: readonly HodSyncItem[]) {
  return items
    .map((item) => {
      const flags = [
        item.overdue ? "OVERDUE" : null,
        item.critical ? "CRITICAL PATH" : null,
      ]
        .filter(Boolean)
        .join(" / ");
      const dependencies = item.incompleteDependencies.length
        ? `Waiting on: ${item.incompleteDependencies.join(", ")}`
        : "Dependencies: ready";
      return [
        `[ ] ${item.task.wbs} ${item.task.title}`,
        `    Due: ${item.task.dueDate ?? "not set"} ${item.details.dueTime ?? "16:00"} | Status: ${item.task.status}${flags ? ` | ${flags}` : ""}`,
        `    Responsible: ${item.details.position}${item.details.individual ? ` — ${item.details.individual}` : ""}`,
        `    ${dependencies}`,
        "    Decision / notes: ______________________________________________",
        "",
      ].join("\n");
    })
    .join("\n");
}

export function HodSyncPackDialog({
  open,
  onOpenChange,
  pack,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  pack: HodSyncPack;
}) {
  const [busy, setBusy] = React.useState<HodGroup | "combined">();
  const [result, setResult] = React.useState<ArtifactWriteResult>();
  const [error, setError] = React.useState<string>();
  async function generate(group: HodGroup | "combined") {
    setBusy(group);
    setError(undefined);
    try {
      const items = group === "combined" ? pack.combined : pack.groups[group];
      setResult(
        await generateHodSyncPack({
          projectTitle: pack.project.title,
          group: group === "combined" ? "Combined" : group,
          body: groupBody(items),
        }),
      );
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not create Sync Pack.",
      );
    } finally {
      setBusy(undefined);
    }
  }
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[88vh] max-w-4xl overflow-auto">
        <DialogHeader>
          <DialogTitle>HOD Sync Pack</DialogTitle>
        </DialogHeader>
        <p className="text-sm text-muted-foreground">
          Overdue work appears first, followed by critical-path items and then
          due-date order. Completed work is excluded.
        </p>
        <div className="flex flex-wrap gap-2">
          <button
            className="rounded bg-primary px-3 py-2 text-sm text-primary-foreground disabled:opacity-50"
            disabled={Boolean(busy)}
            onClick={() => void generate("combined")}
            type="button"
          >
            <FileText className="mr-1 inline h-4 w-4" />
            {busy === "combined" ? "Generating…" : "Combined PDF"}
          </button>
          {GROUPS.map((group) => (
            <button
              className="rounded border px-3 py-2 text-sm disabled:opacity-50"
              disabled={Boolean(busy) || pack.groups[group].length === 0}
              key={group}
              onClick={() => void generate(group)}
              type="button"
            >
              {group === "other" ? "Other" : group} PDF
            </button>
          ))}
        </div>
        {error ? (
          <p className="rounded border border-red-500/40 bg-red-500/10 p-3 text-sm text-red-800 dark:text-red-200">
            {error}
          </p>
        ) : null}
        {result ? (
          <div className="rounded border bg-muted/30 p-3 text-sm">
            <p className="font-medium">Created {result.fileName}</p>
            <p className="break-all text-xs text-muted-foreground">
              {result.path}
            </p>
            <button
              className="mt-2 rounded border px-3 py-1 text-sm"
              onClick={() => void openPath(result.path)}
              type="button"
            >
              Open PDF
            </button>
          </div>
        ) : null}
        <div className="grid gap-4">
          {GROUPS.map((group) =>
            pack.groups[group].length ? (
              <section className="rounded-lg border p-4" key={group}>
                <h3 className="font-semibold">
                  {group === "other" ? "Other departments" : group}
                </h3>
                <div className="mt-2 grid gap-2">
                  {pack.groups[group].map((item) => (
                    <article
                      className="rounded border bg-card p-3 text-sm"
                      key={item.task.id}
                    >
                      <p className="font-medium">
                        □ {item.task.wbs} {item.task.title}
                      </p>
                      <p className="text-xs text-muted-foreground">
                        Due {item.task.dueDate ?? "not set"}{" "}
                        {item.details.dueTime ?? "16:00"} ·{" "}
                        {item.details.position}
                        {item.details.individual
                          ? ` — ${item.details.individual}`
                          : ""}
                      </p>
                      {item.overdue || item.critical ? (
                        <p className="mt-1 text-xs font-medium text-red-700 dark:text-red-300">
                          {item.overdue ? "OVERDUE " : ""}
                          {item.critical ? "CRITICAL PATH" : ""}
                        </p>
                      ) : null}
                    </article>
                  ))}
                </div>
              </section>
            ) : null,
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
