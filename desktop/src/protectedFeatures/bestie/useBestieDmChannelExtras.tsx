import { ListTodo, Plus } from "lucide-react";
import * as React from "react";

import { findBestieDmChannel } from "./filterBestieDmChannels";
import { BestieDmCategorySheet, BestieDmRhsPanel } from "./BestieDmRhsPanel";
import {
  bestieCategoryTitle,
  bestieIdleAuxiliaryKind,
} from "./bestieDmRhsHelpers";
import { useBestieAssignmentQuery } from "./useBestie";
import type { BestieListKind } from "./bestieListTypes";
import type { Channel } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import type { ChannelScreenProps } from "@/features/channels/ui/ChannelScreen.types";
import type { IdleAuxiliaryHeaderControls } from "@/features/channels/ui/IdleAuxiliaryPanel";
import { useFeatureEnabled } from "@/shared/features";
import { normalizePubkey } from "@/shared/lib/pubkey";

type BestieChannelScreenExtras = Pick<
  ChannelScreenProps,
  | "headerEndActions"
  | "idleAuxiliaryPanel"
  | "idleAuxiliaryTitle"
  | "idleAuxiliaryOverridesThread"
  | "idleAuxiliaryHeaderActions"
  | "onCloseIdleAuxiliaryPanel"
>;

export type BestieChannelExtras = BestieChannelScreenExtras & {
  /**
   * Fixed RHS category rows (Reminders / To-dos). Rendered outside
   * ChannelScreen in {@link BestieDmChannelFrame} — never inside idleAuxiliary.
   */
  contextColumn: React.ReactNode | null;
  contextColumnOpen: boolean;
};

/**
 * When the active channel is the Bestie DM, attach project-home-style RHS:
 * fixed category rows in the native right column; idleAuxiliary slide only
 * when drilling into a category’s items (+ add). Empty otherwise (and always
 * when Bestie is disabled).
 */
export function useBestieDmChannelExtras(
  activeChannel: Channel | null | undefined,
): BestieChannelExtras {
  const enabled = useFeatureEnabled("bestie");
  const { assignmentQuery, ownerPubkey, relayUrl } =
    useBestieAssignmentQuery(enabled);
  const [panelOpen, setPanelOpen] = React.useState(true);
  const [activeKind, setActiveKind] = React.useState<BestieListKind | null>(
    null,
  );
  const [adding, setAdding] = React.useState(false);

  const bestiePubkey = assignmentQuery.data?.agentPubkey ?? null;
  const isBestieDm = React.useMemo(() => {
    if (!enabled || !activeChannel || !ownerPubkey || !bestiePubkey) {
      return false;
    }
    const found = findBestieDmChannel(
      [activeChannel],
      ownerPubkey,
      bestiePubkey,
    );
    return found?.id === activeChannel.id;
  }, [activeChannel, bestiePubkey, enabled, ownerPubkey]);

  // Re-open fixed RHS (category list) when navigating into the Bestie DM.
  React.useEffect(() => {
    if (isBestieDm) {
      setPanelOpen(true);
      setActiveKind(null);
      setAdding(false);
    }
  }, [isBestieDm]);

  const scope = React.useMemo(() => {
    if (!relayUrl || !ownerPubkey || !bestiePubkey) return null;
    return {
      agentPubkey: normalizePubkey(bestiePubkey),
      ownerPubkey: normalizePubkey(ownerPubkey),
      relayUrl,
    };
  }, [bestiePubkey, ownerPubkey, relayUrl]);

  const openKind = React.useCallback((kind: BestieListKind) => {
    setAdding(false);
    setActiveKind((current) => (current === kind ? null : kind));
  }, []);

  const onCloseIdleAuxiliaryPanel = React.useCallback(() => {
    // Slide close → back to fixed category list (column stays open).
    setActiveKind(null);
    setAdding(false);
  }, []);

  const headerToggle = React.useMemo(() => {
    if (!isBestieDm) return null;
    const label = panelOpen ? "Hide Bestie list" : "Show Bestie list";
    return (
      <Tooltip disableHoverableContent>
        <TooltipTrigger asChild>
          <Button
            aria-label={label}
            aria-pressed={panelOpen}
            className="shrink-0"
            data-testid="bestie-dm-rhs-toggle"
            onClick={() => {
              setPanelOpen((open) => {
                if (open) {
                  setActiveKind(null);
                  setAdding(false);
                }
                return !open;
              });
            }}
            size="icon"
            title={label}
            type="button"
            variant={panelOpen ? "secondary" : "ghost"}
          >
            <ListTodo />
          </Button>
        </TooltipTrigger>
        <TooltipContent>{label}</TooltipContent>
      </Tooltip>
    );
  }, [isBestieDm, panelOpen]);

  const contextColumn = React.useMemo(() => {
    if (!scope) return null;
    return (
      <BestieDmRhsPanel
        activeKind={activeKind}
        onOpenKind={openKind}
        scope={scope}
      />
    );
  }, [activeKind, openKind, scope]);

  const idleAuxiliaryPanel = React.useMemo(() => {
    if (!scope || activeKind == null) return null;
    return (
      <BestieDmCategorySheet
        adding={adding}
        kind={activeKind}
        onRequestAdd={() => setAdding(true)}
        scope={scope}
      />
    );
  }, [activeKind, adding, scope]);

  const idleAuxiliaryHeaderActions =
    React.useMemo<IdleAuxiliaryHeaderControls | null>(() => {
      if (activeKind == null) return null;
      const addLabel = activeKind === "reminder" ? "Add reminder" : "Add to-do";
      return {
        actions: (
          <Tooltip disableHoverableContent>
            <TooltipTrigger asChild>
              <Button
                aria-label={addLabel}
                aria-pressed={adding}
                className="h-7 w-7 shrink-0 text-muted-foreground hover:text-foreground"
                data-testid="bestie-dm-category-sheet-create"
                onClick={() => setAdding((value) => !value)}
                size="icon"
                title={addLabel}
                type="button"
                variant={adding ? "secondary" : "ghost"}
              >
                <Plus className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>{addLabel}</TooltipContent>
          </Tooltip>
        ),
        backLabel: "Back to Bestie list",
        onBack: () => {
          setActiveKind(null);
          setAdding(false);
        },
      };
    }, [activeKind, adding]);

  return React.useMemo(() => {
    if (!enabled || !isBestieDm || !scope) {
      return {
        contextColumn: null,
        contextColumnOpen: false,
      };
    }

    const screenExtras: BestieChannelScreenExtras = {
      headerEndActions: headerToggle,
    };

    // Slide only when drilling into a category — never for the category list.
    const drillKind = bestieIdleAuxiliaryKind(activeKind);
    if (panelOpen && drillKind != null && idleAuxiliaryPanel) {
      screenExtras.idleAuxiliaryHeaderActions =
        idleAuxiliaryHeaderActions ?? undefined;
      screenExtras.idleAuxiliaryOverridesThread = true;
      screenExtras.idleAuxiliaryPanel = idleAuxiliaryPanel;
      screenExtras.idleAuxiliaryTitle = bestieCategoryTitle(drillKind);
      screenExtras.onCloseIdleAuxiliaryPanel = onCloseIdleAuxiliaryPanel;
    }

    return {
      ...screenExtras,
      contextColumn,
      contextColumnOpen: panelOpen,
    };
  }, [
    activeKind,
    contextColumn,
    enabled,
    headerToggle,
    idleAuxiliaryHeaderActions,
    idleAuxiliaryPanel,
    isBestieDm,
    onCloseIdleAuxiliaryPanel,
    panelOpen,
    scope,
  ]);
}
