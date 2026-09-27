import * as React from "react";

import { listen } from "@tauri-apps/api/event";

import {
  mirrorBrowserAgentRunbook,
  takeBrowserAgentRunbookProposes,
} from "@/features/browser-agent/lib/api";
import type { BrowserAgentGrant } from "@/features/browser-agent/lib/types";

import { sidRunbookRef } from "./keys";
import { upsertAgentProcedure } from "./mutations";
import { shapeRunbookInject } from "./serialize";
import {
  getSiteRunbookOrEmpty,
  setSiteRunbook,
  subscribeSiteRunbooks,
} from "./store";

/**
 * While a playground grant is live: mirror runbook for MCP inject, and apply
 * agent proposals as auto-activated procedures (unless title/id is persisted).
 */
export function useRunbookGrantBridge(
  grant: BrowserAgentGrant | null,
  webviewLabel: string,
): void {
  const surfaceId = grant?.surfaceId;

  const pushMirror = React.useCallback(async () => {
    if (!grant || grant.surface !== "playground" || !surfaceId) return;
    const runbook = getSiteRunbookOrEmpty(sidRunbookRef(surfaceId));
    const inject = shapeRunbookInject(runbook);
    try {
      await mirrorBrowserAgentRunbook({
        webviewLabel: grant.webviewLabel || webviewLabel,
        inject,
        full: {
          agentBrief: runbook.agentBrief,
          procedures: runbook.procedures.map((procedure) => ({
            id: procedure.id,
            title: procedure.title,
            steps: procedure.steps,
            status: procedure.status,
            persisted: procedure.persisted === true,
            sourceAgent: procedure.sourceAgent,
            sourceChannel: procedure.sourceChannel,
            createdAt: procedure.createdAt,
            updatedAt: procedure.updatedAt,
            acceptedAt: procedure.acceptedAt,
          })),
          updatedAt: runbook.updatedAt,
        },
      });
    } catch {
      // Best-effort; MCP can still operate without a runbook file.
    }
  }, [grant, surfaceId, webviewLabel]);

  React.useEffect(() => {
    void pushMirror();
  }, [pushMirror]);

  React.useEffect(() => {
    if (!grant || !surfaceId) return;
    return subscribeSiteRunbooks(() => {
      void pushMirror();
    });
  }, [grant, pushMirror, surfaceId]);

  React.useEffect(() => {
    if (!grant || grant.surface !== "playground" || !surfaceId) return;
    const label = grant.webviewLabel || webviewLabel;
    let cancelled = false;

    const drain = () => {
      void takeBrowserAgentRunbookProposes(label)
        .then((proposals) => {
          if (cancelled || !proposals.length) return;
          let runbook = getSiteRunbookOrEmpty(sidRunbookRef(surfaceId));
          for (const proposal of proposals) {
            const title = proposal.title?.trim();
            if (!title) continue;
            const steps = (proposal.steps ?? "").trim();
            if (!steps) continue;
            try {
              const next = upsertAgentProcedure(runbook, {
                title,
                steps,
                sourceAgent: proposal.sourceAgent,
                sourceChannel: proposal.sourceChannel,
              });
              runbook = next.runbook;
            } catch {
              // Persisted lock — skip this proposal.
            }
          }
          setSiteRunbook(sidRunbookRef(surfaceId), runbook);
        })
        .catch(() => {});
    };

    // Backup poll (wake event is primary).
    const timer = window.setInterval(drain, 750);
    drain();

    let unlisten: (() => void) | undefined;
    void listen<{ webviewLabel?: string; surfaceId?: string }>(
      "browser-agent-runbook-propose",
      (event) => {
        const payload = event.payload;
        if (
          payload?.webviewLabel &&
          payload.webviewLabel !== label &&
          payload.surfaceId !== surfaceId
        ) {
          return;
        }
        drain();
      },
    ).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });

    return () => {
      cancelled = true;
      window.clearInterval(timer);
      unlisten?.();
    };
  }, [grant, surfaceId, webviewLabel]);
}
