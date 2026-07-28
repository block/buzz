import * as React from "react";
import { ChevronLeft, ChevronRight, Filter, History, Plus } from "lucide-react";
import { useIdentityQuery } from "@/shared/api/hooks";
import { dayInTimeZone, getYearRange } from "../domain/dateRange";
import { expandRecurringEvents } from "../domain/occurrences";
import type { BattleRhythmEvent } from "../domain/contracts";
import { useBattleRhythmMutations, useBattleRhythmQuery } from "../hooks";
import { DayShortcast } from "./DayShortcast";
import { EventEditorDialog } from "./EventEditorDialog";
import { ImportReviewDialog } from "./ImportReviewDialog";
import { MonthCalendar } from "./MonthCalendar";
import { SourceHistoryDialog } from "./SourceHistoryDialog";
import { WeekCalendar } from "./WeekCalendar";
import { YearTimeline } from "./YearTimeline";

type CalendarView = "Year" | "Month" | "Week" | "Day";
const TIME_ZONE = "Australia/Sydney";
function shiftDay(day: string, amount: number) {
  const next = new Date(`${day}T12:00:00Z`);
  next.setUTCDate(next.getUTCDate() + amount);
  return next.toISOString().slice(0, 10);
}
export function BattleRhythmScreen() {
  const identity = useIdentityQuery();
  const [view, setView] = React.useState<CalendarView>("Week");
  const [day, setDay] = React.useState(() =>
    dayInTimeZone(new Date(), TIME_ZONE),
  );
  const [editorOpen, setEditorOpen] = React.useState(false);
  const [editingEvent, setEditingEvent] = React.useState<
    BattleRhythmEvent | undefined
  >();
  const [historyOpen, setHistoryOpen] = React.useState(false);
  const [importOpen, setImportOpen] = React.useState(false);
  const range = React.useMemo(() => getYearRange(day, TIME_ZONE, 24), [day]);
  const rhythm = useBattleRhythmQuery(identity.data?.pubkey, range);
  const mutations = useBattleRhythmMutations(
    identity.data?.pubkey ?? "",
    range,
  );
  const events = React.useMemo(
    () => expandRecurringEvents(rhythm.data?.events ?? [], range),
    [range, rhythm.data?.events],
  );
  const visibleEvents = events.filter(
    (event) => event.start.slice(0, 10) === day || view !== "Day",
  );
  const renderView = () => {
    if (view === "Year") return <YearTimeline events={events} />;
    if (view === "Month")
      return (
        <MonthCalendar
          day={day}
          events={events}
          onEdit={openEditor}
          timeZone={TIME_ZONE}
        />
      );
    if (view === "Week")
      return (
        <WeekCalendar
          day={day}
          events={events}
          onEdit={openEditor}
          timeZone={TIME_ZONE}
        />
      );
    return (
      <DayShortcast
        events={visibleEvents}
        routineState="Alongside"
        timeZone={TIME_ZONE}
        onEdit={openEditor}
      />
    );
  };
  function openEditor(event?: BattleRhythmEvent) {
    const original = event
      ? rhythm.data?.events.find(
          (candidate) =>
            candidate.id === event.id ||
            event.id.startsWith(`${candidate.id}:`),
        )
      : undefined;
    setEditingEvent(original);
    setEditorOpen(true);
  }
  return (
    <main
      className="flex min-h-0 flex-1 flex-col overflow-auto p-6"
      data-testid="battle-rhythm-screen"
    >
      <div className="mb-6 flex flex-wrap items-center justify-between gap-3">
        <div>
          <p className="text-2xs uppercase tracking-[0.18em] text-primary">
            Command planning
          </p>
          <h1 className="text-xl font-semibold">Battle Rhythm</h1>
          <p className="text-sm text-muted-foreground">
            A usable schedule even when no sources, model, RAG, or Apple
            permissions are available.
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button
            className="rounded border px-3 py-2 text-sm"
            onClick={() => setDay(dayInTimeZone(new Date(), TIME_ZONE))}
            type="button"
          >
            Today
          </button>
          <button
            aria-label="Previous range"
            className="rounded border p-2"
            onClick={() =>
              setDay(
                shiftDay(
                  day,
                  view === "Year"
                    ? -365
                    : view === "Month"
                      ? -30
                      : view === "Week"
                        ? -7
                        : -1,
                ),
              )
            }
            type="button"
          >
            <ChevronLeft className="h-4 w-4" />
          </button>
          <button
            aria-label="Next range"
            className="rounded border p-2"
            onClick={() =>
              setDay(
                shiftDay(
                  day,
                  view === "Year"
                    ? 365
                    : view === "Month"
                      ? 30
                      : view === "Week"
                        ? 7
                        : 1,
                ),
              )
            }
            type="button"
          >
            <ChevronRight className="h-4 w-4" />
          </button>
          <select
            aria-label="Calendar view"
            className="rounded border bg-background px-3 text-sm"
            onChange={(e) => setView(e.target.value as CalendarView)}
            value={view}
          >
            {(["Year", "Month", "Week", "Day"] as const).map((item) => (
              <option key={item}>{item}</option>
            ))}
          </select>
          <button
            className="rounded border px-3 py-2 text-sm"
            onClick={() => setHistoryOpen(true)}
            type="button"
          >
            <History className="mr-1 inline h-4 w-4" />
            History
          </button>
          <button
            className="rounded border px-3 py-2 text-sm"
            onClick={() => setImportOpen(true)}
            type="button"
          >
            Import Document
          </button>
          <button
            className="rounded border px-3 py-2 text-sm"
            disabled
            title="Planning Review is not configured"
            type="button"
          >
            Planning Review
          </button>
          <button className="rounded border px-3 py-2 text-sm" type="button">
            <Filter className="mr-1 inline h-4 w-4" />
            Filters
          </button>
          <button
            className="rounded border px-3 py-2 text-sm"
            disabled
            type="button"
          >
            Apple unavailable
          </button>
          <button
            className="rounded bg-primary px-3 py-2 text-sm text-primary-foreground"
            onClick={() => openEditor()}
            type="button"
          >
            <Plus className="mr-1 inline h-4 w-4" />
            New Event
          </button>
        </div>
      </div>
      {rhythm.isLoading ? (
        <p className="text-sm text-muted-foreground">Loading calendar…</p>
      ) : rhythm.isError ? (
        <p className="text-sm text-muted-foreground">
          Calendar data is unavailable. You can still plan once a relay identity
          is connected.
        </p>
      ) : (
        renderView()
      )}
      <EventEditorDialog
        event={editingEvent}
        onOpenChange={(open) => {
          setEditorOpen(open);
          if (!open) setEditingEvent(undefined);
        }}
        onSave={async (event) => {
          await mutations.manual.mutateAsync(event);
        }}
        open={editorOpen}
        timeZone={TIME_ZONE}
      />
      <SourceHistoryDialog
        onOpenChange={setHistoryOpen}
        open={historyOpen}
        sources={rhythm.data?.sources ?? []}
      />
      <ImportReviewDialog
        coverage={range}
        events={rhythm.data?.events ?? []}
        onApply={async (input) => {
          await mutations.importRevision.mutateAsync(input);
        }}
        onOpenChange={setImportOpen}
        open={importOpen}
        ownerPubkey={identity.data?.pubkey ?? ""}
        sources={rhythm.data?.sources ?? []}
      />
    </main>
  );
}
