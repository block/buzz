import * as React from "react";

import type { SearchHit } from "@/shared/api/searchTypes";
import { cn } from "@/shared/lib/cn";
import { useOptionalSidebar } from "@/shared/ui/sidebar";
import { ProjectContextRail } from "./ProjectContextRail";
import { ProjectConversationPanel } from "./ProjectConversationPanel";
import { PROJECT_COLUMN_HEADER_BACKDROP_CLASS } from "./projectPanelStyles";

type ProjectConversationPanelContextValue = {
  openConversation: (hit: SearchHit) => void;
};

const ProjectConversationPanelContext =
  React.createContext<ProjectConversationPanelContextValue | null>(null);

/** Makes project-related channel conversations available as an auxiliary panel. */
export function ProjectConversationPanelProvider({
  children,
  onOpenConversation,
}: {
  children: React.ReactNode;
  onOpenConversation: (hit: SearchHit) => void;
}) {
  const value = React.useMemo(
    () => ({ openConversation: onOpenConversation }),
    [onOpenConversation],
  );
  return (
    <ProjectConversationPanelContext.Provider value={value}>
      {children}
    </ProjectConversationPanelContext.Provider>
  );
}

/** Owns project conversation selection and renders the shared auxiliary pane. */
export function ProjectConversationPanelController({
  canResetWidth,
  children,
  closeWhen,
  detachFallbackPanel = false,
  fallbackPanel,
  fallbackPanelOpen = false,
  fallbackPanelResizing = false,
  fallbackPanelWidthPx,
  onOpenConversation,
  onResetWidth,
  onResizeStart,
  resetKey,
  sharedHeaderBackdrop,
  widthPx,
}: {
  canResetWidth: boolean;
  children: React.ReactNode;
  closeWhen: boolean;
  detachFallbackPanel?: boolean;
  fallbackPanel?: React.ReactNode;
  fallbackPanelOpen?: boolean;
  fallbackPanelResizing?: boolean;
  fallbackPanelWidthPx: number;
  onOpenConversation: () => void;
  onResetWidth: () => void;
  onResizeStart: (event: React.PointerEvent<HTMLButtonElement>) => void;
  resetKey: string;
  sharedHeaderBackdrop?: boolean;
  widthPx: number;
}) {
  const sidebar = useOptionalSidebar();
  const [hit, setHit] = React.useState<SearchHit | null>(null);
  const previousResetKeyRef = React.useRef(resetKey);
  React.useEffect(() => {
    const changedContext = previousResetKeyRef.current !== resetKey;
    previousResetKeyRef.current = resetKey;
    if (closeWhen || changedContext) setHit(null);
  }, [closeWhen, resetKey]);
  const openConversation = React.useCallback(
    (nextHit: SearchHit) => {
      onOpenConversation();
      setHit(nextHit);
    },
    [onOpenConversation],
  );
  const detached =
    detachFallbackPanel && hit === null && fallbackPanel !== undefined;
  const fallbackVisible =
    fallbackPanelOpen && hit === null && fallbackPanel !== undefined;
  return (
    <ProjectConversationPanelProvider onOpenConversation={openConversation}>
      <div
        className={cn(
          "relative flex min-h-0 min-w-0 flex-1 flex-row overflow-hidden",
          detached && "bg-sidebar pb-2 pr-2 pt-px",
          detached && sidebar?.open === false && "pl-2",
        )}
        data-detached={detached ? "true" : "false"}
        data-project-context-detached={detached ? "true" : undefined}
        data-project-detail-screen
        data-testid="project-panel-layout"
      >
        {sharedHeaderBackdrop && !detached ? (
          <div
            aria-hidden="true"
            className={`pointer-events-none absolute inset-x-0 top-0 z-20 h-13 ${PROJECT_COLUMN_HEADER_BACKDROP_CLASS}`}
            data-testid="project-shared-header-backdrop"
          />
        ) : null}
        <div
          className={cn(
            detached
              ? "relative ml-px flex min-h-0 min-w-0 flex-1 overflow-hidden rounded-2xl bg-background"
              : "contents",
          )}
          data-testid={detached ? "project-content-pod" : undefined}
        >
          {sharedHeaderBackdrop && detached ? (
            <div
              aria-hidden="true"
              className={`pointer-events-none absolute inset-x-0 top-0 z-20 h-13 ${PROJECT_COLUMN_HEADER_BACKDROP_CLASS}`}
              data-testid="project-shared-header-backdrop"
            />
          ) : null}
          {children}
        </div>
        {hit ? (
          <ProjectConversationPanel
            canResetWidth={canResetWidth}
            hit={hit}
            onClose={() => setHit(null)}
            onResetWidth={onResetWidth}
            onResizeStart={onResizeStart}
            sharedHeaderBackdrop={sharedHeaderBackdrop}
            widthPx={widthPx}
          />
        ) : (
          <ProjectContextRail
            open={fallbackVisible}
            panelWidthPx={fallbackPanelWidthPx}
            resizing={fallbackPanelResizing}
            rounded={detached}
          >
            {fallbackPanel}
          </ProjectContextRail>
        )}
      </div>
    </ProjectConversationPanelProvider>
  );
}

/** Returns null outside project detail, where normal channel navigation remains. */
export function useProjectConversationPanel() {
  return React.useContext(ProjectConversationPanelContext);
}
