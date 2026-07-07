import { Bot, Search, X } from "lucide-react";
import * as React from "react";

import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import {
  coalesceAgentAutocompleteCandidates,
  getMentionableAgentPubkeys,
  getSharedChannelIds,
} from "@/features/agents/lib/agentAutocompleteEligibility";
import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import {
  useFlattenedUserSearchResults,
  useInfiniteUserSearchQuery,
  useUserSearchFetchMoreOnScroll,
  useUsersBatchQuery,
} from "@/features/profile/hooks";
import { formatOwnerLabel } from "@/features/profile/lib/identity";
import {
  getKeyboardSearchSelection,
  rankUserCandidatesBySearch,
} from "@/features/profile/lib/userCandidateSearch";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { ManagedAgent, UserSearchResult } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { safeNpub } from "@/shared/lib/nostrUtils";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import { PubKey } from "@/shared/ui/PubKey";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  MODAL_SEARCH_INPUT_CLASS,
  MODAL_SEARCH_SHELL_CLASS,
} from "@/shared/ui/modalSearchStyles";

const DIRECT_MESSAGE_RECIPIENT_LIMIT = 50;
const DM_RESULT_ROW_INSET_DIVIDER_CLASS =
  "after:pointer-events-none after:absolute after:bottom-0 after:left-[3.75rem] after:right-0 after:h-px after:bg-border/60 after:content-[''] last:after:hidden";
const BUTTON_LABEL_MORPH_DURATION_MS = 220;
const BUTTON_LABEL_MORPH_EASE = "cubic-bezier(0.23, 1, 0.32, 1)";
const BUTTON_LABEL_FADE_MS = Math.min(
  BUTTON_LABEL_MORPH_DURATION_MS * 0.5,
  150,
);
const BUTTON_LABEL_EXIT_ATTR = "data-button-label-exiting";
const BUTTON_LABEL_CURRENT_ATTR = "data-button-label-current";

function formatUserName(user: UserSearchResult) {
  return (
    user.displayName?.trim() ||
    user.nip05Handle?.trim() ||
    truncatePubkey(user.pubkey)
  );
}

type DirectMessageSearchCandidate = UserSearchResult & {
  isManagedAgent?: boolean;
  isMember?: boolean;
  personaId?: string | null;
};

function directMessageCandidateWithAgentMetadata(
  candidate: UserSearchResult,
  managedAgentsByPubkey: ReadonlyMap<string, ManagedAgent>,
): DirectMessageSearchCandidate {
  const agent = managedAgentsByPubkey.get(normalizePubkey(candidate.pubkey));
  return {
    ...candidate,
    isManagedAgent: Boolean(agent),
    personaId: agent?.personaId,
  };
}

function formatDirectMessageSearchName(user: DirectMessageSearchCandidate) {
  return formatUserName(user);
}

