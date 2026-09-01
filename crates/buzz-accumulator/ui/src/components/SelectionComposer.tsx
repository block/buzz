import { useQuery } from "@tanstack/react-query";
import { api, type Selection, type TimeWindow } from "../api";

const DAY = 86_400;

const PRESETS: { label: string; days: number | null }[] = [
  { label: "7d", days: 7 },
  { label: "30d", days: 30 },
  { label: "90d", days: 90 },
  { label: "all", days: null },
];

type Props = {
  selection: Selection;
  onChange: (s: Selection) => void;
  window: TimeWindow;
  onWindowChange: (w: TimeWindow) => void;
};

export function SelectionComposer({
  selection,
  onChange,
  window,
  onWindowChange,
}: Props) {
  const channels = useQuery({ queryKey: ["channels"], queryFn: api.channels });

  const toggleChannel = (id: string) => {
    const has = selection.channels.includes(id);
    onChange({
      ...selection,
      channels: has
        ? selection.channels.filter((c) => c !== id)
        : [...selection.channels, id],
    });
  };

  const activePreset = (days: number | null) => {
    if (days === null) return window.since === undefined;
    if (window.since === undefined) return false;
    const target = Math.floor(Date.now() / 1000) - days * DAY;
    // A preset stays highlighted for the minute it was clicked in.
    return Math.abs(window.since - target) < 60;
  };

  const list = channels.data?.channels.filter((c) => c.active) ?? [];
  const sorted = [...list].sort((a, b) =>
    (a.name ?? a.id).localeCompare(b.name ?? b.id),
  );

  return (
    <div>
      <h2>Selection</h2>

      <div className="field">
        <label>
          Channels{" "}
          <span className="faint">
            ({selection.channels.length || "none"} selected)
          </span>
        </label>
        {channels.isLoading ? (
          <div className="faint">loading…</div>
        ) : (
          <div className="channel-list">
            {sorted.map((c) => (
              <label key={c.id}>
                <input
                  type="checkbox"
                  checked={selection.channels.includes(c.id)}
                  onChange={() => toggleChannel(c.id)}
                />
                <span>{c.name ?? `${c.id.slice(0, 8)}…`}</span>
                {!c.backfill_done && <span className="faint">(syncing)</span>}
              </label>
            ))}
          </div>
        )}
        <div style={{ marginTop: 4 }}>
          <button
            onClick={() => onChange({ ...selection, channels: [] })}
            disabled={selection.channels.length === 0}
          >
            clear
          </button>
        </div>
      </div>

      <div className="field">
        <label>Authors (64-hex pubkeys, one per line)</label>
        <textarea
          rows={2}
          className="mono"
          placeholder="optional — person-across-channels"
          value={selection.authors.join("\n")}
          onChange={(e) =>
            onChange({
              ...selection,
              authors: e.target.value
                .split(/[\s,]+/)
                .map((a) => a.trim())
                .filter(Boolean),
            })
          }
        />
      </div>

      <div className="field">
        <label>Kinds (comma-separated; empty = 9, chat)</label>
        <input
          className="mono"
          placeholder="9"
          value={selection.kinds.join(",")}
          onChange={(e) =>
            onChange({
              ...selection,
              kinds: e.target.value
                .split(",")
                .map((k) => Number.parseInt(k.trim(), 10))
                .filter((k) => Number.isFinite(k) && k >= 0),
            })
          }
        />
      </div>

      <h2>Window</h2>
      <div className="field">
        <div className="preset-row">
          {PRESETS.map((p) => (
            <button
              key={p.label}
              className={activePreset(p.days) ? "active" : ""}
              onClick={() =>
                onWindowChange(
                  p.days === null
                    ? {}
                    : {
                        since:
                          Math.floor(Date.now() / 1000) - (p.days ?? 0) * DAY,
                      },
                )
              }
            >
              {p.label}
            </button>
          ))}
        </div>
      </div>
      <div className="field">
        <label>Since</label>
        <input
          type="datetime-local"
          value={toLocalInput(window.since)}
          onChange={(e) =>
            onWindowChange({ ...window, since: fromLocalInput(e.target.value) })
          }
        />
      </div>
      <div className="field">
        <label>Until (exclusive)</label>
        <input
          type="datetime-local"
          value={toLocalInput(window.until_exclusive)}
          onChange={(e) =>
            onWindowChange({
              ...window,
              until_exclusive: fromLocalInput(e.target.value),
            })
          }
        />
      </div>
    </div>
  );
}

function toLocalInput(ts: number | undefined): string {
  if (ts === undefined) return "";
  const d = new Date(ts * 1000);
  const pad = (n: number) => `${n}`.padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(
    d.getHours(),
  )}:${pad(d.getMinutes())}`;
}

function fromLocalInput(v: string): number | undefined {
  if (!v) return undefined;
  const ms = new Date(v).getTime();
  return Number.isFinite(ms) ? Math.floor(ms / 1000) : undefined;
}
