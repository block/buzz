import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import {
  api,
  fmtDay,
  fmtTime,
  fmtTokens,
  selectionIsRunnable,
  shortId,
  type Bucket,
  type PageCursor,
  type Selection,
  type TimeWindow,
} from "../api";

type Props = { selection: Selection; window: TimeWindow };

/** Selection preview: stats, daily rhythm, and the actual matching events. */
export function Explore({ selection, window }: Props) {
  const runnable = selectionIsRunnable(selection);

  const preview = useQuery({
    queryKey: ["preview", selection, window],
    queryFn: () => api.preview(selection, window),
    enabled: runnable,
  });

  const events = useInfiniteQuery({
    queryKey: ["events", selection, window],
    queryFn: ({ pageParam }) => api.events(selection, window, 50, pageParam),
    initialPageParam: null as PageCursor | null,
    getNextPageParam: (last) => last.next,
    enabled: runnable,
  });

  if (!runnable) {
    return (
      <div className="center-note">
        Pick at least one channel or author on the left to preview a selection.
        <br />
        Everything here is free — no model is called until you press Run on a
        fold.
      </div>
    );
  }

  const p = preview.data;
  const allEvents = events.data?.pages.flatMap((page) => page.events) ?? [];

  return (
    <div>
      {preview.isError && (
        <div className="error-box">{(preview.error as Error).message}</div>
      )}
      {p && (
        <div className="stat-row">
          <div className="stat">
            <div className="num">{p.count.toLocaleString()}</div>
            <div className="cap">events</div>
          </div>
          <div className="stat">
            <div className="num">{p.total_chars.toLocaleString()}</div>
            <div className="cap">chars</div>
          </div>
          <div className="stat">
            <div className="num">~{fmtTokens(Math.ceil(p.total_chars / 4))}</div>
            <div className="cap">est. input tokens</div>
          </div>
          {p.oldest_ts !== null && p.newest_ts !== null && (
            <div className="stat">
              <div className="num" style={{ fontSize: 14, paddingTop: 5 }}>
                {fmtTime(p.oldest_ts)} → {fmtTime(p.newest_ts)}
              </div>
              <div className="cap">span</div>
            </div>
          )}
        </div>
      )}

      {p && p.buckets.length > 0 && <RhythmStrip buckets={p.buckets} />}

      {allEvents.map((ev) => (
        <EventRow
          key={ev.id}
          id={ev.id}
          author={ev.author_name ?? shortId(ev.pubkey)}
          time={ev.created_at}
          content={ev.content}
        />
      ))}
      {events.hasNextPage && (
        <button
          onClick={() => events.fetchNextPage()}
          disabled={events.isFetchingNextPage}
        >
          {events.isFetchingNextPage ? "loading…" : "load more"}
        </button>
      )}
      {events.data && allEvents.length === 0 && (
        <div className="center-note">No events match this selection.</div>
      )}
    </div>
  );
}

function EventRow({
  id,
  author,
  time,
  content,
}: {
  id: string;
  author: string;
  time: number;
  content: string;
}) {
  const [expanded, setExpanded] = useState(false);
  return (
    <div
      className={`event-row${expanded ? " expanded" : ""}`}
      onClick={() => setExpanded((e) => !e)}
    >
      <div className="head">
        <strong>{author}</strong>
        <span>{fmtTime(time)}</span>
        <span className="mono faint">{shortId(id)}</span>
      </div>
      <div className="body">{content}</div>
    </div>
  );
}

/** Daily activity histogram. Single series: one hue, gaps filled with
 * zero-height days so rhythm (and silence) is visible. */
function RhythmStrip({ buckets }: { buckets: Bucket[] }) {
  const DAY = 86_400;
  const first = buckets[0];
  const last = buckets[buckets.length - 1];
  if (!first || !last) return null;

  const byDay = new Map(buckets.map((b) => [b.day, b.count]));
  const days: Bucket[] = [];
  for (let d = first.day; d <= last.day; d += DAY) {
    days.push({ day: d, count: byDay.get(d) ?? 0 });
  }
  // Beyond ~a year of days, group whole weeks so bars stay visible.
  const perWeek = days.length > 366;
  const cells: Bucket[] = perWeek
    ? Array.from({ length: Math.ceil(days.length / 7) }, (_, i) => ({
        day: days[i * 7]?.day ?? 0,
        count: days
          .slice(i * 7, i * 7 + 7)
          .reduce((acc, b) => acc + b.count, 0),
      }))
    : days;

  const w = 800;
  const h = 64;
  const gap = 2;
  const bw = Math.max(1, w / cells.length - gap);
  const max = Math.max(1, ...cells.map((c) => c.count));

  return (
    <div className="rhythm">
      <div className="cap faint" style={{ marginBottom: 6, fontSize: 11 }}>
        ACTIVITY BY {perWeek ? "WEEK" : "DAY"} · {fmtDay(first.day)} –{" "}
        {fmtDay(last.day)}
      </div>
      <svg
        viewBox={`0 0 ${w} ${h}`}
        width="100%"
        height={h}
        role="img"
        aria-label="activity histogram"
      >
        {cells.map((c, i) => {
          const bh = Math.max(c.count > 0 ? 2 : 0, (c.count / max) * (h - 4));
          return (
            <rect
              key={c.day}
              className="bar"
              x={(i * w) / cells.length}
              y={h - bh}
              width={bw}
              height={bh}
              rx={Math.min(2, bw / 2)}
              fill="var(--accent)"
            >
              <title>
                {fmtDay(c.day)}: {c.count} events
              </title>
            </rect>
          );
        })}
      </svg>
    </div>
  );
}