function prefersReducedMotion() {
  return (
    typeof window !== "undefined" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

function createButtonLabelSpan(text: string) {
  const span = document.createElement("span");
  span.setAttribute(BUTTON_LABEL_CURRENT_ATTR, "");
  span.textContent = text;
  span.style.display = "inline-block";
  span.style.willChange = "opacity, transform";
  return span;
}

function clearButtonLabelRoot(root: HTMLElement, text: string) {
  root.replaceChildren(createButtonLabelSpan(text));
  root.style.width = "auto";
  root.style.height = "auto";
}

function MorphingButtonLabel({ text }: { text: string }) {
  const rootRef = React.useRef<HTMLSpanElement>(null);
  const currentTextRef = React.useRef("");
  const cleanupSizeTransitionRef = React.useRef<(() => void) | null>(null);

  React.useLayoutEffect(() => {
    const root = rootRef.current;
    if (!root || text === currentTextRef.current) {
      return;
    }
    const morphRoot = root;

    cleanupSizeTransitionRef.current?.();
    cleanupSizeTransitionRef.current = null;

    if (!currentTextRef.current || prefersReducedMotion()) {
      clearButtonLabelRoot(root, text);
      currentTextRef.current = text;
      return;
    }

    root.querySelectorAll(`[${BUTTON_LABEL_EXIT_ATTR}]`).forEach((element) => {
      element.remove();
    });

    const oldRect = root.getBoundingClientRect();
    const oldWidth = oldRect.width;
    const oldHeight = oldRect.height;
    const rootRect = root.getBoundingClientRect();
    const currentChild = root.querySelector<HTMLElement>(
      `[${BUTTON_LABEL_CURRENT_ATTR}]`,
    );

    if (!currentChild || oldWidth === 0 || oldHeight === 0) {
      clearButtonLabelRoot(root, text);
      currentTextRef.current = text;
      return;
    }

    const currentChildRect = currentChild.getBoundingClientRect();
    const currentChildOpacity =
      Number(getComputedStyle(currentChild).opacity) || 1;
    currentChild.getAnimations().forEach((animation) => {
      animation.cancel();
    });
    currentChild.removeAttribute(BUTTON_LABEL_CURRENT_ATTR);
    currentChild.setAttribute(BUTTON_LABEL_EXIT_ATTR, "");
    currentChild.style.position = "absolute";
    currentChild.style.pointerEvents = "none";
    currentChild.style.left = `${currentChildRect.left - rootRect.left}px`;
    currentChild.style.top = `${currentChildRect.top - rootRect.top}px`;
    currentChild.style.width = `${currentChildRect.width}px`;
    currentChild.style.height = `${currentChildRect.height}px`;
    currentChild.style.opacity = String(currentChildOpacity);

    const nextChild = createButtonLabelSpan(text);
    root.appendChild(nextChild);

    root.style.width = "auto";
    root.style.height = "auto";
    void root.offsetWidth;

    const nextRect = root.getBoundingClientRect();

    root.style.width = `${oldWidth}px`;
    root.style.height = `${oldHeight}px`;
    void root.offsetWidth;

    root.style.width = `${nextRect.width}px`;
    root.style.height = `${nextRect.height}px`;

    function cleanupSizeTransition() {
      morphRoot.removeEventListener("transitionend", handleTransitionEnd);
      window.clearTimeout(fallbackTimer);
      cleanupSizeTransitionRef.current = null;
      if (currentTextRef.current === text) {
        morphRoot.style.width = "auto";
        morphRoot.style.height = "auto";
      }
    }

    function handleTransitionEnd(event: TransitionEvent) {
      if (event.target !== morphRoot) {
        return;
      }
      if (event.propertyName !== "width" && event.propertyName !== "height") {
        return;
      }
      cleanupSizeTransition();
    }

    root.addEventListener("transitionend", handleTransitionEnd);
    const fallbackTimer = window.setTimeout(
      cleanupSizeTransition,
      BUTTON_LABEL_MORPH_DURATION_MS + 50,
    );
    cleanupSizeTransitionRef.current = () => {
      root.removeEventListener("transitionend", handleTransitionEnd);
      window.clearTimeout(fallbackTimer);
    };

    currentChild.animate(
      [{ transform: "none" }, { transform: "scale(0.95)" }],
      {
        duration: BUTTON_LABEL_MORPH_DURATION_MS,
        easing: BUTTON_LABEL_MORPH_EASE,
        fill: "both",
      },
    );
    const exitFade = currentChild.animate(
      [{ opacity: currentChildOpacity }, { opacity: 0 }],
      {
        duration: Math.min(BUTTON_LABEL_MORPH_DURATION_MS * 0.25, 150),
        easing: "linear",
        fill: "both",
      },
    );
    exitFade.onfinish = () => currentChild.remove();

    nextChild.animate([{ transform: "scale(0.95)" }, { transform: "none" }], {
      duration: BUTTON_LABEL_MORPH_DURATION_MS,
      easing: BUTTON_LABEL_MORPH_EASE,
      fill: "both",
    });
    nextChild.animate([{ opacity: 0 }, { opacity: 1 }], {
      delay: Math.min(BUTTON_LABEL_MORPH_DURATION_MS * 0.25, 150),
      duration: BUTTON_LABEL_FADE_MS,
      easing: "linear",
      fill: "both",
    });

    currentTextRef.current = text;
  }, [text]);

  React.useEffect(() => {
    return () => {
      cleanupSizeTransitionRef.current?.();
      rootRef.current?.getAnimations({ subtree: true }).forEach((animation) => {
        animation.cancel();
      });
    };
  }, []);

  return (
    <>
      <span
        aria-hidden="true"
        className="relative inline-block whitespace-nowrap text-left align-middle leading-none will-change-[width,height]"
        ref={rootRef}
        style={{
          transitionDuration: `${BUTTON_LABEL_MORPH_DURATION_MS}ms`,
          transitionProperty: "width, height",
          transitionTimingFunction: BUTTON_LABEL_MORPH_EASE,
        }}
      />
      <span className="sr-only">{text}</span>
    </>
  );
}

export function NewDirectMessageDialog({
  currentPubkey,
  isPending,
  onOpenChange,
  onSubmit,
  open,
}: {
  currentPubkey?: string;
  isPending: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (input: { pubkeys: string[] }) => Promise<void>;
  open: boolean;
}) {
  const [searchQuery, setSearchQuery] = React.useState("");
  const [selectedUsers, setSelectedUsers] = React.useState<UserSearchResult[]>(
    [],
  );
  const [submitErrorMessage, setSubmitErrorMessage] = React.useState<
    string | null
  >(null);
  const searchInputRef = React.useRef<HTMLInputElement>(null);
  const selectedRecipientsRef = React.useRef<HTMLDivElement>(null);
  const suppressCloseAutoFocusRef = React.useRef(false);
  const selectedUsersCountRef = React.useRef(0);
  const [selectedRecipientsHeight, setSelectedRecipientsHeight] =
    React.useState(0);
  const deferredSearchQuery = React.useDeferredValue(searchQuery.trim());
  const hasReachedRecipientLimit = selectedUsers.length >= 8;
  const selectedPubkeys = React.useMemo(
    () => new Set(selectedUsers.map((user) => normalizePubkey(user.pubkey))),
    [selectedUsers],
  );
  const identityQuery = useIdentityQuery();
  const managedAgentsQuery = useManagedAgentsQuery({ enabled: open });
  const relayAgentsQuery = useRelayAgentsQuery({ enabled: open });
  const channelsQuery = useChannelsQuery({ enabled: open });
  const userSearchQuery = useInfiniteUserSearchQuery(deferredSearchQuery, {
    allowEmpty: true,
    enabled: open && !hasReachedRecipientLimit,
    limit: DIRECT_MESSAGE_RECIPIENT_LIMIT,
  });
  const userSearchResults = useFlattenedUserSearchResults(userSearchQuery.data);
  const isArchivedDiscovery = useIsArchivedPredicate();
  const searchResults = React.useMemo(() => {
    const candidatesByPubkey = new Map<string, DirectMessageSearchCandidate>();
    const managedAgentsByPubkey = new Map(
      (managedAgentsQuery.data ?? []).map((agent) => [
        normalizePubkey(agent.pubkey),
        agent,
      ]),
    );
    const currentPubkeyNormalized = currentPubkey
      ? normalizePubkey(currentPubkey)
      : null;
    const eligibleAgentPubkeys = getMentionableAgentPubkeys({
      currentPubkey,
      managedAgentPubkeys: (managedAgentsQuery.data ?? []).map(
        (agent) => agent.pubkey,
      ),
      relayAgents: relayAgentsQuery.data,
      sharedChannelIds: getSharedChannelIds(channelsQuery.data),
    });

    const addCandidate = (candidate: DirectMessageSearchCandidate) => {
      const pubkey = normalizePubkey(candidate.pubkey);

      if (
        pubkey === currentPubkeyNormalized ||
        selectedPubkeys.has(pubkey) ||
        isArchivedDiscovery(pubkey) ||
        (candidate.isAgent && !eligibleAgentPubkeys.has(pubkey))
      ) {
        return;
      }

      const current = candidatesByPubkey.get(pubkey);
      if (!current) {
        candidatesByPubkey.set(pubkey, { ...candidate, pubkey });
        return;
      }

      const candidateName = candidate.displayName?.trim() || null;
      const currentName = current.displayName?.trim() || null;

      candidatesByPubkey.set(pubkey, {
        pubkey,
        avatarUrl: current.avatarUrl ?? candidate.avatarUrl ?? null,
        displayName:
          candidate.isAgent && candidateName
            ? candidateName
            : current.isAgent
              ? currentName
              : (currentName ?? candidateName),
        nip05Handle: current.nip05Handle ?? candidate.nip05Handle ?? null,
        ownerPubkey: current.ownerPubkey ?? candidate.ownerPubkey ?? null,
        isAgent: current.isAgent || candidate.isAgent,
        isManagedAgent: current.isManagedAgent || candidate.isManagedAgent,
        isMember: current.isMember || candidate.isMember,
        personaId: current.personaId ?? candidate.personaId,
      });
    };

    for (const user of userSearchResults) {
      addCandidate(
        directMessageCandidateWithAgentMetadata(user, managedAgentsByPubkey),
      );
    }

    for (const agent of relayAgentsQuery.data ?? []) {
      if (!eligibleAgentPubkeys.has(normalizePubkey(agent.pubkey))) {
        continue;
      }

      addCandidate({
        pubkey: agent.pubkey,
        displayName: agent.name,
        avatarUrl: null,
        nip05Handle: null,
        ownerPubkey: null,
        isAgent: true,
      });
    }

    for (const agent of managedAgentsQuery.data ?? []) {
      addCandidate({
        pubkey: agent.pubkey,
        displayName: agent.name,
        avatarUrl: null,
        nip05Handle: null,
        ownerPubkey: currentPubkey ?? null,
        isAgent: true,
        isManagedAgent: true,
        personaId: agent.personaId,
      });
    }

    const coalescedCandidates = coalesceAgentAutocompleteCandidates(
      [...candidatesByPubkey.values()],
      {
        currentPubkey,
        getLabel: formatDirectMessageSearchName,
      },
    );

    return rankUserCandidatesBySearch({
      allowEmptyQuery: true,
      candidates: coalescedCandidates,
      getLabel: formatUserName,
      limit: Math.max(
        DIRECT_MESSAGE_RECIPIENT_LIMIT,
        coalescedCandidates.length,
      ),
      query: deferredSearchQuery,
    });
  }, [
    channelsQuery.data,
    currentPubkey,
    deferredSearchQuery,
    isArchivedDiscovery,
    managedAgentsQuery.data,
    relayAgentsQuery.data,
    selectedPubkeys,
    userSearchResults,
  ]);
  const isDirectoryLoading =
    userSearchQuery.isLoading ||
    managedAgentsQuery.isLoading ||
    relayAgentsQuery.isLoading ||
    channelsQuery.isLoading;
  const handleDirectoryScroll = useUserSearchFetchMoreOnScroll(
    userSearchQuery,
    !hasReachedRecipientLimit,
  );

  const searchOwnerPubkeys = React.useMemo(
    () => [
      ...new Set(
        searchResults
          .map((user) => user.ownerPubkey)
          .filter((pubkey): pubkey is string =>
            Boolean(
              pubkey &&
                pubkey.toLowerCase() !==
                  identityQuery.data?.pubkey?.toLowerCase(),
            ),
          ),
      ),
    ],
    [identityQuery.data?.pubkey, searchResults],
  );
  const ownerProfilesQuery = useUsersBatchQuery(searchOwnerPubkeys, {
    enabled: open && searchOwnerPubkeys.length > 0,
  });

  React.useEffect(() => {
    if (!open) {
      setSearchQuery("");
      setSelectedUsers([]);
      setSubmitErrorMessage(null);
      setSelectedRecipientsHeight(0);
      selectedUsersCountRef.current = 0;
      return;
    }

    suppressCloseAutoFocusRef.current = false;
    searchInputRef.current?.focus();
  }, [open]);

  React.useEffect(() => {
    if (selectedUsers.length > selectedUsersCountRef.current) {
      setSearchQuery("");
    }
    selectedUsersCountRef.current = selectedUsers.length;
  }, [selectedUsers.length]);

  React.useEffect(() => {
    const node = selectedRecipientsRef.current;
    if (!node) {
      setSelectedRecipientsHeight(0);
      return;
    }

    const updateHeight = () => {
      setSelectedRecipientsHeight(
        selectedUsers.length > 0 ? node.scrollHeight : 0,
      );
    };

    const animationFrame = window.requestAnimationFrame(updateHeight);
    const resizeObserver = new ResizeObserver(updateHeight);
    resizeObserver.observe(node);

    return () => {
      window.cancelAnimationFrame(animationFrame);
      resizeObserver.disconnect();
    };
  }, [selectedUsers.length]);

  function handleSelectUser(user: UserSearchResult) {
    if (hasReachedRecipientLimit) {
      return;
    }

    setSelectedUsers((current) => {
      const pubkey = normalizePubkey(user.pubkey);
      if (
        current.some(
          (candidate) => normalizePubkey(candidate.pubkey) === pubkey,
        )
      ) {
        return current;
      }

      return [...current, user];
    });
    setSearchQuery("");
    setSubmitErrorMessage(null);
    searchInputRef.current?.focus({ preventScroll: true });
  }

  async function submitDirectMessage() {
    if (isPending || selectedUsers.length === 0) {
      return;
    }

    setSubmitErrorMessage(null);

    try {
      await onSubmit({
        pubkeys: selectedUsers.map((user) => user.pubkey),
      });
      suppressCloseAutoFocusRef.current = true;
      onOpenChange(false);
    } catch (error) {
      setSubmitErrorMessage(
        error instanceof Error
          ? error.message
          : "Failed to open direct message.",
      );
    }
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        aria-describedby={undefined}
        className="max-w-xl gap-0 overflow-hidden border-0 px-6 pb-0 pt-6"
        data-testid="new-dm-dialog"
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          searchInputRef.current?.focus({ preventScroll: true });
        }}
        onCloseAutoFocus={(event) => {
          if (!suppressCloseAutoFocusRef.current) {
            return;
          }

          event.preventDefault();
          suppressCloseAutoFocusRef.current = false;
        }}
        showCloseButton={false}
      >
        <DialogHeader className="space-y-0 pb-5">
          <div className="flex items-center justify-between gap-4">
            <DialogTitle>New direct message</DialogTitle>
            <DialogClose className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors duration-150 ease-out hover:bg-accent hover:text-accent-foreground focus:outline-hidden focus:ring-1 focus:ring-ring">
              <X className="h-4 w-4" />
              <span className="sr-only">Close</span>
            </DialogClose>
          </div>
          <label className={MODAL_SEARCH_SHELL_CLASS} htmlFor="new-dm-search">
            <Search className="h-4 w-4 shrink-0 text-muted-foreground/55 transition-colors duration-150 ease-out group-hover/search:text-muted-foreground group-focus-within/search:text-foreground" />
            <input
              autoCapitalize="none"
              autoCorrect="off"
              className={MODAL_SEARCH_INPUT_CLASS}
              data-testid="new-dm-search"
              disabled={isPending}
              id="new-dm-search"
              onChange={(event) => {
                setSearchQuery(event.target.value);
                setSubmitErrorMessage(null);
              }}
              onKeyDown={(event) => {
                if (event.key !== "Enter") {
                  return;
                }

                if (searchQuery.trim().length === 0) {
                  if (selectedUsers.length > 0) {
                    event.preventDefault();
                    void submitDirectMessage();
                  }
                  return;
                }

                const keyboardSelection = getKeyboardSearchSelection({
                  currentQuery: searchQuery,
                  rankedQuery: deferredSearchQuery,
                  results: searchResults,
                });
                if (!keyboardSelection) {
                  return;
                }

                event.preventDefault();
                handleSelectUser(keyboardSelection);
              }}
              placeholder="Search people and agents"
              ref={searchInputRef}
              spellCheck={false}
              type="text"
              value={searchQuery}
            />
          </label>
        </DialogHeader>

        <form
          className="flex flex-col"
          onSubmit={(event) => {
            event.preventDefault();
            void submitDirectMessage();
          }}
        >
          <div
            className="overflow-hidden transition-[height,opacity] duration-200 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none"
            style={{
              height: selectedRecipientsHeight,
              opacity: selectedUsers.length > 0 ? 1 : 0,
            }}
          >
            <div className="pb-4" ref={selectedRecipientsRef}>
              {selectedUsers.length > 0 ? (
                <div className="flex flex-wrap gap-2">
                  {selectedUsers.map((user) => (
                    <button
                      aria-label={`Remove ${formatUserName(user)}`}
                      className="group/selected-recipient inline-flex items-center gap-2 rounded-full border border-border/80 bg-background/80 py-1 pl-1 pr-3 text-xs transition-colors duration-150 ease-out hover:bg-muted/50 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60"
                      data-testid={`new-dm-selected-${user.pubkey}`}
                      disabled={isPending}
                      key={user.pubkey}
                      onClick={() => {
                        setSelectedUsers((current) =>
                          current.filter(
                            (candidate) => candidate.pubkey !== user.pubkey,
                          ),
                        );
                      }}
                      type="button"
                    >
                      <span className="relative h-8 w-8 shrink-0">
                        <ProfileAvatar
                          avatarUrl={user.avatarUrl}
                          className="h-8 w-8 text-xs shadow-none transition-opacity duration-150 ease-out group-hover/selected-recipient:opacity-0 group-focus-visible/selected-recipient:opacity-0"
                          iconClassName="h-4 w-4"
                          label={formatUserName(user)}
                        />
                        <span className="absolute inset-0 flex items-center justify-center rounded-full bg-primary text-primary-foreground opacity-0 shadow transition-colors duration-150 ease-out group-hover/selected-recipient:bg-primary/90 group-hover/selected-recipient:opacity-100 group-focus-visible/selected-recipient:bg-primary/90 group-focus-visible/selected-recipient:opacity-100">
                          <X aria-hidden="true" className="h-4 w-4" />
                        </span>
                      </span>
                      <span className="font-medium">
                        {formatUserName(user)}
                      </span>
                      {user.isAgent ? (
                        <Bot
                          aria-label="agent"
                          className="h-4 w-4 text-muted-foreground"
                        />
                      ) : null}
                    </button>
                  ))}
                </div>
              ) : null}
              {selectedUsers.length > 0 ? (
                <div className="mt-2 space-y-1">
                  {selectedUsers.map((user) => (
                    <div
                      className="flex min-w-0 flex-wrap items-baseline gap-x-2 text-2xs text-muted-foreground"
                      data-testid={`new-dm-pubkey-${user.pubkey}`}
                      key={user.pubkey}
                    >
                      <span className="font-medium">
                        {formatUserName(user)}
                      </span>
                      <PubKey pubkey={user.pubkey} variant="full" />
                    </div>
                  ))}
                </div>
              ) : null}
            </div>
          </div>

          <div
            className="overflow-hidden transition-[max-height,opacity] duration-200 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none"
            style={{
              maxHeight: hasReachedRecipientLimit ? 0 : "min(50vh, 24rem)",
              opacity: hasReachedRecipientLimit ? 0 : 1,
            }}
          >
            <div
              className="h-[min(50vh,24rem)] overflow-y-auto rounded-xl border border-border/70 bg-background/70"
              data-testid="new-dm-results"
              onScroll={handleDirectoryScroll}
            >
              {searchResults.length > 0 ? (
                <div>
                  {searchResults.map((user) => {
                    const ownerLabel = formatOwnerLabel(
                      user.ownerPubkey,
                      currentPubkey ?? identityQuery.data?.pubkey,
                      ownerProfilesQuery.data?.profiles,
                    );

                    return (
                      <div
                        className={cn(
                          "group/dm-result relative flex min-h-14 w-full items-center gap-3 px-4 py-3.5 text-left transition-colors duration-150 ease-out hover:bg-muted/40 focus-within:bg-muted/40",
                          DM_RESULT_ROW_INSET_DIVIDER_CLASS,
                        )}
                        key={user.pubkey}
                      >
                        <button
                          aria-label={`Add ${formatUserName(user)}`}
                          className="absolute inset-0 z-0 cursor-pointer focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
                          data-testid={`new-dm-result-${user.pubkey}`}
                          disabled={isPending || hasReachedRecipientLimit}
                          onClick={() => {
                            handleSelectUser(user);
                          }}
                          type="button"
                        />
                        <ProfileAvatar
                          avatarUrl={user.avatarUrl}
                          className="pointer-events-none relative z-10 h-8 w-8 text-xs shadow-none"
                          iconClassName="h-4 w-4"
                          label={formatUserName(user)}
                        />
                        <div className="pointer-events-none relative z-10 min-w-0 flex-1">
                          {user.isAgent ? (
                            <div className="min-w-0">
                              <div className="flex min-w-0 items-center gap-2">
                                <span className="truncate text-sm font-medium tracking-tight">
                                  {formatUserName(user)}
                                </span>
                                <span className="inline-flex shrink-0 items-center gap-1 text-xs text-muted-foreground">
                                  <Bot
                                    aria-hidden="true"
                                    className="h-3 w-3"
                                    data-testid="new-dm-agent-icon"
                                  />
                                  agent
                                </span>
                              </div>
                              {ownerLabel ? (
                                <span className="block truncate text-xs text-muted-foreground">
                                  owned by {ownerLabel}
                                </span>
                              ) : null}
                              <span
                                className="hidden min-w-0 break-all font-mono text-2xs leading-snug text-muted-foreground group-hover/dm-result:block group-focus-within/dm-result:block"
                                data-testid={`new-dm-npub-${user.pubkey}`}
                              >
                                {safeNpub(user.pubkey) ??
                                  truncatePubkey(user.pubkey)}
                              </span>
                            </div>
                          ) : (
                            <span className="block truncate text-sm font-medium tracking-tight">
                              {formatUserName(user)}
                            </span>
                          )}
                        </div>
                        <Button
                          aria-label={`Add ${formatUserName(user)}`}
                          className={cn(
                            "relative z-20 shrink-0 transition-opacity duration-150 ease-out",
                            isPending
                              ? "invisible opacity-0 disabled:opacity-0"
                              : "opacity-0 group-hover/dm-result:opacity-100 group-focus-within/dm-result:opacity-100",
                          )}
                          data-testid={`new-dm-add-${user.pubkey}`}
                          disabled={isPending || hasReachedRecipientLimit}
                          onClick={(event) => {
                            event.stopPropagation();
                            handleSelectUser(user);
                          }}
                          size="sm"
                          type="button"
                        >
                          Add
                        </Button>
                      </div>
                    );
                  })}
                </div>
              ) : isDirectoryLoading ? (
                <p className="px-4 py-3 text-sm text-muted-foreground">
                  {deferredSearchQuery.length === 0
                    ? "Loading people and agents…"
                    : "Searching…"}
                </p>
              ) : (
                <p
                  className="px-4 py-3 text-sm text-muted-foreground"
                  data-testid="new-dm-empty"
                >
                  {deferredSearchQuery.length === 0
                    ? "No people or agents available to message."
                    : "No matching users."}
                </p>
              )}
            </div>
          </div>

          {hasReachedRecipientLimit ? (
            <p
              className="mt-4 text-sm text-destructive"
              data-testid="new-dm-limit"
            >
              DMs support up to nine people, including you.
            </p>
          ) : null}

          {userSearchQuery.error instanceof Error ? (
            <p className="mt-4 text-sm text-destructive">
              {userSearchQuery.error.message}
            </p>
          ) : null}

          {submitErrorMessage ? (
            <p className="mt-4 text-sm text-destructive">
              {submitErrorMessage}
            </p>
          ) : null}

          <div className="flex items-center gap-3 py-4">
            <div className="ml-auto flex items-center gap-2">
              <Button
                disabled={isPending}
                onClick={() => onOpenChange(false)}
                type="button"
                variant="ghost"
              >
                Cancel
              </Button>
              <Button
                data-testid="new-dm-submit"
                disabled={isPending || selectedUsers.length === 0}
                type="submit"
              >
                <MorphingButtonLabel
                  text={
                    isPending
                      ? "Opening..."
                      : selectedUsers.length > 1
                        ? "Start group DM"
                        : "Message"
                  }
                />
              </Button>
            </div>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
