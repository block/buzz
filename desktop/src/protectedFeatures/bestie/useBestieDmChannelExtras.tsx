import { ListTodo, Plus } from "lucide-react";
import * as React from "react";

import { findBestieDmChannel } from "./filterBestieDmChannels";
import { subscribeBestieRhsOpen } from "./bestieRhsOpenRequest";
import { BestieDmCategorySheet, BestieDmRhsPanel } from "./BestieDmRhsPanel";
import { BestieDmCoffeeSheet } from "./BestieDmCoffeeSheet";
import { BestieDmJobsSheet } from "./BestieDmJobsSheet";
import { BestieDmThreadsSheet } from "./BestieDmThreadsSheet";
import { BestieDmScratchSheet } from "./BestieDmScratchSheet";
import { BestieDmTodosSheet } from "./BestieDmTodosSheet";
import { BESTIE_COFFEE_BREW_EVENT } from "./bestieCoffeeSchedule";
import {
  abandonBestieCoffeePendingForScope,
  beginBestieCoffeeRunForScope,
} from "./bestieCoffeeStore";
import { useBestieCoffeeLive } from "./useBestieCoffeeLive";
import { useBestieThreadSummarizeLive } from "./useBestieThreadSummarizeLive";
import {
  bestieCategoryTitle,
  bestieIdleAuxiliaryKind,
  type BestieRhsKind,
} from "./bestieDmRhsHelpers";
import { setBestieViewingDm } from "./bestieAttentionStore";
import { useBestieAssignmentQuery } from "./useBestie";
import type { Channel } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import type { ChannelScreenProps } from "@/features/channels/ui/ChannelScreen.types";
import type { IdleAuxiliaryHeaderControls } from "@/features/channels/ui/IdleAuxiliaryPanel";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { stripBestieOutboundHints } from "./bestieOutboundHints";
import { withBestieLiveListStateHint } from "./bestieLiveListState";

type BestieChannelScreenExtras = Pick<
  ChannelScreenProps,
  | "headerEndActions"
  | "idleAuxiliaryPanel"
  | "idleAuxiliaryTitle"
  | "idleAuxiliaryOverridesThread"
  | "idleAuxiliaryHeaderActions"
  | "onCloseIdleAuxiliaryPanel"
  | "transformDisplayedMessageBody"
  | "transformOutboundMessageContent"
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
 * when drilling into a category’s items (+ add). Empty otherwise.
 */
