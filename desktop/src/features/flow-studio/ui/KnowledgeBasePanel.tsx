import * as React from "react";
import { useQuery } from "@tanstack/react-query";

import { useCommunities } from "@/features/communities/useCommunities";
import { publishKbDocument } from "@/shared/api/tauriHiveStudio";
import { Button } from "@/shared/ui/button";

const DEFAULT_KB_ID = "default";

export function KnowledgeBasePanel() {
  const { activeCommunity } = useCommunities();
  const [filename, setFilename] = React.useState("notes.txt");
  const [content, setContent] = React.useState("");
  const [query, setQuery] = React.useState("");
  const [message, setMessage] = React.useState<string | null>(null);

  const relayHttp = activeCommunity?.relayUrl?.replace(/^ws/i, "http");

  const searchQuery = useQuery({
    enabled: Boolean(relayHttp && query.trim()),
    queryFn: async () => {
      const res = await fetch(
        `${relayHttp}/flow-studio/knowledge/search?knowledge_base_id=${encodeURIComponent(DEFAULT_KB_ID)}&q=${encodeURIComponent(query)}&mode=semantic`,
      );
      if (!res.ok) {
        throw new Error(`HTTP ${res.status}`);
      }
      return (await res.json()) as {
        hits?: Array<{ document_id: string; content: string }>;
      };
    },
    queryKey: ["flow-kb-search", relayHttp, query],
  });

  const ingest = () => {
    const documentId = `doc-${Date.now()}`;
    void publishKbDocument({
      knowledgeBaseId: DEFAULT_KB_ID,
      documentId,
      filename,
      mimeType: "text/plain",
      content,
    })
      .then((result) => setMessage(result.message))
      .catch((error: unknown) => {
        setMessage(error instanceof Error ? error.message : "Ingest failed");
      });
  };

  return (
    <section className="mt-6 rounded-lg border border-border bg-card p-4">
      <h2 className="text-sm font-medium">Knowledge base</h2>
      <p className="mt-1 text-sm text-muted-foreground">
        Ingest documents as kind 46250; semantic search via pgvector (hash
        embedding MVP).
      </p>
      <label className="mt-3 block text-sm">
        Filename
        <input
          className="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm"
          onChange={(event) => setFilename(event.target.value)}
          value={filename}
        />
      </label>
      <label className="mt-3 block text-sm">
        Content
        <textarea
          className="mt-1 min-h-24 w-full rounded-md border border-border bg-background px-3 py-2 text-sm"
          onChange={(event) => setContent(event.target.value)}
          value={content}
        />
      </label>
      <div className="mt-3 flex gap-2">
        <Button
          disabled={!content.trim()}
          onClick={ingest}
          size="sm"
          type="button"
        >
          Ingest document
        </Button>
      </div>
      <div className="mt-4 flex gap-2">
        <input
          className="flex-1 rounded-md border border-border bg-background px-3 py-2 text-sm"
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Search knowledge…"
          value={query}
        />
        <Button
          disabled={!query.trim()}
          onClick={() => void searchQuery.refetch()}
          size="sm"
          type="button"
          variant="outline"
        >
          Search
        </Button>
      </div>
      {message ? (
        <p className="mt-2 text-sm text-muted-foreground">{message}</p>
      ) : null}
      {searchQuery.data?.hits?.length ? (
        <ul className="mt-3 space-y-2 text-sm">
          {searchQuery.data.hits.map((hit) => (
            <li
              className="rounded-md border border-border bg-background p-2"
              key={`${hit.document_id}-${hit.content.slice(0, 16)}`}
            >
              <span className="text-2xs text-muted-foreground">
                {hit.document_id}
              </span>
              <p className="mt-1 whitespace-pre-wrap">{hit.content}</p>
            </li>
          ))}
        </ul>
      ) : null}
    </section>
  );
}
