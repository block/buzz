import * as React from "react";
import { useQuery } from "@tanstack/react-query";

import { useCommunities } from "@/features/communities/useCommunities";
import { publishFlowFile } from "@/shared/api/tauriHiveStudio";
import { Button } from "@/shared/ui/button";

export function FilesPanel() {
  const { activeCommunity } = useCommunities();
  const [filename, setFilename] = React.useState("");
  const [mediaUrl, setMediaUrl] = React.useState("");
  const [message, setMessage] = React.useState<string | null>(null);

  const relayHttp = activeCommunity?.relayUrl?.replace(/^ws/i, "http");

  const filesQuery = useQuery({
    enabled: Boolean(relayHttp),
    queryFn: async () => {
      const res = await fetch(`${relayHttp}/flow-studio/files`);
      if (!res.ok) {
        throw new Error(`HTTP ${res.status}`);
      }
      return (await res.json()) as {
        files?: Array<{
          file_id: string;
          filename: string;
          media_url?: string | null;
          version: number;
        }>;
      };
    },
    queryKey: ["flow-files", relayHttp],
    refetchInterval: 5000,
  });

  const registerFile = () => {
    if (!filename.trim()) return;
    const fileId = `file-${Date.now()}`;
    void publishFlowFile(
      fileId,
      filename,
      mediaUrl.trim() ? mediaUrl.trim() : null,
    )
      .then((result) => {
        setMessage(result.message);
        setFilename("");
        setMediaUrl("");
        void filesQuery.refetch();
      })
      .catch((error: unknown) => {
        setMessage(error instanceof Error ? error.message : "Register failed");
      });
  };

  return (
    <section className="mt-6 rounded-lg border border-border bg-card p-4">
      <h2 className="text-sm font-medium">Files</h2>
      <p className="mt-1 text-sm text-muted-foreground">
        Metadata via kind 46350; upload bytes through Buzz media, then paste the
        media URL here.
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
        Media URL (optional)
        <input
          className="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm"
          onChange={(event) => setMediaUrl(event.target.value)}
          placeholder="https://relay/media/…"
          value={mediaUrl}
        />
      </label>
      <Button
        className="mt-3"
        disabled={!filename.trim()}
        onClick={registerFile}
        size="sm"
        type="button"
      >
        Register file
      </Button>
      {message ? (
        <p className="mt-2 text-sm text-muted-foreground">{message}</p>
      ) : null}
      <ul className="mt-4 space-y-2 text-sm">
        {(filesQuery.data?.files ?? []).map((file) => (
          <li
            className="rounded-md border border-border bg-background p-2"
            key={file.file_id}
          >
            <div className="font-medium">{file.filename}</div>
            <div className="text-2xs text-muted-foreground">
              v{file.version} · {file.file_id}
            </div>
            {file.media_url ? (
              <a
                className="mt-1 block truncate text-xs text-primary"
                href={file.media_url}
                rel="noreferrer"
                target="_blank"
              >
                {file.media_url}
              </a>
            ) : null}
          </li>
        ))}
      </ul>
    </section>
  );
}
