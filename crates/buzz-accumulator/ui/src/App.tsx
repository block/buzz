import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api, type Selection, type TimeWindow } from "./api";
import { ArtifactReader } from "./components/ArtifactReader";
import { Explore } from "./components/Explore";
import { FoldsBoard } from "./components/FoldsBoard";
import { SelectionComposer } from "./components/SelectionComposer";

export type View =
  | { kind: "explore" }
  | { kind: "artifact"; fold: string; version: number };

/** Initial selection/window from URL query params, so a selection is
 * shareable: `?channels=<uuid,uuid>&authors=<hex>&kinds=9&since=<ts>`. */
function fromUrl(): { selection: Selection; window: TimeWindow } {
  const q = new URLSearchParams(location.search);
  const list = (key: string) =>
    (q.get(key) ?? "").split(",").map((s) => s.trim()).filter(Boolean);
  const num = (key: string) => {
    const n = Number.parseInt(q.get(key) ?? "", 10);
    return Number.isFinite(n) ? n : undefined;
  };
  return {
    selection: {
      channels: list("channels"),
      authors: list("authors"),
      threads: list("threads"),
    tags: [],
      kinds: list("kinds")
        .map((k) => Number.parseInt(k, 10))
        .filter((k) => Number.isFinite(k) && k >= 0),
    },
    window: { since: num("since"), until_exclusive: num("until") },
  };
}

/** Initial view from `?fold=<name>&v=<version>`, so artifacts are shareable. */
function viewFromUrl(): View {
  const q = new URLSearchParams(location.search);
  const fold = q.get("fold");
  const v = Number.parseInt(q.get("v") ?? "1", 10);
  return fold && Number.isFinite(v) && v >= 1
    ? { kind: "artifact", fold, version: v }
    : { kind: "explore" };
}

export function App() {
  const initial = fromUrl();
  const [selection, setSelection] = useState<Selection>(initial.selection);
  const [window, setWindow] = useState<TimeWindow>(initial.window);
  const [view, setView] = useState<View>(viewFromUrl);

  const status = useQuery({
    queryKey: ["status"],
    queryFn: api.status,
    refetchInterval: 5000,
  });

  const s = status.data;
  const dotClass = status.isError
    ? "error"
    : s?.connection === "connected"
      ? "connected"
      : "";

  return (
    <>
      <header className="app-header">
        <h1>🐝 accumulator</h1>
        <span className="status-pill">
          <span className={`status-dot ${dotClass}`} />
          {status.isError
            ? "daemon unreachable — is it running on :4640?"
            : s
              ? `${s.connection} · ${s.total_events.toLocaleString()} events · ${
                  s.folds
                } folds · ${s.artifacts} artifacts${
                  s.backfill_complete ? "" : " · backfilling…"
                }`
              : "connecting…"}
        </span>
        {view.kind === "artifact" && (
          <button onClick={() => setView({ kind: "explore" })}>
            ← back to explore
          </button>
        )}
      </header>
      <div className="columns">
        <aside className="pane left">
          <SelectionComposer
            selection={selection}
            onChange={setSelection}
            window={window}
            onWindowChange={setWindow}
          />
        </aside>
        <main className="pane">
          {view.kind === "explore" ? (
            <Explore selection={selection} window={window} />
          ) : (
            <ArtifactReader
              fold={view.fold}
              version={view.version}
              onVersion={(v) =>
                setView({ kind: "artifact", fold: view.fold, version: v })
              }
            />
          )}
        </main>
        <aside className="pane right">
          <FoldsBoard
            selection={selection}
            onOpenArtifact={(fold, version) =>
              setView({ kind: "artifact", fold, version })
            }
          />
        </aside>
      </div>
    </>
  );
}
