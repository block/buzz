import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { CircleArrowUp, ExternalLink } from "lucide-react";

import { useUpdaterContext } from "./hooks/UpdaterProvider";
import { shouldShowSidebarUpdateCard } from "./sidebarUpdateCardVisibility";
import { SidebarCompactActionCard } from "@/shared/ui/sidebar-action-card";
import { Spinner } from "@/shared/ui/spinner";

type SidebarUpdateCardProps = {
  onDismiss: () => void;
};

type SidebarUpdateCompactCardProps = SidebarUpdateCardProps & {
  actionTestId?: string;
  testId?: string;
};

export function SidebarUpdateCompactCard({
  actionTestId,
  onDismiss,
  testId = "sidebar-update-card-compact",
}: SidebarUpdateCompactCardProps) {
  const { relaunch } = useUpdaterContext();
  const [isRestartPending, setIsRestartPending] = React.useState(false);
  const restartPendingRef = React.useRef(false);
  const restartFrameRef = React.useRef<number | null>(null);
  const restartTimeoutRef = React.useRef<number | null>(null);

  React.useEffect(() => {
    return () => {
      if (restartFrameRef.current !== null) {
        window.cancelAnimationFrame(restartFrameRef.current);
      }
      if (restartTimeoutRef.current !== null) {
        window.clearTimeout(restartTimeoutRef.current);
      }
      restartPendingRef.current = false;
    };
  }, []);

  const handleRestart = React.useCallback(() => {
    if (restartPendingRef.current) {
      return;
    }

    restartPendingRef.current = true;
    setIsRestartPending(true);
    restartFrameRef.current = window.requestAnimationFrame(() => {
      restartFrameRef.current = null;
      restartTimeoutRef.current = window.setTimeout(() => {
        restartTimeoutRef.current = null;
        void relaunch()
          .catch((error) => {
            console.error("[SidebarUpdateCard] relaunch failed:", error);
          })
          .finally(() => {
            restartPendingRef.current = false;
            setIsRestartPending(false);
          });
      }, 0);
    });
  }, [relaunch]);

  return (
    <SidebarCompactActionCard
      actionAriaLabel="Restart now to apply update"
      actionDisabled={isRestartPending}
      actionTestId={actionTestId}
      description={isRestartPending ? "Restarting" : "Click to restart"}
      dismissLabel="Dismiss update notification"
      icon={
        isRestartPending ? (
          <Spinner aria-hidden="true" className="h-5 w-5 border-2" />
        ) : (
          <CircleArrowUp aria-hidden="true" className="h-5 w-5" />
        )
      }
      iconKey={isRestartPending ? "pending" : "idle"}
      onAction={handleRestart}
      onDismiss={onDismiss}
      testId={testId}
      title="Ready to update!"
    />
  );
}

export function SidebarUpdateCard({ onDismiss }: SidebarUpdateCardProps) {
  const { status } = useUpdaterContext();

  if (!shouldShowSidebarUpdateCard(status)) {
    return null;
  }

  if (status.state === "manual-required") {
    return (
      <SidebarCompactActionCard
        actionAriaLabel="Download update from GitHub"
        actionTestId="sidebar-update-download-github"
        description={`v${status.version} available — download from GitHub. Switch to AppImage for automatic updates.`}
        dismissLabel="Dismiss update notification"
        icon={<ExternalLink aria-hidden="true" className="h-5 w-5" />}
        iconKey="manual"
        onAction={() => void openUrl(status.releaseUrl)}
        onDismiss={onDismiss}
        testId="sidebar-update-card-manual"
        title="Update available"
      />
    );
  }

  return (
    <SidebarUpdateCompactCard
      actionTestId="sidebar-update-restart"
      onDismiss={onDismiss}
      testId="sidebar-update-card"
    />
  );
}
