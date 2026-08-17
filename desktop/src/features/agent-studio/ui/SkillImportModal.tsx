import * as React from "react";
import { Download } from "lucide-react";

import { useCommunities } from "@/features/communities/useCommunities";
import { publishSkillImport } from "@/shared/api/tauriHiveStudio";
import { Button } from "@/shared/ui/button";

type SkillImportModalProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
};

export function SkillImportModal({
  open,
  onOpenChange,
}: SkillImportModalProps) {
  const { activeCommunity } = useCommunities();
  const [repo, setRepo] = React.useState("");
  const [skillId, setSkillId] = React.useState("");
  const [message, setMessage] = React.useState<string | null>(null);
  const [loading, setLoading] = React.useState(false);

  React.useEffect(() => {
    if (!open) {
      setMessage(null);
    }
  }, [open]);

  if (!open) {
    return null;
  }

  const relayHttp = activeCommunity?.relayUrl?.replace(/^ws/i, "http");

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      data-testid="skill-import-modal"
    >
      <div className="w-full max-w-md rounded-lg border border-border bg-card p-4 shadow-lg">
        <h2 className="text-base font-medium">Import skill from GitHub</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Imports a skill from GitHub and publishes kind 47250 to the relay.
        </p>
        <label className="mt-4 block text-sm">
          Repository URL
          <input
            className="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm"
            onChange={(e) => setRepo(e.target.value)}
            placeholder="https://github.com/owner/repo"
            value={repo}
          />
        </label>
        <label className="mt-3 block text-sm">
          Skill ID
          <input
            className="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm"
            onChange={(e) => setSkillId(e.target.value)}
            placeholder="lint-rust"
            value={skillId}
          />
        </label>
        {message ? (
          <p className="mt-3 text-sm text-muted-foreground">{message}</p>
        ) : null}
        <div className="mt-4 flex justify-end gap-2">
          <Button
            onClick={() => onOpenChange(false)}
            type="button"
            variant="ghost"
          >
            Cancel
          </Button>
          <Button
            disabled={loading || !relayHttp || !repo || !skillId}
            onClick={() => {
              if (!relayHttp) return;
              setLoading(true);
              void fetch(`${relayHttp}/agent-studio/skills/import`, {
                body: JSON.stringify({ repo, skill_id: skillId }),
                headers: { "Content-Type": "application/json" },
                method: "POST",
              })
                .then((res) => res.json())
                .then(
                  (data: {
                    message?: string;
                    accepted?: boolean;
                    event_payload?: {
                      source_repo?: string | null;
                      source_commit?: string | null;
                    };
                  }) => {
                    if (!data.accepted) {
                      setMessage(data.message ?? "Import planning failed");
                      return;
                    }
                    const sourceRepo = data.event_payload?.source_repo ?? null;
                    const sourceCommit =
                      data.event_payload?.source_commit ?? null;
                    return publishSkillImport(
                      skillId,
                      sourceRepo,
                      sourceCommit,
                    ).then((published) => {
                      setMessage(
                        published.message ?? data.message ?? "Skill imported",
                      );
                      onOpenChange(false);
                    });
                  },
                )
                .catch((e: unknown) => {
                  setMessage(e instanceof Error ? e.message : "Import failed");
                })
                .finally(() => setLoading(false));
            }}
            type="button"
          >
            <Download className="mr-2 h-4 w-4" />
            Import
          </Button>
        </div>
      </div>
    </div>
  );
}
