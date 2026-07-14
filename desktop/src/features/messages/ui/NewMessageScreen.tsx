import { Bot, X } from "lucide-react";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  useOpenDmMutation,
  useUpsertCachedChannel,
} from "@/features/channels/hooks";
import type { Channel } from "@/shared/api/types";
import { useSendMessageMutation } from "@/features/messages/hooks";
import { getKeyboardSearchSelection } from "@/features/profile/lib/userCandidateSearch";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { useIdentityQuery } from "@/shared/api/hooks";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  POOF_ORIGIN_CLASS,
  POOF_TRIGGER_CLASS,
} from "@/shared/ui/PoofBurstProvider";
import { PubKey } from "@/shared/ui/PubKey";
import { Popover, PopoverAnchor, PopoverContent } from "@/shared/ui/popover";
import { Skeleton } from "@/shared/ui/skeleton";

import { MessageComposer } from "./MessageComposer";
import { NewMessageResultRow } from "./NewMessageResultRow";
import {
  formatRecipientName,
  useNewMessageRecipients,
} from "./useNewMessageRecipients";

/**
 * Conversation-shaped compose surface for starting a direct message. The
 * normal chat header becomes an inline "To:" field, while recipient discovery
 * lives in an attached popover instead of taking over the message area.
 */
export function NewMessageScreen() {
  const identityQuery = useIdentityQuery();
  const currentPubkey = identityQuery.data?.pubkey;
  const openDmMutation = useOpenDmMutation();
  const upsertCachedChannel = useUpsertCachedChannel();
  const sendMessageMutation = useSendMessageMutation(null, identityQuery.data);
  const { goChannel } = useAppNavigation();

  const [isRecipientPickerOpen, setIsRecipientPickerOpen] =
    React.useState(true);
  const [highlightedRecipientPubkey, setHighlightedRecipientPubkey] =
    React.useState<string | null>(null);
  const [inspectedRecipientPubkey, setInspectedRecipientPubkey] =
    React.useState<string | null>(null);
  const [submitErrorMessage, setSubmitErrorMessage] = React.useState<
    string | null
  >(null);
  const [isPreparingMentionSend, setIsPreparingMentionSend] =
    React.useState(false);
  const searchInputRef = React.useRef<HTMLInputElement>(null);
  const toFieldRef = React.useRef<HTMLDivElement>(null);
  const preparedDirectMessageRef = React.useRef<Channel | null>(null);
  const isMountedRef = React.useRef(false);
  const isPending =
    isPreparingMentionSend ||
    openDmMutation.isPending ||
    sendMessageMutation.isPending;

  const {
    deferredSearchQuery,
    handleDirectoryScroll,
    hasReachedRecipientLimit,
    isDirectoryLoading,
    ownerProfiles,
    removeUser,
    searchError,
    searchQuery,
    searchResults,
    selectUser,
    selectedUsers,
    setSearchQuery,
  } = useNewMessageRecipients({ active: true, currentPubkey });

  const isSearchTransitionPending = searchQuery.trim() !== deferredSearchQuery;
  const visibleSearchResults =
    isSearchTransitionPending || isDirectoryLoading ? [] : searchResults;
  const showRecipientPicker = isRecipientPickerOpen && !isPending;
  const highlightedRecipientIndex = React.useMemo(() => {
    if (!showRecipientPicker || visibleSearchResults.length === 0) {
      return -1;
    }

    if (highlightedRecipientPubkey === null) {
      return 0;
    }

    return visibleSearchResults.findIndex(
      (user) => user.pubkey === highlightedRecipientPubkey,
    );
  }, [highlightedRecipientPubkey, showRecipientPicker, visibleSearchResults]);
  const highlightedRecipient =
    highlightedRecipientIndex < 0
      ? null
      : (visibleSearchResults[highlightedRecipientIndex] ?? null);

  React.useEffect(() => {
    if (
      highlightedRecipientPubkey &&
      !isSearchTransitionPending &&
      highlightedRecipientIndex < 0
    ) {
      setHighlightedRecipientPubkey(null);
    }
  }, [
    highlightedRecipientIndex,
    highlightedRecipientPubkey,
    isSearchTransitionPending,
  ]);

  React.useEffect(() => {
    if (!highlightedRecipient || !showRecipientPicker) {
      return;
    }

    document
      .getElementById(`new-dm-option-${highlightedRecipient.pubkey}`)
      ?.scrollIntoView({ block: "nearest" });
  }, [highlightedRecipient, showRecipientPicker]);

  React.useEffect(() => {
    isMountedRef.current = true;
    searchInputRef.current?.focus({ preventScroll: true });

    return () => {
      isMountedRef.current = false;
    };
  }, []);

  const handleRemoveUser = React.useCallback(
    (pubkey: string) => {
      preparedDirectMessageRef.current = null;
      setInspectedRecipientPubkey((current) =>
        current === pubkey ? null : current,
      );
      removeUser(pubkey);
    },
    [removeUser],
  );

  const handleSelectUser = React.useCallback(
    (user: Parameters<typeof selectUser>[0]) => {
      preparedDirectMessageRef.current = null;
      selectUser(user);
      setHighlightedRecipientPubkey(null);
      setSubmitErrorMessage(null);
      setIsRecipientPickerOpen(true);
      searchInputRef.current?.focus({ preventScroll: true });
    },
    [selectUser],
  );

  const handleResultSelect = React.useCallback(
    (user: Parameters<typeof selectUser>[0]) => {
      const isSelected = selectedUsers.some(
        (selectedUser) => selectedUser.pubkey === user.pubkey,
      );
      if (isSelected) {
        setSearchQuery("");
        setHighlightedRecipientPubkey(null);
        setSubmitErrorMessage(null);
        setIsRecipientPickerOpen(true);
        searchInputRef.current?.focus({ preventScroll: true });
        return;
      }

      handleSelectUser(user);
    },
    [handleSelectUser, selectedUsers, setSearchQuery],
  );

  const openDirectMessage = React.useCallback(
    async (additionalParticipantPubkeys: string[] = []) => {
      const requestedPubkeys = [
        ...new Set(
          [
            ...selectedUsers.map((user) => user.pubkey),
            ...additionalParticipantPubkeys,
          ].map(normalizePubkey),
        ),
      ].filter(Boolean);
      const preparedDirectMessage = preparedDirectMessageRef.current;
      const preparedParticipantPubkeys = new Set(
        preparedDirectMessage?.participantPubkeys.map(normalizePubkey) ?? [],
      );
      if (
        preparedDirectMessage &&
        requestedPubkeys.every((pubkey) =>
          preparedParticipantPubkeys.has(pubkey),
        )
      ) {
        return preparedDirectMessage;
      }

      if (
        openDmMutation.isPending ||
        sendMessageMutation.isPending ||
        requestedPubkeys.length === 0
      ) {
        return null;
      }

      setSubmitErrorMessage(null);

      try {
        const directMessage = await openDmMutation.mutateAsync({
          pubkeys: requestedPubkeys,
        });
        preparedDirectMessageRef.current = directMessage;
        return directMessage;
      } catch (error) {
        setSubmitErrorMessage(
          error instanceof Error
            ? error.message
            : "Failed to open direct message.",
        );
        return null;
      }
    },
    [
      openDmMutation.isPending,
      openDmMutation.mutateAsync,
      selectedUsers,
      sendMessageMutation.isPending,
    ],
  );

  const prepareSendChannel = React.useCallback(
    async (additionalParticipantPubkeys: string[] = []) => {
      const directMessage = await openDirectMessage(
        additionalParticipantPubkeys,
      );
      return directMessage?.id ?? null;
    },
    [openDirectMessage],
  );

  const sendFirstMessage = React.useCallback(
    async (
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
    ) => {
      const directMessage = await openDirectMessage();
      if (!directMessage) {
        throw new Error(
          submitErrorMessage ?? "Choose at least one recipient first.",
        );
      }

      if (!isMountedRef.current) {
        return;
      }

      try {
        await sendMessageMutation.mutateAsync({
          targetChannel: directMessage,
          content,
          mentionPubkeys,
          mediaTags,
        });
      } catch (error) {
        preparedDirectMessageRef.current = null;
        const message =
          error instanceof Error ? error.message : "Failed to send message.";
        setSubmitErrorMessage(message);
        throw error;
      }

      if (!isMountedRef.current) {
        return;
      }

      await upsertCachedChannel(directMessage);
      await goChannel(directMessage.id, { replace: true });
    },
    [
      goChannel,
      openDirectMessage,
      sendMessageMutation,
      submitErrorMessage,
      upsertCachedChannel,
    ],
  );

  const composerPlaceholder =
    selectedUsers.length === 0
      ? "Choose a recipient to start a message"
      : selectedUsers.length === 1
        ? `Message ${formatRecipientName(selectedUsers[0])}`
        : `Message ${selectedUsers.length} people`;

  return (
    <div
      className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden"
      data-testid="new-message-page"
    >
      <header
        className="relative z-40 shrink-0 cursor-default select-none border-b border-border/35 bg-background/80 px-5 py-2 backdrop-blur-md supports-backdrop-filter:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/55"
        data-testid="new-message-header"
        data-tauri-drag-region
      >
        <div className="flex min-h-9 min-w-0 items-center">
          <Popover
            onOpenChange={(open) => {
              setIsRecipientPickerOpen(
                open || inspectedRecipientPubkey !== null,
              );
            }}
            open={showRecipientPicker}
          >
            <PopoverAnchor asChild>
              {/* biome-ignore lint/a11y/noStaticElementInteractions: clicking anywhere in the recipient field focuses its input */}
              {/* biome-ignore lint/a11y/useKeyWithClickEvents: the nested combobox is the keyboard-accessible focus target */}
              <div
                className="group/to-field flex min-h-9 min-w-0 flex-1 cursor-text flex-wrap items-center gap-1.5 py-1"
                data-testid="new-message-to-field"
                onClick={() => {
                  setIsRecipientPickerOpen(true);
                  searchInputRef.current?.focus({ preventScroll: true });
                }}
                ref={toFieldRef}
              >
                <span className="shrink-0 text-base font-semibold tracking-tight">
                  To:
                </span>
                {selectedUsers.map((user) => (
                  <div
                    className="inline-flex h-7 max-w-56 items-center gap-1.5 rounded-full bg-muted px-1.5 pr-2.5 text-sm transition-colors hover:bg-muted/80"
                    key={user.pubkey}
                  >
                    <button
                      aria-label={`Remove ${formatRecipientName(user)}`}
                      className={cn(
                        "group/remove-recipient relative h-5 w-5 shrink-0 rounded-full focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60",
                        POOF_TRIGGER_CLASS,
                        POOF_ORIGIN_CLASS,
                      )}
                      data-testid={`new-dm-selected-${user.pubkey}`}
                      disabled={isPending}
                      onClick={(event) => {
                        event.stopPropagation();
                        handleRemoveUser(user.pubkey);
                      }}
                      onPointerDown={(event) => {
                        if (event.button !== 0) {
                          return;
                        }
                        event.stopPropagation();
                        handleRemoveUser(user.pubkey);
                      }}
                      type="button"
                    >
                      <ProfileAvatar
                        avatarUrl={user.avatarUrl}
                        className="h-5 w-5 text-3xs shadow-none transition-opacity group-hover/remove-recipient:opacity-0 group-focus-visible/remove-recipient:opacity-0"
                        iconClassName="h-2.5 w-2.5"
                        label={formatRecipientName(user)}
                      />
                      <span className="absolute inset-0 flex items-center justify-center rounded-full bg-foreground text-background opacity-0 transition-opacity group-hover/remove-recipient:opacity-100 group-focus-visible/remove-recipient:opacity-100">
                        <X aria-hidden="true" className="h-3 w-3" />
                      </span>
                    </button>
                    <Popover
                      onOpenChange={(open) => {
                        setInspectedRecipientPubkey(open ? user.pubkey : null);
                      }}
                      open={inspectedRecipientPubkey === user.pubkey}
                    >
                      <PopoverAnchor asChild>
                        <button
                          aria-expanded={
                            inspectedRecipientPubkey === user.pubkey
                          }
                          aria-haspopup="dialog"
                          aria-label={`Verify ${formatRecipientName(user)} public key`}
                          className="min-w-0 cursor-pointer truncate rounded font-medium hover:underline focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
                          data-testid={`new-dm-recipient-name-${user.pubkey}`}
                          onClick={(event) => {
                            event.stopPropagation();
                            setInspectedRecipientPubkey((current) =>
                              current === user.pubkey ? null : user.pubkey,
                            );
                          }}
                          type="button"
                        >
                          {formatRecipientName(user)}
                        </button>
                      </PopoverAnchor>
                      <PopoverContent
                        align="start"
                        className="w-96 max-w-[90vw] space-y-2"
                        data-new-dm-key-popover=""
                        data-testid={`new-dm-selected-key-popover-${user.pubkey}`}
                        onOpenAutoFocus={(event) => event.preventDefault()}
                      >
                        <p className="text-sm font-medium">
                          Verify {formatRecipientName(user)}
                        </p>
                        <PubKey
                          pubkey={user.pubkey}
                          testId={`new-dm-selected-pubkey-${user.pubkey}`}
                          variant="full"
                        />
                        <p className="break-all font-mono text-xs text-muted-foreground">
                          {user.pubkey}
                        </p>
                      </PopoverContent>
                    </Popover>
                    {user.isAgent ? (
                      <Bot
                        aria-label="agent"
                        className="h-3.5 w-3.5 shrink-0 text-muted-foreground"
                      />
                    ) : null}
                  </div>
                ))}
                <input
                  aria-activedescendant={
                    highlightedRecipient
                      ? `new-dm-option-${highlightedRecipient.pubkey}`
                      : undefined
                  }
                  aria-controls="new-dm-results"
                  aria-expanded={showRecipientPicker}
                  aria-label="To"
                  autoComplete="off"
                  autoCorrect="off"
                  className="h-7 min-w-32 flex-1 bg-transparent text-base outline-hidden placeholder:text-muted-foreground"
                  data-testid="new-dm-search"
                  disabled={isPending}
                  id="new-dm-search"
                  onChange={(event) => {
                    setSearchQuery(event.target.value);
                    setHighlightedRecipientPubkey(null);
                    setSubmitErrorMessage(null);
                    setIsRecipientPickerOpen(true);
                  }}
                  onFocus={() => setIsRecipientPickerOpen(true)}
                  onKeyDown={(event) => {
                    if (event.key === "Escape") {
                      event.preventDefault();
                      if (inspectedRecipientPubkey) {
                        setInspectedRecipientPubkey(null);
                        return;
                      }
                      setHighlightedRecipientPubkey(null);
                      setIsRecipientPickerOpen(false);
                      return;
                    }

                    if (
                      (event.key === "ArrowDown" || event.key === "ArrowUp") &&
                      visibleSearchResults.length > 0
                    ) {
                      event.preventDefault();
                      setIsRecipientPickerOpen(true);
                      setHighlightedRecipientPubkey(() => {
                        const current = highlightedRecipientIndex;
                        if (current < 0) {
                          const initialIndex =
                            event.key === "ArrowDown"
                              ? 0
                              : visibleSearchResults.length - 1;
                          return (
                            visibleSearchResults[initialIndex]?.pubkey ?? null
                          );
                        }

                        const direction = event.key === "ArrowDown" ? 1 : -1;
                        const nextIndex =
                          (current + direction + visibleSearchResults.length) %
                          visibleSearchResults.length;
                        return visibleSearchResults[nextIndex]?.pubkey ?? null;
                      });
                      return;
                    }

                    if (
                      event.key === "Backspace" &&
                      searchQuery.length === 0 &&
                      selectedUsers.length > 0
                    ) {
                      event.preventDefault();
                      const lastRecipient =
                        selectedUsers[selectedUsers.length - 1];
                      if (lastRecipient) {
                        document
                          .querySelector<HTMLElement>(
                            `[data-testid="new-dm-selected-${lastRecipient.pubkey}"]`,
                          )
                          ?.dispatchEvent(
                            new MouseEvent("click", {
                              bubbles: true,
                              detail: 0,
                            }),
                          );
                      }
                      return;
                    }

                    if (event.key !== "Enter") {
                      return;
                    }

                    if (searchQuery.trim().length === 0) {
                      if (highlightedRecipient) {
                        event.preventDefault();
                        handleSelectUser(highlightedRecipient);
                      }
                      return;
                    }

                    const keyboardSelection =
                      highlightedRecipient ??
                      getKeyboardSearchSelection({
                        currentQuery: searchQuery,
                        rankedQuery: deferredSearchQuery,
                        results: visibleSearchResults,
                      });
                    if (!keyboardSelection) {
                      return;
                    }

                    event.preventDefault();
                    handleResultSelect(keyboardSelection);
                  }}
                  ref={searchInputRef}
                  role="combobox"
                  spellCheck={false}
                  type="text"
                  value={searchQuery}
                />
              </div>
            </PopoverAnchor>
            <PopoverContent
              align="start"
              className="w-[min(36rem,calc(100vw-3rem))] overflow-hidden p-0"
              data-testid="new-message-recipient-popover"
              onCloseAutoFocus={(event) => event.preventDefault()}
              onInteractOutside={(event) => {
                const target = event.detail.originalEvent.target;
                if (
                  target instanceof Element &&
                  (toFieldRef.current?.contains(target) ||
                    Boolean(target.closest("[data-new-dm-key-popover]")))
                ) {
                  event.preventDefault();
                }
              }}
              onOpenAutoFocus={(event) => event.preventDefault()}
              side="bottom"
              sideOffset={6}
            >
              <div
                className="max-h-80 overflow-y-auto"
                data-testid="new-dm-results"
                id="new-dm-results"
                onScroll={handleDirectoryScroll}
                role="listbox"
              >
                {visibleSearchResults.length > 0 ? (
                  <div>
                    {visibleSearchResults.map((user) => {
                      const isSelected = selectedUsers.some(
                        (selectedUser) => selectedUser.pubkey === user.pubkey,
                      );
                      return (
                        <NewMessageResultRow
                          currentPubkey={currentPubkey}
                          disabled={
                            isPending ||
                            (hasReachedRecipientLimit && !isSelected)
                          }
                          isAlreadySelected={isSelected}
                          isKeyboardHighlighted={
                            highlightedRecipient?.pubkey === user.pubkey
                          }
                          key={user.pubkey}
                          onSelect={handleResultSelect}
                          ownerProfiles={ownerProfiles}
                          user={user}
                        />
                      );
                    })}
                  </div>
                ) : isDirectoryLoading || isSearchTransitionPending ? (
                  <div
                    aria-busy="true"
                    aria-label="Loading people and agents"
                    className="space-y-3 px-4 py-3"
                    data-testid="new-dm-loading"
                    role="status"
                  >
                    {["w-40", "w-32", "w-48"].map((nameWidth) => (
                      <div className="flex items-center gap-3" key={nameWidth}>
                        <Skeleton className="h-9 w-9 shrink-0 rounded-full" />
                        <div className="flex min-w-0 flex-1 flex-col gap-2">
                          <Skeleton className={`h-4 ${nameWidth}`} />
                          <Skeleton className="h-3 w-24" />
                        </div>
                      </div>
                    ))}
                  </div>
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
            </PopoverContent>
          </Popover>

          {isPending ? (
            <span
              className="shrink-0 pl-2 text-sm text-muted-foreground"
              data-testid="new-dm-opening"
            >
              Opening…
            </span>
          ) : null}
        </div>
      </header>

      <div
        className="min-h-0 flex-1 bg-background"
        data-testid="new-message-body"
      />

      {hasReachedRecipientLimit ? (
        <p
          className="px-5 pb-2 text-sm text-muted-foreground"
          data-testid="new-dm-limit"
        >
          DMs support up to nine people, including you.
        </p>
      ) : null}
      {searchError ? (
        <p className="px-5 pb-2 text-sm text-destructive">
          {searchError.message}
        </p>
      ) : null}
      {submitErrorMessage ? (
        <p className="px-5 pb-2 text-sm text-destructive">
          {submitErrorMessage}
        </p>
      ) : null}

      <MessageComposer
        channelName="new message"
        channelType="dm"
        containerClassName="px-5"
        disabled={isPending || selectedUsers.length === 0}
        isSending={isPending}
        onPrepareSendChannel={prepareSendChannel}
        onPreparingMentionSendChange={setIsPreparingMentionSend}
        onSend={sendFirstMessage}
        placeholder={composerPlaceholder}
      />
      <div aria-hidden="true" className="min-h-8 bg-background px-5 pb-1.5" />
    </div>
  );
}
