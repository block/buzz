import { useQuery } from "@tanstack/react-query";
import { useState, type ReactNode } from "react";
import { api, fmtTime, shortId } from "../api";

type Props = {
  fold: string;
  version: number;
  onVersion: (v: number) => void;
};

/** Renders one artifact version: Working Context + Log with citation chips,
 * a version scrubber, and a pinned-evidence rail. */
export function ArtifactReader({ fold, version, onVersion }: Props) {
  const [pinned, setPinned] = useState<string[]>([]);

  const chain = useQuery({
    queryKey: ["artifacts", fold],
    queryFn: () => api.artifacts(fold),
  });
  const artifact = useQuery({
    queryKey: ["artifact", fold, version],
    queryFn: () => api.artifact(fold, version),
  });

  const pin = (id: string) =>
    setPinned((p) => (p.includes(id) ? p : [id, ...p]));

  const a = artifact.data;

  return (
    <div>
      <div className="toolbar">
        <strong className="mono">{fold}</strong>
        <div className="version-strip">
          {chain.data?.artifacts.map((s) => (
            <button
              key={s.version}
              className={s.version === version ? "active" : ""}
              onClick={() => onVersion(s.version)}
              title={`${fmtTime(s.created_at)} · ${s.shown} shown${
                s.truncated ? " · chunked" : ""
              }`}
            >
              v{s.version}
            </button>
          ))}
        </div>
      </div>
      {a && (
        <div className="muted" style={{ fontSize: 12, marginBottom: 10 }}>
          {fmtTime(a.created_at)} · model {a.model} · {a.shown_ids.length} shown
          {a.truncated ? " · chunked window" : ""}
          {a.coverage_since !== null && a.coverage_until !== null && (
            <>
              {" "}
              · covers {fmtTime(a.coverage_since)} → {fmtTime(a.coverage_until)}
            </>
          )}
        </div>
      )}
      {artifact.isError && (
        <div className="error-box">{(artifact.error as Error).message}</div>
      )}
      <div className="artifact">
        <div className="artifact-body">
          {a ? renderOutput(a.output, pin) : <span className="faint">loading…</span>}
        </div>
        <div className="evidence">
          <h2>Evidence</h2>
          {pinned.length === 0 && (
            <div className="faint" style={{ fontSize: 12 }}>
              Click a citation chip to pin the source message here — every
              claim is one click from its evidence.
            </div>
          )}
          {pinned.map((id) => (
            <EvidenceCard
              key={id}
              id={id}
              onClose={() => setPinned((p) => p.filter((x) => x !== id))}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

function EvidenceCard({ id, onClose }: { id: string; onClose: () => void }) {
  const ev = useQuery({
    queryKey: ["event", id],
    queryFn: () => api.event(id),
    retry: false,
  });
  return (
    <div className="evidence-card">
      <div className="head">
        <span>
          {ev.data ? (
            <>
              <strong>{ev.data.author_name ?? shortId(ev.data.pubkey)}</strong>{" "}
              · {fmtTime(ev.data.created_at)}
            </>
          ) : (
            <span className="mono">{shortId(id)}…</span>
          )}
        </span>
        <button onClick={onClose} style={{ padding: "0 6px" }}>
          ×
        </button>
      </div>
      <div className="body">
        {ev.isError
          ? `not in the mirror: ${(ev.error as Error).message}`
          : (ev.data?.content ?? "loading…")}
      </div>
    </div>
  );
}

const CITATION = /\[event:([0-9a-f]{64})\]/g;

/** Minimal renderer for the artifact's markdown-shaped output: headings,
 * list items, paragraphs — with `[event:<id>]` turned into citation chips. */
function renderOutput(output: string, pin: (id: string) => void): ReactNode {
  const blocks: ReactNode[] = [];
  let list: ReactNode[] = [];
  let key = 0;

  const flushList = () => {
    if (list.length > 0) {
      blocks.push(<ul key={key++}>{list}</ul>);
      list = [];
    }
  };

  for (const line of output.split("\n")) {
    const trimmed = line.trim();
    if (trimmed.startsWith("- ") || trimmed.startsWith("* ")) {
      list.push(<li key={key++}>{inline(trimmed.slice(2), pin)}</li>);
      continue;
    }
    flushList();
    if (trimmed.startsWith("## ")) {
      blocks.push(<h4 key={key++}>{inline(trimmed.slice(3), pin)}</h4>);
    } else if (trimmed.startsWith("# ")) {
      blocks.push(<h3 key={key++}>{inline(trimmed.slice(2), pin)}</h3>);
    } else if (trimmed.length > 0) {
      blocks.push(<p key={key++}>{inline(trimmed, pin)}</p>);
    }
  }
  flushList();
  return blocks;
}

function inline(text: string, pin: (id: string) => void): ReactNode[] {
  const out: ReactNode[] = [];
  let last = 0;
  let key = 0;
  CITATION.lastIndex = 0;
  let m = CITATION.exec(text);
  while (m !== null) {
    if (m.index > last) out.push(text.slice(last, m.index));
    const id = m[1];
    if (id) {
      out.push(
        <button
          key={key++}
          className="chip"
          title={id}
          onClick={() => pin(id)}
        >
          {shortId(id)}
        </button>,
      );
    }
    last = m.index + m[0].length;
    m = CITATION.exec(text);
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}
