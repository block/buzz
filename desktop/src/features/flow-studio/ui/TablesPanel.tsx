import * as React from "react";
import { useQuery } from "@tanstack/react-query";

import { useCommunities } from "@/features/communities/useCommunities";
import { publishTableRow, deleteTableRow } from "@/shared/api/tauriHiveStudio";
import { Button } from "@/shared/ui/button";

const DEFAULT_TABLE_ID = "customers";

export function TablesPanel() {
  const { activeCommunity } = useCommunities();
  const [tableId, setTableId] = React.useState(DEFAULT_TABLE_ID);
  const [rowId, setRowId] = React.useState("");
  const [rowJson, setRowJson] = React.useState('{"name":"Acme"}');
  const [message, setMessage] = React.useState<string | null>(null);

  const relayHttp = activeCommunity?.relayUrl?.replace(/^ws/i, "http");

  const rowsQuery = useQuery({
    enabled: Boolean(relayHttp && tableId),
    queryFn: async () => {
      const res = await fetch(
        `${relayHttp}/flow-studio/tables/${encodeURIComponent(tableId)}/rows`,
      );
      if (!res.ok) {
        throw new Error(`HTTP ${res.status}`);
      }
      return (await res.json()) as {
        rows?: Array<{ row_id: string; row_json: Record<string, unknown> }>;
      };
    },
    queryKey: ["flow-table-rows", relayHttp, tableId],
    refetchInterval: 5000,
  });

  const saveRow = () => {
    const id = rowId.trim() || `row-${Date.now()}`;
    void publishTableRow(tableId, id, rowJson)
      .then((result) => {
        setMessage(result.message);
        setRowId(id);
        void rowsQuery.refetch();
      })
      .catch((error: unknown) => {
        setMessage(error instanceof Error ? error.message : "Save failed");
      });
  };

  return (
    <section className="mt-6 rounded-lg border border-border bg-card p-4">
      <h2 className="text-sm font-medium">Tables</h2>
      <p className="mt-1 text-sm text-muted-foreground">
        CRUD via kind 46300, projected to Postgres per community.
      </p>
      <label className="mt-3 block text-sm">
        Table ID
        <input
          className="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm"
          onChange={(event) => setTableId(event.target.value)}
          value={tableId}
        />
      </label>
      <label className="mt-3 block text-sm">
        Row ID (optional)
        <input
          className="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm"
          onChange={(event) => setRowId(event.target.value)}
          placeholder="auto-generated if empty"
          value={rowId}
        />
      </label>
      <label className="mt-3 block text-sm">
        Row JSON
        <textarea
          className="mt-1 min-h-20 w-full rounded-md border border-border bg-background px-3 py-2 font-mono text-sm"
          onChange={(event) => setRowJson(event.target.value)}
          value={rowJson}
        />
      </label>
      <Button className="mt-3" onClick={saveRow} size="sm" type="button">
        Save row
      </Button>
      {message ? (
        <p className="mt-2 text-sm text-muted-foreground">{message}</p>
      ) : null}
      <ul className="mt-4 space-y-2 text-sm">
        {(rowsQuery.data?.rows ?? []).map((row) => (
          <li
            className="rounded-md border border-border bg-background p-2 font-mono text-xs"
            key={row.row_id}
          >
            <div className="flex items-center justify-between gap-2">
              <span className="text-muted-foreground">{row.row_id}</span>
              <Button
                onClick={() => {
                  void deleteTableRow(tableId, row.row_id)
                    .then((result) => {
                      setMessage(result.message);
                      void rowsQuery.refetch();
                    })
                    .catch((error: unknown) => {
                      setMessage(
                        error instanceof Error
                          ? error.message
                          : "Delete failed",
                      );
                    });
                }}
                size="sm"
                type="button"
                variant="ghost"
              >
                Delete
              </Button>
            </div>
            <pre className="mt-1 whitespace-pre-wrap">
              {JSON.stringify(row.row_json, null, 2)}
            </pre>
          </li>
        ))}
      </ul>
    </section>
  );
}
