import * as React from "react";
import { CopyPlus, Plus } from "lucide-react";
import {
  parsePlanningPlaybook,
  type PlanningPlaybookV1,
  type PlaybookTaskTemplateV1,
} from "../domain/extendedContracts";
import {
  schedulePlaybook,
  type PlaybookScheduleContext,
  type ScheduledPlaybookTask,
} from "../domain/playbookSchedule";

const PRE_DEPARTURE_TEMPLATES: readonly PlaybookTaskTemplateV1[] =
  Object.freeze(
    [
      ["nav-plan", "Navigation plan briefed", "Navigation", "Navigator", 7200],
      [
        "pilotage",
        "Departure pilotage briefed",
        "Navigation",
        "Navigator",
        4320,
      ],
      ["passage", "Passage plan promulgated", "Navigation", "Navigator", 2880],
      ["optask", "OPTASK RAS sent", "Operations", "Operations Officer", 2880],
      [
        "stores",
        "Mission-essential stores embarked",
        "SO",
        "Supply Officer",
        1440,
      ],
      [
        "engineering",
        "Engineering readiness confirmed",
        "MEO",
        "Marine Engineering Officer",
        1440,
      ],
      [
        "rounds",
        "Securing for sea rounds complete",
        "XO",
        "Executive Officer",
        960,
      ],
      ["review", "Command readiness review", "XO", "Executive Officer", 480],
    ].map(([id, title, department, position, offset], index) =>
      Object.freeze({
        id: String(id),
        title: String(title),
        instructions: `Complete and report ${String(title).toLowerCase()}.`,
        timing: "before" as const,
        offsetMinutes: Number(offset),
        durationMinutes: index === 7 ? 60 : 120,
        dependencyIds:
          index === 7
            ? Object.freeze([
                "nav-plan",
                "pilotage",
                "passage",
                "optask",
                "stores",
                "engineering",
                "rounds",
              ])
            : Object.freeze([]),
        department: String(department),
        position: String(position),
        agentId: index === 3 ? "builtin:command-operations" : null,
        outputType: "response" as const,
        reschedulable: true,
        locked: false,
        linkedCapabilityId: null,
        linkedMissionRequirementId: null,
      }),
    ),
  );

export function defaultPreDeparturePlaybook(): PlanningPlaybookV1 {
  const now = new Date().toISOString();
  return parsePlanningPlaybook({
    schemaVersion: 1,
    id: crypto.randomUUID(),
    title: "Pre-Departure",
    description:
      "Standard preparation for sailing, scheduled against ship routine.",
    status: "active",
    revisionId: crypto.randomUUID(),
    taskTemplates: PRE_DEPARTURE_TEMPLATES,
    createdAt: now,
    updatedAt: now,
  });
}

function parseLines(lines: string): readonly PlaybookTaskTemplateV1[] {
  return Object.freeze(
    lines
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean)
      .map((line, index) => {
        const [title, department = "XO", position = department, days = "1"] =
          line.split("|").map((value) => value.trim());
        return Object.freeze({
          id: crypto.randomUUID(),
          title: title || `Task ${index + 1}`,
          instructions: `Complete and report ${title || `Task ${index + 1}`}.`,
          timing: "before" as const,
          offsetMinutes: Math.max(0, Number(days) || 0) * 1440,
          durationMinutes: 120,
          dependencyIds: Object.freeze([]),
          department,
          position,
          agentId: null,
          outputType: "response" as const,
          reschedulable: true,
          locked: false,
          linkedCapabilityId: null,
          linkedMissionRequirementId: null,
        });
      }),
  );
}

