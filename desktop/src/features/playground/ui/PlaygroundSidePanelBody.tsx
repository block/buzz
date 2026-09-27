import * as React from "react";

import type { PlaygroundConversation } from "../lib/conversation";
import {
  PLAYGROUND_HULA,
  PLAYGROUND_VERSION,
} from "@/features/playground/lib/types";
import {
  openPopoutWindow,
  popoutErrorMessage,
} from "@/features/popout/lib/popoutWindow";
import { toast } from "sonner";
import { isMainBrowserTab } from "../lib/browserGroups";
import {
  getPlaygroundViewport,
  setPlaygroundViewport,
  type PlaygroundChromeMode,
} from "../lib/playgroundViewport";
import { PLAYGROUND_SIDE_PANEL_ROOT_CLASS } from "../lib/overlayLayout";
import { PlaygroundChrome } from "./PlaygroundChrome";
import { PlaygroundStage } from "./PlaygroundStage";
import { PlaygroundTabStrip } from "./PlaygroundTabStrip";
import {
  closePlaygroundTab,
  getBrowserForSid,
  getPlaygroundStore,
  subscribePlayground,
  switchPlaygroundTab,
  type PlaygroundSession,
} from "../lib/sessions";
import { rebindBrowserAgentGrantToTab } from "@/features/browser-agent/lib/api";
import { syncAndAnnounceBrowserTabs } from "../lib/runtime";
import { browserWebviewLabel } from "@/features/browser-agent/lib/labels";
import { currentWindowLabel } from "../lib/webview";

/**
 * Playground chrome + stage hosted inside the channel RHS idle-auxiliary
 * panel (same slide-out shell as openLinkSidePanel). Keeps Agent Observe/Drive
 * on the playground-{sid} webview — not a grant-less pin webview.
 */
export function PlaygroundSidePanelBody({
  conversation,
  session,
}: {
  conversation: PlaygroundConversation | null;
  session: PlaygroundSession;
}) {
  const [mode, setMode] = React.useState<PlaygroundChromeMode>(
    () => getPlaygroundViewport(session.sid).mode,
  );
  const [layoutEpoch, setLayoutEpoch] = React.useState(0);
  const playgroundStore = React.useSyncExternalStore(
    subscribePlayground,
    getPlaygroundStore,
    getPlaygroundStore,
  );
  const browser = React.useMemo(
    () => getBrowserForSid(session.sid),
    [playgroundStore.browsers, session.sid],
  );

  async function rebindGrantOnTabSwitch(fromSid: string, toSid: string) {
    if (fromSid === toSid) return;
    const windowLabel = currentWindowLabel();
    try {
      await rebindBrowserAgentGrantToTab({
        fromSurfaceId: fromSid,
        toSurfaceId: toSid,
        toWebviewLabel: browserWebviewLabel({
          surface: "playground",
          surfaceId: toSid,
          windowLabel,
        }),
        fromWebviewLabel: browserWebviewLabel({
          surface: "playground",
          surfaceId: fromSid,
          windowLabel,
        }),
      });
    } catch {
      // Best-effort.
    }
  }

  function handleSelectTab(sid: string) {
    const fromSid = session.sid;
    switchPlaygroundTab(sid);
    void rebindGrantOnTabSwitch(fromSid, sid);
    const group = getBrowserForSid(sid);
    if (group) {
      void syncAndAnnounceBrowserTabs(group.browserId, {
        kind: "tab_switched",
        surfaceId: sid,
      }).catch(() => undefined);
    }
  }

  function handleCloseTab(sid: string) {
    const group = getBrowserForSid(sid);
    const browserId = group?.browserId;
    closePlaygroundTab(sid);
    if (browserId) {
      void syncAndAnnounceBrowserTabs(browserId).catch(() => undefined);
    }
  }

  const handleModeChange = React.useCallback(
    (next: PlaygroundChromeMode) => {
      setMode(next);
      setPlaygroundViewport(session.sid, { mode: next });
    },
    [session.sid],
  );

  const bumpStageLayout = React.useCallback(() => {
    setLayoutEpoch((value) => value + 1);
  }, []);

  async function handleDetach() {
    try {
      const threadId = conversation?.draftKey.startsWith("thread:")
        ? conversation.draftKey.slice("thread:".length)
        : undefined;
      await openPopoutWindow({
        kind: "playground",
        title: session.name,
        seed: session.sid,
        ...(conversation?.channelId
          ? { channelId: conversation.channelId }
          : {}),
        ...(threadId ? { threadId } : {}),
        playground: {
          hula: PLAYGROUND_HULA,
          v: PLAYGROUND_VERSION,
          name: session.name,
          url: session.url,
          sid: session.sid,
          ...(session.pin ? { pin: session.pin } : {}),
          ...(session.stack ? { stack: session.stack } : {}),
          ...(session.expires != null ? { expires: session.expires } : {}),
        },
      });
    } catch (error) {
      toast.error(popoutErrorMessage(error, "Could not detach playground."));
    }
  }

  // IdleAuxiliaryPanel owns title + close; hide dock/dismiss and Detach stays.
  return (
    <div
      className={PLAYGROUND_SIDE_PANEL_ROOT_CLASS}
      data-playground-host="side-panel"
      data-testid="playground-side-panel"
    >
      <PlaygroundChrome
        conversation={conversation}
        docked={false}
        fullscreen={false}
        hideDismiss
        hideDock
        mode={mode}
        onModeChange={handleModeChange}
        onStageResync={bumpStageLayout}
        onToggleDock={() => {}}
        onToggleFullscreen={() => {}}
        session={session}
        onDetach={() => void handleDetach()}
        showDetach
        showFullscreen={false}
        showInspect={false}
        tabs={
          browser ? (
            <PlaygroundTabStrip
              browser={browser}
              onClose={handleCloseTab}
              onSelect={handleSelectTab}
              sessions={playgroundStore.sessions}
            />
          ) : null
        }
        urlBarReadOnly={
          browser != null && !isMainBrowserTab(browser, session.sid)
        }
      />
      <PlaygroundStage
        layoutKey={`side-panel:${layoutEpoch}`}
        mode={mode}
        session={session}
      />
    </div>
  );
}
