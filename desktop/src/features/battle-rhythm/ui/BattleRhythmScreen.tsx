import * as React from "react";
import { ChevronLeft, ChevronRight, Filter, History, Plus } from "lucide-react";
import { useIdentityQuery } from "@/shared/api/hooks";
import { readAppleInputs } from "@/shared/api/tauriAppleInputs";
import {
  publishBattleRhythmToApple,
  type ApplePublicationStatus,
} from "../data/applePublication";
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
  const [appleStatus, setAppleStatus] =
    React.useState<ApplePublicationStatus>();
  const [appleBusy, setAppleBusy] = React.useState(false);
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
  const publishToApple = React.useCallback(async () => {
    setAppleBusy(true);
    try {
      const status = await publishBattleRhythmToApple(events, range);
      setAppleStatus(status);
    } catch (cause) {
      setAppleStatus({
        state: "unavailable",
        permission: "unavailable",
        calendarIdentifier: null,
        created: 0,
        updated: 0,
        deleted: 0,
        unchanged: 0,
        error:
          cause instanceof Error
            ? cause.message
            : "Apple Calendar publication is unavailable.",
      });
    } finally {
      setAppleBusy(false);
    }
  }, [events, range]);
  React.useEffect(() => {
    if (!rhythm.isSuccess) return;
    setAppleStatus((current) =>
      current
        ? { ...current, state: "changes_pending" }
        : {
            state: "changes_pending",
            permission: "unavailable",
            calendarIdentifier: null,
            created: 0,
            updated: 0,
            deleted: 0,
            unchanged: 0,
            error: null,
          },
    );
    void publishToApple();
  }, [publishToApple, rhythm.isSuccess]);
  async function handleApplePublication() {
    if (appleStatus?.state === "permission_required") {
      await readAppleInputs({
        operation: "request_permission",
        arguments: { source: "calendar" },
      });
    }
    await publishToApple();
  }
  const appleLabel = appleBusy
    ? "Publishing…"
    : appleStatus?.state === "published"
      ? "Published to Apple"
      : appleStatus?.state === "permission_required"
        ? "Allow Apple Calendar"
        : appleStatus?.state === "changes_pending"
          ? "Changes pending"
          : "Retry Apple publication";
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
            className="rounded border px-3 py-2 text-sm disabled:opacity-50"
            disabled={appleBusy || !rhythm.isSuccess}
            onClick={handleApplePublication}
            title={
              appleStatus?.error ??
              "One-way publication to HMAS Supply Battle Rhythm"
            }
            type="button"
          >
            {appleLabel}
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
