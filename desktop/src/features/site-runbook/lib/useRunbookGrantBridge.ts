import * as React from "react";

import {
  mirrorBrowserAgentRunbook,
  takeBrowserAgentRunbookProposes,
} from "@/features/browser-agent/lib/api";
import type { BrowserAgentGrant } from "@/features/browser-agent/lib/types";

import { sidRunbookRef } from "./keys";
import { proposeProcedure } from "./mutations";
import { shapeRunbookInject } from "./serialize";
import {
  getSiteRunbookOrEmpty,
  setSiteRunbook,
  subscribeSiteRunbooks,
} from "./store";

/**
 * While a playground grant is live: mirror runbook for MCP inject, and apply
 * agent proposals as pending procedures (UI Accept required).
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
    const timer = window.setInterval(() => {
      void takeBrowserAgentRunbookProposes(label)
        .then((proposals) => {
          if (!proposals.length) return;
          let runbook = getSiteRunbookOrEmpty(sidRunbookRef(surfaceId));
          for (const proposal of proposals) {
            const title = proposal.title?.trim();
            if (!title) continue;
            const next = proposeProcedure(runbook, {
              title,
              steps: proposal.steps ?? "",
              sourceAgent: proposal.sourceAgent,
              sourceChannel: proposal.sourceChannel,
            });
            runbook = next.runbook;
          }
          setSiteRunbook(sidRunbookRef(surfaceId), runbook);
        })
        .catch(() => {});
    }, 1500);
    return () => window.clearInterval(timer);
  }, [grant, surfaceId, webviewLabel]);
}
