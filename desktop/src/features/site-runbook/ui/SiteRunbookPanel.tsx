import { BookOpen, Check, Pencil, Trash2, X } from "lucide-react";
import * as React from "react";

import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";

import type { SiteRunbook, SiteRunbookProcedure } from "../lib/types";

export type SiteRunbookPanelProps = {
  runbook: SiteRunbook;
  onSetBrief: (brief: string) => void;
  onAccept: (procedureId: string) => void;
  onReject: (procedureId: string) => void;
  onArchive: (procedureId: string) => void;
  onDelete: (procedureId: string) => void;
  onUpdate: (
    procedureId: string,
    patch: { title?: string; steps?: string },
  ) => void;
  onSetPersisted: (procedureId: string, persisted: boolean) => void;
  onAddManual: (title: string, steps: string) => void;
  /** Compact embed vs dialog body. */
  compact?: boolean;
};

export function SiteRunbookPanel({
  runbook,
  onSetBrief,
  onAccept,
  onReject,
  onArchive,
  onDelete,
  onUpdate,
  onSetPersisted,
  onAddManual,
  compact = false,
}: SiteRunbookPanelProps) {
  const [briefDraft, setBriefDraft] = React.useState(runbook.agentBrief);
  const [selectedId, setSelectedId] = React.useState<string | null>(null);
  const [editingId, setEditingId] = React.useState<string | null>(null);
  const [editTitle, setEditTitle] = React.useState("");
  const [editSteps, setEditSteps] = React.useState("");
  const [adding, setAdding] = React.useState(false);
  const [newTitle, setNewTitle] = React.useState("");
  const [newSteps, setNewSteps] = React.useState("");

  React.useEffect(() => {
    setBriefDraft(runbook.agentBrief);
  }, [runbook.agentBrief]);

  const pending = runbook.procedures.filter((p) => p.status === "pending");
  const active = runbook.procedures.filter((p) => p.status === "active");
  const archived = runbook.procedures.filter((p) => p.status === "archived");
  const selected =
    runbook.procedures.find((p) => p.id === selectedId) ?? null;

  function startEdit(procedure: SiteRunbookProcedure) {
    setEditingId(procedure.id);
    setEditTitle(procedure.title);
    setEditSteps(procedure.steps);
    setSelectedId(procedure.id);
  }

  function saveEdit() {
    if (!editingId) return;
    onUpdate(editingId, { title: editTitle, steps: editSteps });
    setEditingId(null);
  }

  const empty =
    !runbook.agentBrief.trim() && runbook.procedures.length === 0;

  return (
    <div
      className={compact ? "space-y-3" : "space-y-4"}
      data-testid="site-runbook-panel"
    >
      <div className="space-y-1.5">
        <div className="flex items-center gap-2">
          <BookOpen className="h-4 w-4 text-muted-foreground" />
          <p className="text-sm font-medium">Agent brief</p>
        </div>
        <p className="text-xs text-muted-foreground">
          Short instructions injected when an agent gets Observe or Drive on
          this browser.
        </p>
        <Textarea
          className="min-h-[72px] text-sm"
          data-testid="site-runbook-brief"
          onBlur={() => {
            if (briefDraft !== runbook.agentBrief) onSetBrief(briefDraft);
          }}
          onChange={(event) => setBriefDraft(event.target.value)}
          placeholder="e.g. Log in with SSO. Prefer the left nav. Never submit without Confirm."
          value={briefDraft}
        />
      </div>

      {pending.length > 0 ? (
        <section className="space-y-2" data-testid="site-runbook-pending">
          <p className="text-sm font-medium">Pending proposals</p>
          <p className="text-2xs text-muted-foreground">
            Legacy pending items. New agent proposals auto-activate unless a
            persisted procedure blocks them.
          </p>
          <ul className="space-y-2">
            {pending.map((procedure) => (
              <li
                className="rounded-lg border border-amber-500/40 bg-amber-500/5 px-3 py-2"
                key={procedure.id}
              >
                <div className="flex items-start justify-between gap-2">
                  <div className="min-w-0">
                    <p className="truncate text-sm font-medium">
                      {procedure.title}
                    </p>
                    {procedure.sourceAgent ? (
                      <p className="text-2xs text-muted-foreground">
                        From agent {procedure.sourceAgent.slice(0, 8)}…
                      </p>
                    ) : null}
                  </div>
                  <div className="flex shrink-0 gap-1">
                    <Button
                      data-testid={`site-runbook-accept-${procedure.id}`}
                      onClick={() => onAccept(procedure.id)}
                      size="xs"
                      type="button"
                      variant="secondary"
                    >
                      <Check className="mr-1 h-3 w-3" />
                      Accept
                    </Button>
                    <Button
                      data-testid={`site-runbook-reject-${procedure.id}`}
                      onClick={() => onReject(procedure.id)}
                      size="xs"
                      type="button"
                      variant="ghost"
                    >
                      <X className="mr-1 h-3 w-3" />
                      Reject
                    </Button>
                  </div>
                </div>
                <pre className="mt-2 max-h-32 overflow-auto whitespace-pre-wrap text-2xs text-muted-foreground">
                  {procedure.steps || "(no steps)"}
                </pre>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      <section className="space-y-2">
        <div className="flex items-center justify-between gap-2">
          <p className="text-sm font-medium">Procedures</p>
          <Button
            data-testid="site-runbook-add"
            onClick={() => setAdding(true)}
            size="xs"
            type="button"
            variant="outline"
          >
            Add
          </Button>
        </div>

        {empty ? (
          <div
            className="rounded-lg border border-dashed border-border/70 px-4 py-6 text-center text-sm text-muted-foreground"
            data-testid="site-runbook-empty"
          >
            No runbook yet. Agents learn how to use this site while they Drive.
            Agents can learn and auto-activate how-tos while they Drive. Check
            Persist to lock an item so agents cannot change it. The agent brief
            is always human-owned.
          </div>
        ) : null}

        {active.length === 0 && !empty ? (
          <p className="text-xs text-muted-foreground">No active procedures.</p>
        ) : null}

        <ul className="divide-y divide-border/60 rounded-lg border border-border/70">
          {active.map((procedure) => (
            <li key={procedure.id}>
              <button
                className="flex w-full items-center justify-between gap-2 px-3 py-2 text-left hover:bg-muted/40"
                data-testid={`site-runbook-proc-${procedure.id}`}
                onClick={() =>
                  setSelectedId((id) =>
                    id === procedure.id ? null : procedure.id,
                  )
                }
                type="button"
              >
                <span className="truncate text-sm">{procedure.title}</span>
                <span className="flex shrink-0 items-center gap-1">
                  {procedure.persisted ? (
                    <Badge
                      className="normal-case tracking-normal"
                      variant="secondary"
                    >
                      persisted
                    </Badge>
                  ) : null}
                  <Badge
                    className="normal-case tracking-normal"
                    variant="outline"
                  >
                    active
                  </Badge>
                </span>
              </button>
              {selected?.id === procedure.id ? (
                <div className="space-y-2 border-t border-border/60 bg-muted/20 px-3 py-2">
                  {editingId === procedure.id ? (
                    <>
                      <Input
                        data-testid="site-runbook-edit-title"
                        onChange={(e) => setEditTitle(e.target.value)}
                        value={editTitle}
                      />
                      <Textarea
                        className="min-h-[100px] text-sm"
                        data-testid="site-runbook-edit-steps"
                        onChange={(e) => setEditSteps(e.target.value)}
                        value={editSteps}
                      />
                      <div className="flex gap-2">
                        <Button
                          onClick={saveEdit}
                          size="xs"
                          type="button"
                          variant="secondary"
                        >
                          Save
                        </Button>
                        <Button
                          onClick={() => setEditingId(null)}
                          size="xs"
                          type="button"
                          variant="ghost"
                        >
                          Cancel
                        </Button>
                      </div>
                    </>
                  ) : (
                    <>
                      <pre className="max-h-48 overflow-auto whitespace-pre-wrap text-2xs text-muted-foreground">
                        {procedure.steps || "(no steps)"}
                      </pre>
                      <label className="flex items-center gap-2 text-xs text-muted-foreground">
                        <input
                          checked={procedure.persisted === true}
                          data-testid={`site-runbook-persist-${procedure.id}`}
                          onChange={(event) =>
                            onSetPersisted(procedure.id, event.target.checked)
                          }
                          type="checkbox"
                        />
                        Persist (lock from agents)
                      </label>
                      <div className="flex flex-wrap gap-1">
                        <Button
                          onClick={() => startEdit(procedure)}
                          size="xs"
                          type="button"
                          variant="ghost"
                        >
                          <Pencil className="mr-1 h-3 w-3" />
                          Edit
                        </Button>
                        <Button
                          onClick={() => onArchive(procedure.id)}
                          size="xs"
                          type="button"
                          variant="ghost"
                        >
                          Archive
                        </Button>
                        <Button
                          onClick={() => onDelete(procedure.id)}
                          size="xs"
                          type="button"
                          variant="ghost"
                        >
                          <Trash2 className="mr-1 h-3 w-3 text-destructive" />
                          Delete
                        </Button>
                      </div>
                    </>
                  )}
                </div>
              ) : null}
            </li>
          ))}
        </ul>

        {archived.length > 0 ? (
          <details className="text-xs text-muted-foreground">
            <summary className="cursor-pointer">
              Archived ({archived.length})
            </summary>
            <ul className="mt-1 space-y-1 pl-2">
              {archived.map((procedure) => (
                <li
                  className="flex items-center justify-between gap-2"
                  key={procedure.id}
                >
                  <span className="truncate">{procedure.title}</span>
                  <Button
                    onClick={() => onDelete(procedure.id)}
                    size="xs"
                    type="button"
                    variant="ghost"
                  >
                    Delete
                  </Button>
                </li>
              ))}
            </ul>
          </details>
        ) : null}
      </section>

      {adding ? (
        <section
          className="space-y-2 rounded-lg border border-border/70 p-3"
          data-testid="site-runbook-add-form"
        >
          <p className="text-sm font-medium">New procedure</p>
          <Input
            data-testid="site-runbook-new-title"
            onChange={(e) => setNewTitle(e.target.value)}
            placeholder="Title"
            value={newTitle}
          />
          <Textarea
            className="min-h-[80px] text-sm"
            data-testid="site-runbook-new-steps"
            onChange={(e) => setNewSteps(e.target.value)}
            placeholder="Steps (markdown)"
            value={newSteps}
          />
          <div className="flex gap-2">
            <Button
              data-testid="site-runbook-new-save"
              disabled={!newTitle.trim()}
              onClick={() => {
                onAddManual(newTitle, newSteps);
                setNewTitle("");
                setNewSteps("");
                setAdding(false);
              }}
              size="xs"
              type="button"
              variant="secondary"
            >
              Save
            </Button>
            <Button
              onClick={() => setAdding(false)}
              size="xs"
              type="button"
              variant="ghost"
            >
              Cancel
            </Button>
          </div>
        </section>
      ) : null}
    </div>
  );
}