export function PlaybookWorkspace({
  playbooks,
  routineAt,
  onSave,
  onApply,
}: {
  playbooks: readonly PlanningPlaybookV1[];
  routineAt: (
    date: string,
  ) => Pick<PlaybookScheduleContext, "routine" | "timeZone">;
  onSave: (playbook: PlanningPlaybookV1) => Promise<void>;
  onApply: (
    playbook: PlanningPlaybookV1,
    tasks: readonly ScheduledPlaybookTask[],
  ) => Promise<void>;
}) {
  const [title, setTitle] = React.useState("");
  const [description, setDescription] = React.useState("");
  const [lines, setLines] = React.useState("");
  const [selectedId, setSelectedId] = React.useState(playbooks[0]?.id ?? "");
  const [anchorDate, setAnchorDate] = React.useState(
    new Date().toISOString().slice(0, 10),
  );
  const [anchorTime, setAnchorTime] = React.useState("08:00");
  const [proposal, setProposal] =
    React.useState<readonly ScheduledPlaybookTask[]>();
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string>();
  const selected = playbooks.find((item) => item.id === selectedId);
  React.useEffect(() => {
    if (!selectedId && playbooks[0]) setSelectedId(playbooks[0].id);
  }, [playbooks, selectedId]);
  async function createCustom() {
    const now = new Date().toISOString();
    const templates = parseLines(lines);
    if (!title.trim() || templates.length === 0) return;
    setBusy(true);
    try {
      await onSave(
        parsePlanningPlaybook({
          schemaVersion: 1,
          id: crypto.randomUUID(),
          title,
          description: description.trim() || `${title} operational playbook`,
          status: "active",
          revisionId: crypto.randomUUID(),
          taskTemplates: templates,
          createdAt: now,
          updatedAt: now,
        }),
      );
      setTitle("");
      setDescription("");
      setLines("");
    } finally {
      setBusy(false);
    }
  }
  function preview() {
    if (!selected) return;
    setError(undefined);
    try {
      const routine = routineAt(anchorDate);
      setProposal(
        schedulePlaybook(selected, {
          anchorDate,
          anchorTime,
          ...routine,
        }),
      );
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not schedule playbook.",
      );
    }
  }
  async function apply() {
    if (!selected || !proposal) return;
    setBusy(true);
    try {
      await onApply(selected, proposal);
      setProposal(undefined);
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Some playbook tasks were not saved.",
      );
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="grid gap-5" data-testid="playbook-workspace">
      <div className="rounded-xl border bg-card p-4">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="font-semibold">Operational playbooks</h3>
            <p className="text-sm text-muted-foreground">
              Text-first reusable task sets. Dates are reviewed before anything
              is added to the plan.
            </p>
          </div>
          {!playbooks.some((item) => item.title === "Pre-Departure") ? (
            <button
              className="rounded border px-3 py-2 text-sm"
              onClick={() => void onSave(defaultPreDeparturePlaybook())}
              type="button"
            >
              <CopyPlus className="mr-1 inline h-4 w-4" />
              Add Pre-Departure
            </button>
          ) : null}
        </div>
        <div className="mt-4 grid gap-3 md:grid-cols-3">
          <label className="grid gap-1 text-sm">
            Playbook
            <select
              className="rounded border bg-background px-3 py-2"
              onChange={(event) => {
                setSelectedId(event.target.value);
                setProposal(undefined);
              }}
              value={selectedId}
            >
              <option value="">Select a playbook</option>
              {playbooks
                .filter((item) => item.status === "active")
                .map((playbook) => (
                  <option key={playbook.id} value={playbook.id}>
                    {playbook.title} ({playbook.taskTemplates.length})
                  </option>
                ))}
            </select>
          </label>
          <label className="grid gap-1 text-sm">
            Anchor date
            <input
              className="rounded border bg-background px-3 py-2"
              onChange={(event) => setAnchorDate(event.target.value)}
              type="date"
              value={anchorDate}
            />
          </label>
          <label className="grid gap-1 text-sm">
            Anchor time (ship time)
            <input
              className="rounded border bg-background px-3 py-2"
              onChange={(event) => setAnchorTime(event.target.value)}
              type="time"
              value={anchorTime}
            />
          </label>
        </div>
        <button
          className="mt-3 rounded bg-primary px-3 py-2 text-sm text-primary-foreground disabled:opacity-50"
          disabled={!selected || !anchorDate || !anchorTime}
          onClick={preview}
          type="button"
        >
          Preview schedule
        </button>
      </div>
      {proposal && selected ? (
        <div className="rounded-xl border p-4">
          <h3 className="font-semibold">Review before applying</h3>
          <div className="mt-3 grid gap-2">
            {proposal.map((item) => (
              <div
                className="rounded border p-3 text-sm"
                key={item.template.id}
              >
                <p className="font-medium">{item.template.title}</p>
                <p className="text-xs text-muted-foreground">
                  {item.plannedStart} {item.plannedStartTime} → {item.dueDate}{" "}
                  {item.dueTime} · {item.template.department} · {item.timeZone}
                </p>
              </div>
            ))}
          </div>
          <button
            className="mt-3 rounded bg-primary px-3 py-2 text-sm text-primary-foreground disabled:opacity-50"
            disabled={busy}
            onClick={() => void apply()}
            type="button"
          >
            {busy ? "Applying…" : "Apply scheduled tasks"}
          </button>
        </div>
      ) : null}
      {error ? (
        <p className="rounded border border-red-500/40 bg-red-500/10 p-3 text-sm text-red-800 dark:text-red-200">
          {error}
        </p>
      ) : null}
      <div className="rounded-xl border p-4">
        <h3 className="font-semibold">Build a playbook</h3>
        <div className="mt-3 grid gap-3">
          <label className="grid gap-1 text-sm">
            Name
            <input
              className="rounded border bg-background px-3 py-2"
              onChange={(event) => setTitle(event.target.value)}
              value={title}
            />
          </label>
          <label className="grid gap-1 text-sm">
            Description
            <input
              className="rounded border bg-background px-3 py-2"
              onChange={(event) => setDescription(event.target.value)}
              value={description}
            />
          </label>
          <label className="grid gap-1 text-sm">
            Tasks — one per line: Task | HOD | Position | days before anchor
            <textarea
              className="min-h-32 rounded border bg-background px-3 py-2 font-mono text-sm"
              onChange={(event) => setLines(event.target.value)}
              placeholder="Securing for sea rounds | XO | Executive Officer | 2"
              value={lines}
            />
          </label>
          <button
            className="w-fit rounded border px-3 py-2 text-sm disabled:opacity-50"
            disabled={busy || !title.trim() || !lines.trim()}
            onClick={() => void createCustom()}
            type="button"
          >
            <Plus className="mr-1 inline h-4 w-4" />
            Save playbook
          </button>
        </div>
      </div>
    </section>
  );
}
