import * as React from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import type { BattleRhythmEvent } from "../domain/contracts";
import { localDateTimeToRfc3339 } from "../domain/dateRange";
import type { ProposedCalendarEvent } from "../domain/deterministicChecks";

type Props = {
  event?: BattleRhythmEvent;
  onOpenChange: (open: boolean) => void;
  onSave: (event: BattleRhythmEvent) => Promise<void> | void;
  open: boolean;
  prefill?: ProposedCalendarEvent;
  timeZone: string;
};
export function EventEditorDialog({
  event,
  onOpenChange,
  onSave,
  open,
  prefill,
  timeZone,
}: Props) {
  const draft = event ?? prefill;
  const [title, setTitle] = React.useState(draft?.title ?? "");
  const [start, setStart] = React.useState(
    draft?.start.slice(0, 16) ?? new Date().toISOString().slice(0, 16),
  );
  const [end, setEnd] = React.useState(
    draft?.end.slice(0, 16) ??
      new Date(Date.now() + 3_600_000).toISOString().slice(0, 16),
  );
  const [allDay, setAllDay] = React.useState(draft?.allDay ?? false);
  const [owner, setOwner] = React.useState(draft?.responsibleOwner ?? "");
  const [location, setLocation] = React.useState(event?.location ?? "");
  const [remarks, setRemarks] = React.useState(draft?.remarks ?? "");
  const [frequency, setFrequency] = React.useState(
    event?.recurrence?.frequency ?? "none",
  );
  const [interval, setInterval] = React.useState(
    String(event?.recurrence?.interval ?? 1),
  );
  const [until, setUntil] = React.useState(
    event?.recurrence?.until?.slice(0, 16) ?? "",
  );
  const [exclusions, setExclusions] = React.useState(
    event?.excludedOccurrenceStarts.join(", ") ?? "",
  );
  const [saveError, setSaveError] = React.useState<string | null>(null);
  const [saving, setSaving] = React.useState(false);
  React.useEffect(() => {
    if (!open) return;
    const nextDraft = event ?? prefill;
    setTitle(nextDraft?.title ?? "");
    setStart(
      nextDraft?.start.slice(0, 16) ?? new Date().toISOString().slice(0, 16),
    );
    setEnd(
      nextDraft?.end.slice(0, 16) ??
        new Date(Date.now() + 3_600_000).toISOString().slice(0, 16),
    );
    setAllDay(nextDraft?.allDay ?? false);
    setOwner(nextDraft?.responsibleOwner ?? "");
    setLocation(event?.location ?? "");
    setRemarks(nextDraft?.remarks ?? "");
    setFrequency(event?.recurrence?.frequency ?? "none");
    setInterval(String(event?.recurrence?.interval ?? 1));
    setUntil(event?.recurrence?.until?.slice(0, 16) ?? "");
    setExclusions(event?.excludedOccurrenceStarts.join(", ") ?? "");
    setSaveError(null);
    setSaving(false);
  }, [event, open, prefill]);
  const submit = async (form: React.FormEvent) => {
    form.preventDefault();
    if (!title.trim()) return;
    const recurrence =
      frequency === "none"
        ? null
        : {
            frequency: frequency as "daily" | "weekly" | "monthly",
            interval: Number(interval),
            until: until ? localDateTimeToRfc3339(until, timeZone) : null,
            seriesId: event?.recurrence?.seriesId ?? crypto.randomUUID(),
          };
    setSaveError(null);
    setSaving(true);
    try {
      await onSave({
        schemaVersion: 1,
        id: event?.ownership.kind === "manual" ? event.id : crypto.randomUUID(),
        ownership: { kind: "manual" },
        title: title.trim(),
        description: null,
        type: event?.type ?? prefill?.type ?? "activity",
        start: localDateTimeToRfc3339(start, timeZone),
        end: localDateTimeToRfc3339(end, timeZone),
        allDay,
        timeZone,
        status: "approved",
        location: location || null,
        responsibleOwner: owner || null,
        participants: [],
        remarks: remarks || null,
        linkedPlanId: null,
        linkedTaskId: null,
        linkedMissionRequirementId: null,
        parentActivityId: event?.ownership.kind === "source" ? event.id : null,
        recurrence,
        excludedOccurrenceStarts: exclusions
          ? exclusions.split(/\s*,\s*/).filter(Boolean)
          : [],
      });
      onOpenChange(false);
    } catch (error) {
      setSaveError(
        error instanceof Error ? error.message : "Unable to save this event.",
      );
    } finally {
      setSaving(false);
    }
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>
            {event?.ownership.kind === "source"
              ? "Create local adjustment"
              : event
                ? "Edit manual event"
                : "New manual event"}
          </DialogTitle>
        </DialogHeader>
        {event?.ownership.kind === "source" ? (
          <p className="rounded border border-amber-500/40 bg-amber-500/10 p-3 text-sm">
            The imported source remains unchanged. Saving creates a local
            adjustment that replaces it in the working calendar.
          </p>
        ) : null}
        <form className="grid gap-3" onSubmit={submit}>
          <label className="grid gap-1 text-sm">
            Event
            <input
              autoFocus
              className="rounded border bg-background p-2"
              onChange={(e) => setTitle(e.target.value)}
              required
              value={title}
            />
          </label>
          <div className="grid grid-cols-3 gap-3">
            <label className="grid gap-1 text-sm">
              Recurrence
              <select
                className="rounded border bg-background p-2"
                onChange={(e) => setFrequency(e.target.value)}
                value={frequency}
              >
                <option value="none">None</option>
                <option value="daily">Daily</option>
                <option value="weekly">Weekly</option>
                <option value="monthly">Monthly</option>
              </select>
            </label>
            <label className="grid gap-1 text-sm">
              Interval
              <input
                className="rounded border bg-background p-2"
                disabled={frequency === "none"}
                min="1"
                onChange={(e) => setInterval(e.target.value)}
                type="number"
                value={interval}
              />
            </label>
            <label className="grid gap-1 text-sm">
              Until
              <input
                className="rounded border bg-background p-2"
                disabled={frequency === "none"}
                onChange={(e) => setUntil(e.target.value)}
                type="datetime-local"
                value={until}
              />
            </label>
          </div>
          <label className="grid gap-1 text-sm">
            Excluded occurrence starts
            <input
              className="rounded border bg-background p-2"
              disabled={frequency === "none"}
              onChange={(e) => setExclusions(e.target.value)}
              placeholder="2026-08-17T08:00:00+10:00"
              value={exclusions}
            />
          </label>
          <div className="grid grid-cols-2 gap-3">
            <label className="grid gap-1 text-sm">
              Start
              <input
                className="rounded border bg-background p-2"
                onChange={(e) => setStart(e.target.value)}
                type="datetime-local"
                value={start}
              />
            </label>
            <label className="grid gap-1 text-sm">
              End
              <input
                className="rounded border bg-background p-2"
                onChange={(e) => setEnd(e.target.value)}
                type="datetime-local"
                value={end}
              />
            </label>
          </div>
          <label className="flex items-center gap-2 text-sm">
            <input
              checked={allDay}
              onChange={(e) => setAllDay(e.target.checked)}
              type="checkbox"
            />
            All day
          </label>
          <div className="grid grid-cols-2 gap-3">
            <label className="grid gap-1 text-sm">
              Location
              <input
                className="rounded border bg-background p-2"
                onChange={(e) => setLocation(e.target.value)}
                value={location}
              />
            </label>
            <label className="grid gap-1 text-sm">
              Responsible owner
              <input
                className="rounded border bg-background p-2"
                onChange={(e) => setOwner(e.target.value)}
                value={owner}
              />
            </label>
          </div>
          <label className="grid gap-1 text-sm">
            Remarks
            <textarea
              className="rounded border bg-background p-2"
              onChange={(e) => setRemarks(e.target.value)}
              value={remarks}
            />
          </label>
          <p className="text-2xs text-muted-foreground">
            Time zone: {timeZone}. Exclusions must be sorted occurrence start
            timestamps.
          </p>
          {saveError ? (
            <p className="text-sm text-destructive" role="alert">
              {saveError}
            </p>
          ) : null}
          <button
            className="rounded bg-primary px-3 py-2 text-sm text-primary-foreground"
            disabled={saving}
            type="submit"
          >
            {saving ? "Saving…" : "Save event"}
          </button>
        </form>
      </DialogContent>
    </Dialog>
  );
}