export function useBestieDmChannelExtras(
  activeChannel: Channel | null | undefined,
): BestieChannelExtras {
  const { assignmentQuery, ownerPubkey, relayUrl } =
    useBestieAssignmentQuery(true);
  const [panelOpen, setPanelOpen] = React.useState(true);
  const [activeKind, setActiveKind] = React.useState<BestieRhsKind | null>(
    null,
  );
  const [adding, setAdding] = React.useState(false);

  const bestiePubkey = assignmentQuery.data?.agentPubkey ?? null;
  const isBestieDm = React.useMemo(() => {
    if (!activeChannel || !ownerPubkey || !bestiePubkey) {
      return false;
    }
    const found = findBestieDmChannel(
      [activeChannel],
      ownerPubkey,
      bestiePubkey,
    );
    return found?.id === activeChannel.id;
  }, [activeChannel, bestiePubkey, ownerPubkey]);

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

  // Footer unread clears while the Bestie DM thread is the active view.
  React.useEffect(() => {
    setBestieViewingDm(isBestieDm);
    return () => setBestieViewingDm(false);
  }, [isBestieDm]);

  const openKind = React.useCallback((kind: BestieRhsKind) => {
    setAdding(false);
    setActiveKind((current) => (current === kind ? null : kind));
  }, []);

  React.useEffect(() => {
    if (!isBestieDm) return;
    return subscribeBestieRhsOpen((kind) => {
      setPanelOpen(true);
      setAdding(false);
      setActiveKind(kind);
    });
  }, [isBestieDm]);

  const onCloseIdleAuxiliaryPanel = React.useCallback(() => {
    // Slide close → back to fixed category list (column stays open).
    setActiveKind(null);
    setAdding(false);
  }, []);

  const headerToggle = React.useMemo(() => {
    if (!isBestieDm) return null;
    const label = panelOpen ? "Hide Assistant list" : "Show Assistant list";
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

  const bestieChannelForCoffee = isBestieDm ? activeChannel : null;
  const { brewDisabled, coffeeLive } = useBestieCoffeeLive(
    scope,
    bestieChannelForCoffee,
  );
  const { summarizeDisabled, summarizeLive, summarizeLiveThreadId } =
    useBestieThreadSummarizeLive(scope, bestieChannelForCoffee);
  const requestCoffeeBrew = React.useCallback(() => {
    if (!scope || brewDisabled) return;
    let begun = beginBestieCoffeeRunForScope(scope, "brew");
    if (!begun) {
      // Leftover pending with Brew re-enabled (idle past grace) — finalize + retry.
      abandonBestieCoffeePendingForScope(scope);
      begun = beginBestieCoffeeRunForScope(scope, "brew");
    }
    if (!begun) return;
    window.dispatchEvent(
      new CustomEvent(BESTIE_COFFEE_BREW_EVENT, {
        detail: { agentPubkey: scope.agentPubkey },
      }),
    );
  }, [brewDisabled, scope]);

  const contextColumn = React.useMemo(() => {
    if (!scope) return null;
    return (
      <BestieDmRhsPanel
        activeKind={activeKind}
        coffeeLive={coffeeLive}
        onOpenKind={openKind}
        scope={scope}
        summarizeLive={summarizeLive}
      />
    );
  }, [activeKind, coffeeLive, openKind, scope, summarizeLive]);

  const idleAuxiliaryPanel = React.useMemo(() => {
    if (!scope || activeKind == null) return null;
    if (activeKind === "job") {
      return (
        <BestieDmJobsSheet
          adding={adding}
          onRequestAdd={() => setAdding(true)}
          scope={scope}
        />
      );
    }
    if (activeKind === "coffee") {
      return (
        <BestieDmCoffeeSheet
          brewDisabled={brewDisabled}
          coffeeLive={coffeeLive}
          onBrew={requestCoffeeBrew}
          scope={scope}
        />
      );
    }
    if (activeKind === "thread") {
      return (
        <BestieDmThreadsSheet
          adding={adding}
          onRequestAdd={() => setAdding(true)}
          scope={scope}
          summarizeDisabled={summarizeDisabled}
          summarizeLive={summarizeLive}
          summarizeLiveThreadId={summarizeLiveThreadId}
        />
      );
    }
    if (activeKind === "scratch") {
      return (
        <BestieDmScratchSheet
          adding={adding}
          onAdded={() => setAdding(false)}
          onRequestAdd={() => setAdding(true)}
          scope={scope}
        />
      );
    }
    if (activeKind === "todo") {
      return (
        <BestieDmTodosSheet
          adding={adding}
          onRequestAdd={() => setAdding(true)}
          scope={scope}
        />
      );
    }
    return (
      <BestieDmCategorySheet
        adding={adding}
        kind={activeKind}
        onRequestAdd={() => setAdding(true)}
        scope={scope}
      />
    );
  }, [activeKind, adding, brewDisabled, coffeeLive, requestCoffeeBrew, scope, summarizeDisabled, summarizeLive, summarizeLiveThreadId]);

  const idleAuxiliaryHeaderActions =
    React.useMemo<IdleAuxiliaryHeaderControls | null>(() => {
      if (activeKind == null) return null;
      const back = {
        backLabel: "Back to Assistant list",
        backVariant: "chip" as const,
        onBack: () => {
          setActiveKind(null);
          setAdding(false);
        },
      };
      if (activeKind === "coffee") {
        return { ...back };
      }
      const addLabel =
        activeKind === "reminder"
          ? "Add reminder"
          : activeKind === "todo"
            ? "Add to-do"
            : activeKind === "scratch"
              ? "Add scratch note"
              : activeKind === "thread"
                ? "Add thread"
                : "Add job";
      return {
        ...back,
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
      };
    }, [activeKind, adding]);

  return React.useMemo(() => {
    if (!isBestieDm || !scope) {
      return {
        contextColumn: null,
        contextColumnOpen: false,
      };
    }

    const screenExtras: BestieChannelScreenExtras = {
      headerEndActions: headerToggle,
      transformDisplayedMessageBody: stripBestieOutboundHints,
      transformOutboundMessageContent: (content) =>
        withBestieLiveListStateHint(content, scope),
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
    headerToggle,
    idleAuxiliaryHeaderActions,
    idleAuxiliaryPanel,
    isBestieDm,
    onCloseIdleAuxiliaryPanel,
    panelOpen,
    scope,
  ]);
}
