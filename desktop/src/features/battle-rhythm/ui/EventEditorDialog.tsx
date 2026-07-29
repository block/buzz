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
import {
  datePart,
  defaultEventWindow,
  editorEndForEvent,
  editorLocalDateTime,
  eventWindowForSave,
  isCompleteLocalDateTime,
  shiftEndKeepingDuration,
  timePart,
  withDate,
  withTime,
} from "../domain/eventEditorDateTime";

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
  const initialWindow = defaultEventWindow(timeZone);
  const [title, setTitle] = React.useState(draft?.title ?? "");
  const [start, setStart] = React.useState(
    draft ? editorLocalDateTime(draft.start, timeZone) : initialWindow.start,
  );
  const [end, setEnd] = React.useState(
    draft
      ? editorEndForEvent(draft.end, draft.allDay, timeZone)
      : initialWindow.end,
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
    event?.recurrence?.until
      ? editorLocalDateTime(event.recurrence.until, timeZone)
      : "",
  );
  const [exclusions, setExclusions] = React.useState(
    event?.excludedOccurrenceStarts.join(", ") ?? "",
  );
  const [saveError, setSaveError] = React.useState<string | null>(null);
  const [saving, setSaving] = React.useState(false);
  const validStartRef = React.useRef(start);
  React.useEffect(() => {
    if (!open) return;
    const nextDraft = event ?? prefill;
    const nextWindow = defaultEventWindow(timeZone);
    const nextStart = nextDraft
      ? editorLocalDateTime(nextDraft.start, timeZone)
      : nextWindow.start;
    setTitle(nextDraft?.title ?? "");
    setStart(nextStart);
    validStartRef.current = nextStart;
    setEnd(
      nextDraft
        ? editorEndForEvent(nextDraft.end, nextDraft.allDay, timeZone)
        : nextWindow.end,
    );
    setAllDay(nextDraft?.allDay ?? false);
    setOwner(nextDraft?.responsibleOwner ?? "");
    setLocation(event?.location ?? "");
    setRemarks(nextDraft?.remarks ?? "");
    setFrequency(event?.recurrence?.frequency ?? "none");
    setInterval(String(event?.recurrence?.interval ?? 1));
    setUntil(
      event?.recurrence?.until
        ? editorLocalDateTime(event.recurrence.until, timeZone)
        : "",
    );
    setExclusions(event?.excludedOccurrenceStarts.join(", ") ?? "");
    setSaveError(null);
    setSaving(false);
  }, [event, open, prefill, timeZone]);
  const updateStart = (nextStart: string) => {
    if (isCompleteLocalDateTime(nextStart)) {
      const previousStart = validStartRef.current;
      setEnd((currentEnd) =>
        shiftEndKeepingDuration(previousStart, currentEnd, nextStart),
      );
      validStartRef.current = nextStart;
    }
    setStart(nextStart);
  };
  const submit = async (form: React.FormEvent) => {
    form.preventDefault();
    if (!title.trim()) return;
    setSaveError(null);
    setSaving(true);
    try {
      const window = eventWindowForSave(start, end, allDay, timeZone);
      const recurrence =
        frequency === "none"
          ? null
          : {
              frequency: frequency as "daily" | "weekly" | "monthly",
              interval: Number(interval),
              until: until ? localDateTimeToRfc3339(until, timeZone) : null,
              seriesId: event?.recurrence?.seriesId ?? crypto.randomUUID(),
            };
      await onSave({
        schemaVersion: 1,
        id: event?.ownership.kind === "manual" ? event.id : crypto.randomUUID(),
        ownership: { kind: "manual" },
        title: title.trim(),
        description: null,
        type: event?.type ?? prefill?.type ?? "activity",
        start: window.start,
        end: window.end,
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
      <DialogContent className="max-w-2xl">
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
              autoCapitalize="sentences"
              autoCorrect="on"
              className="rounded border bg-background p-2"
              lang="en-AU"
              onChange={(e) => setTitle(e.target.value)}
              required
              spellCheck={true}
              value={title}
            />
          </label>
          <div
            className={`grid gap-3 ${
              frequency === "none"
                ? "grid-cols-1"
                : "sm:grid-cols-[minmax(0,0.55fr)_minmax(0,1fr)]"
            }`}
          >
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
            {frequency !== "none" ? (
              <div className="grid min-w-0 grid-cols-[minmax(5rem,0.4fr)_minmax(0,1fr)] gap-3">
                <label className="grid min-w-0 gap-1 text-sm">
                  Interval
                  <input
                    className="min-w-0 rounded border bg-background p-2"
                    min="1"
                    onChange={(e) => setInterval(e.target.value)}
                    type="number"
                    value={interval}
                  />
                </label>
                <fieldset className="grid min-w-0 gap-1 text-sm">
                  <legend>Until</legend>
                  <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_6.5rem] gap-2">
                    <label className="sr-only" htmlFor="recurrence-until-date">
                      Until date
                    </label>
                    <input
                      className="min-w-0 rounded border bg-background p-2"
                      id="recurrence-until-date"
                      onChange={(e) =>
                        setUntil((current) =>
                          current
                            ? withDate(current, e.target.value)
                            : `${e.target.value}T${timePart(start)}`,
                        )
                      }
                      type="date"
                      value={until ? datePart(until) : ""}
                    />
                    <label className="sr-only" htmlFor="recurrence-until-time">
                      Until time (24 hour)
                    </label>
                    <input
                      className="min-w-0 rounded border bg-background p-2"
                      id="recurrence-until-time"
                      inputMode="numeric"
                      onChange={(e) =>
                        setUntil((current) =>
                          current
                            ? withTime(current, e.target.value)
                            : `${datePart(start)}T${e.target.value}`,
                        )
                      }
                      pattern="(?:[01]\d|2[0-3]):[0-5]\d"
                      placeholder="HH:mm"
                      type="text"
                      value={until ? timePart(until) : ""}
                    />
                  </div>
                </fieldset>
              </div>
            ) : null}
          </div>
          {frequency !== "none" ? (
            <label className="grid gap-1 text-sm">
              Excluded occurrence starts
              <input
                className="rounded border bg-background p-2"
                onChange={(e) => setExclusions(e.target.value)}
                placeholder="2026-08-17T08:00:00+10:00"
                value={exclusions}
              />
            </label>
          ) : null}
          <div className="grid grid-cols-2 gap-3">
            <fieldset className="grid min-w-0 gap-1 text-sm">
              <legend>Start</legend>
              <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_6.5rem] gap-2">
                <label className="sr-only" htmlFor="event-start-date">
                  Start date
                </label>
                <input
                  className="min-w-0 rounded border bg-background p-2"
                  id="event-start-date"
                  onChange={(e) => updateStart(withDate(start, e.target.value))}
                  type="date"
                  value={datePart(start)}
                />
                <label className="sr-only" htmlFor="event-start-time">
                  Start time (24 hour)
                </label>
                <input
                  className="min-w-0 rounded border bg-background p-2"
                  id="event-start-time"
                  inputMode="numeric"
                  onBlur={() => updateStart(start)}
                  onChange={(e) => updateStart(withTime(start, e.target.value))}
                  pattern="(?:[01]\d|2[0-3]):[0-5]\d"
                  placeholder="HH:mm"
                  type="text"
                  value={timePart(start)}
                />
              </div>
            </fieldset>
            <fieldset className="grid min-w-0 gap-1 text-sm">
              <legend>End</legend>
              <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_6.5rem] gap-2">
                <label className="sr-only" htmlFor="event-end-date">
                  End date
                </label>
                <input
                  className="min-w-0 rounded border bg-background p-2"
                  id="event-end-date"
                  onChange={(e) => setEnd(withDate(end, e.target.value))}
                  type="date"
                  value={datePart(end)}
                />
                <label className="sr-only" htmlFor="event-end-time">
                  End time (24 hour)
                </label>
                <input
                  className="min-w-0 rounded border bg-background p-2"
                  id="event-end-time"
                  inputMode="numeric"
                  onChange={(e) => setEnd(withTime(end, e.target.value))}
                  pattern="(?:[01]\d|2[0-3]):[0-5]\d"
                  placeholder="HH:mm"
                  type="text"
                  value={timePart(end)}
                />
              </div>
            </fieldset>
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
                autoCapitalize="sentences"
                autoCorrect="on"
                className="rounded border bg-background p-2"
                lang="en-AU"
                onChange={(e) => setLocation(e.target.value)}
                spellCheck={true}
                value={location}
              />
            </label>
            <label className="grid gap-1 text-sm">
              Responsible owner
              <input
                autoCapitalize="words"
                autoCorrect="on"
                className="rounded border bg-background p-2"
                lang="en-AU"
                onChange={(e) => setOwner(e.target.value)}
                spellCheck={true}
                value={owner}
              />
            </label>
          </div>
          <label className="grid gap-1 text-sm">
            Remarks
            <textarea
              autoCapitalize="sentences"
              autoCorrect="on"
              className="rounded border bg-background p-2"
              lang="en-AU"
              onChange={(e) => setRemarks(e.target.value)}
              spellCheck={true}
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
