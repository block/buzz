import * as React from "react";
import { ChevronDown, Search, X } from "lucide-react";
import {
  mergeAllowlist,
  parsePubkeyInput,
} from "@/features/agents/lib/respondToAllowlist";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { PubKey } from "@/shared/ui/PubKey";
import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import { useUserSearchQuery } from "@/features/profile/hooks";
import type { RespondToMode, UserSearchResult } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { PersonaDropdownField } from "./PersonaDropdownField";
import type { PersonaDropdownOption } from "./personaDialogPickers";

/**
 * Inbound author gate UI for create/edit agent dialogs.
 *
 * Dropdown:
 *   - Owner only  (default; matches `buzz-acp --respond-to=owner-only`)
 *   - Anyone      (`--respond-to=anyone` — fully open bot)
 *   - Allowlist   (`--respond-to=allowlist`, plus the chip list as
 *                  `--respond-to-allowlist`)
 *
 * `nobody` is intentionally not surfaced — it pairs with a heartbeat-only
 * setup that has no meaningful GUI use case.
 *
 * Validation is duplicated lightly here for inline UX feedback only; the
 * authoritative validator is `validate_respond_to_allowlist` in
 * `desktop/src-tauri/src/managed_agents/types.rs`.
 */

function formatSearchUserName(user: UserSearchResult) {
  return (
    user.displayName?.trim() ||
    user.nip05Handle?.trim() ||
    truncatePubkey(user.pubkey)
  );
}

function formatSearchUserSecondary(user: UserSearchResult) {
  const displayName = user.displayName?.trim();
  const nip05Handle = user.nip05Handle?.trim();
  if (displayName && nip05Handle) {
    return nip05Handle;
  }
  return truncatePubkey(user.pubkey);
}

const RESPOND_TO_OPTIONS: PersonaDropdownOption[] = [
  { label: "Only me (default)", value: "owner-only" },
  { label: "Anyone", value: "anyone" },
  { label: "Allowlist", value: "allowlist" },
];

export function CreateAgentRespondToField({
  mode,
  allowlist,
  onModeChange,
  onAllowlistChange,
  ownerPubkey,
  disabled,
  variant,
}: {
  mode: RespondToMode;
  allowlist: string[];
  onModeChange: (mode: RespondToMode) => void;
  onAllowlistChange: (allowlist: string[]) => void;
  /**
   * The agent's own owner pubkey, when known (Edit dialog). The owner is
   * always implicitly included by the harness — surfaced here as a small
   * informational note so it doesn't look like a missing entry.
   */
  ownerPubkey?: string | null;
  disabled?: boolean;
  /** When "persona", uses PersonaDropdownField styling to match the persona dialog. */
  variant?: "default" | "persona";
}) {
  const [query, setQuery] = React.useState("");
  const [isDirectEntryOpen, setIsDirectEntryOpen] = React.useState(false);
  const [pasteText, setPasteText] = React.useState("");

  const deferredQuery = React.useDeferredValue(query.trim());
  const allowlistSet = React.useMemo(
    () => new Set(allowlist.map((p) => p.toLowerCase())),
    [allowlist],
  );
  const userSearchQuery = useUserSearchQuery(deferredQuery, {
    enabled: mode === "allowlist" && deferredQuery.length > 0,
    limit: 8,
  });
  const isArchivedDiscovery = useIsArchivedPredicate();
  const searchResults = React.useMemo(
    () =>
      (userSearchQuery.data ?? []).filter(
        (user) =>
          !allowlistSet.has(user.pubkey.toLowerCase()) &&
          !isArchivedDiscovery(user.pubkey),
      ),
    [allowlistSet, isArchivedDiscovery, userSearchQuery.data],
  );

  const pasteParsed = React.useMemo(
    () => parsePubkeyInput(pasteText),
    [pasteText],
  );

  function handleAddSearchResult(user: UserSearchResult) {
    onAllowlistChange(mergeAllowlist(allowlist, [user.pubkey]));
    setQuery("");
  }

  function handleAddRawPubkey(pubkey: string) {
    onAllowlistChange(mergeAllowlist(allowlist, [pubkey]));
    setQuery("");
  }

  function handleRemove(pubkey: string) {
    onAllowlistChange(
      allowlist.filter((p) => p.toLowerCase() !== pubkey.toLowerCase()),
    );
  }

  function handleAddFromPaste() {
    if (pasteParsed.valid.length === 0) return;
    onAllowlistChange(mergeAllowlist(allowlist, pasteParsed.valid));
    setPasteText("");
  }

  const isPersonaVariant = variant === "persona";

  return (
    <div className="space-y-2" data-testid="agent-respond-to">
      <label
        className={
          isPersonaVariant
            ? "text-sm font-medium text-foreground"
            : "text-sm font-medium"
        }
        htmlFor="agent-respond-to"
      >
        Who can talk to this agent
      </label>
      {isPersonaVariant ? (
        <PersonaDropdownField
          disabled={disabled}
          id="agent-respond-to"
          onValueChange={(value) => onModeChange(value as RespondToMode)}
          options={RESPOND_TO_OPTIONS}
          placeholder="Only me (default)"
          value={mode}
        />
      ) : (
        <select
          className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
          data-testid="agent-respond-to-select"
          disabled={disabled}
          id="agent-respond-to"
          onChange={(e) => onModeChange(e.target.value as RespondToMode)}
          value={mode}
        >
          <option value="owner-only">Owner only (default)</option>
          <option value="anyone">Anyone</option>
          <option value="allowlist">Allowlist</option>
        </select>
      )}
      {!isPersonaVariant ? (
        <p className="text-xs text-muted-foreground">
          Controls which Nostr authors the agent listens to (@mentions, DMs,
          thread replies). The agent&apos;s owner can always shut it down with
          <span className="font-mono"> !shutdown</span>.
        </p>
      ) : null}
      {mode === "allowlist" ? (
        <AllowlistPicker
          allowlist={allowlist}
          deferredQuery={deferredQuery}
          disabled={disabled}
          isDirectEntryOpen={isDirectEntryOpen}
          onAddFromPaste={handleAddFromPaste}
          onAddRawPubkey={handleAddRawPubkey}
          onAddSearchResult={handleAddSearchResult}
          onPasteTextChange={setPasteText}
          onQueryChange={setQuery}
          onRemove={handleRemove}
          onToggleDirectEntry={() => setIsDirectEntryOpen((v) => !v)}
          ownerPubkey={ownerPubkey ?? null}
          pasteInvalid={pasteParsed.invalid}
          pasteText={pasteText}
          pasteValidCount={pasteParsed.valid.length}
          query={query}
          searchError={
            userSearchQuery.error instanceof Error
              ? userSearchQuery.error.message
              : null
          }
          searchIsLoading={userSearchQuery.isLoading}
          searchResults={searchResults}
          variant={isPersonaVariant ? "persona" : "default"}
        />
      ) : null}
    </div>
  );
}

const HEX_64_RE = /^[0-9a-f]{64}$/i;

function AllowlistPicker({
  allowlist,
  deferredQuery,
  disabled,
  isDirectEntryOpen,
  onAddFromPaste,
  onAddRawPubkey,
  onAddSearchResult,
  onPasteTextChange,
  onQueryChange,
  onRemove,
  onToggleDirectEntry,
  ownerPubkey,
  pasteInvalid,
  pasteText,
  pasteValidCount,
  query,
  searchError,
  searchIsLoading,
  searchResults,
  variant = "default",
}: {
  allowlist: string[];
  deferredQuery: string;
  disabled?: boolean;
  isDirectEntryOpen: boolean;
  onAddFromPaste: () => void;
  onAddRawPubkey: (pubkey: string) => void;
  onAddSearchResult: (user: UserSearchResult) => void;
  onPasteTextChange: (value: string) => void;
  onQueryChange: (value: string) => void;
  onRemove: (pubkey: string) => void;
  onToggleDirectEntry: () => void;
  ownerPubkey: string | null;
  pasteInvalid: string[];
  pasteText: string;
  pasteValidCount: number;
  query: string;
  searchError: string | null;
  searchIsLoading: boolean;
  searchResults: UserSearchResult[];
  variant?: "default" | "persona";
}) {
  const isPersona = variant === "persona";

  // Detect if the query is a valid hex pubkey that's not already in the list.
  const queryIsHexPubkey =
    HEX_64_RE.test(deferredQuery) &&
    !allowlist.some((p) => p.toLowerCase() === deferredQuery.toLowerCase());

  return (
    <div
      className={
        isPersona
          ? "space-y-2.5"
          : "space-y-2.5 rounded-xl border border-border/80 bg-muted/15 p-3"
      }
      data-testid="agent-respond-to-allowlist"
    >
      {!isPersona ? (
        <div className="flex items-center justify-between gap-2">
          <span className="text-sm font-medium">Allowed pubkeys</span>
          <span className="rounded-full bg-background px-2 py-1 text-2xs font-medium leading-none text-muted-foreground">
            {allowlist.length} selected
          </span>
        </div>
      ) : null}
      {!isPersona && ownerPubkey ? (
        <p className="text-xs text-muted-foreground">
          Owner (
          <PubKey pubkey={ownerPubkey} />) is always implicitly allowed by the
          harness — no need to add it here.
        </p>
      ) : !isPersona ? (
        <p className="text-xs text-muted-foreground">
          The agent&apos;s owner is always implicitly allowed.
        </p>
      ) : null}
      <div className="rounded-lg border border-border/80 bg-background">
        <div className="flex items-center gap-2 px-2.5 py-2">
          <Search className="h-4 w-4 text-muted-foreground" />
          <Input
            className="h-auto border-0 px-0 py-0 shadow-none focus-visible:ring-0"
            data-testid="agent-respond-to-search"
            disabled={disabled}
            onChange={(event) => onQueryChange(event.target.value)}
            placeholder={
              isPersona ? "Search people" : "Search by name or NIP-05."
            }
            value={query}
          />
        </div>
        {allowlist.length > 0 ? (
          <div className="flex flex-wrap gap-1.5 border-t border-border/70 px-2.5 py-2">
            {allowlist.map((pubkey) => (
              <div
                className="inline-flex items-center gap-1.5 rounded-full border border-border/80 bg-muted/60 px-2.5 py-1 text-2xs leading-none"
                data-testid={`agent-respond-to-chip-${pubkey}`}
                key={pubkey}
              >
                <UserAvatar
                  avatarUrl={null}
                  displayName={truncatePubkey(pubkey)}
                  size="xs"
                />
                <PubKey pubkey={pubkey} />
                <button
                  aria-label={`Remove ${truncatePubkey(pubkey)}`}
                  className="text-muted-foreground transition-colors hover:text-foreground"
                  disabled={disabled}
                  onClick={() => onRemove(pubkey)}
                  type="button"
                >
                  <X className="h-4 w-4" />
                </button>
              </div>
            ))}
          </div>
        ) : null}
        {deferredQuery.length > 0 ? (
          <div className="border-t border-border/70 px-2 py-2">
            {searchIsLoading ? (
              <p className="px-2 py-1 text-sm text-muted-foreground">
                Searching…
              </p>
            ) : searchResults.length > 0 ? (
              <div className="max-h-44 space-y-1 overflow-y-auto">
                {searchResults.map((result) => (
                  <button
                    className="flex w-full items-center justify-between rounded-md px-2.5 py-1.5 text-left transition-colors hover:bg-accent hover:text-accent-foreground"
                    data-testid={`agent-respond-to-result-${result.pubkey}`}
                    key={result.pubkey}
                    onClick={() => onAddSearchResult(result)}
                    type="button"
                  >
                    <div className="flex items-center gap-2 min-w-0">
                      <UserAvatar
                        avatarUrl={result.avatarUrl}
                        displayName={formatSearchUserName(result)}
                        size="xs"
                      />
                      <div className="min-w-0">
                        <p className="truncate text-sm font-medium leading-5">
                          {formatSearchUserName(result)}
                        </p>
                        <p className="truncate text-xs text-muted-foreground">
                          {formatSearchUserSecondary(result)}
                        </p>
                      </div>
                    </div>
                    <span className="text-xs text-muted-foreground">Add</span>
                  </button>
                ))}
              </div>
            ) : queryIsHexPubkey ? (
              <button
                className="flex w-full items-center justify-between rounded-md px-2.5 py-1.5 text-left transition-colors hover:bg-accent hover:text-accent-foreground"
                data-testid="agent-respond-to-add-raw-pubkey"
                onClick={() => onAddRawPubkey(deferredQuery.toLowerCase())}
                type="button"
              >
                <div className="flex items-center gap-2 min-w-0">
                  <UserAvatar
                    avatarUrl={null}
                    displayName={truncatePubkey(deferredQuery)}
                    size="xs"
                  />
                  <div className="min-w-0">
                    <p className="truncate text-sm font-medium leading-5">
                      {truncatePubkey(deferredQuery)}
                    </p>
                    <p className="truncate text-xs text-muted-foreground">
                      Add pubkey directly
                    </p>
                  </div>
                </div>
                <span className="text-xs text-muted-foreground">Add</span>
              </button>
            ) : (
              <p className="px-2 py-1 text-sm text-muted-foreground">
                No matching users.
              </p>
            )}
          </div>
        ) : null}
      </div>
      {searchError ? (
        <p className="text-sm text-destructive">{searchError}</p>
      ) : null}
      {!isPersona ? (
        <div className="space-y-2">
          <button
            aria-controls="agent-respond-to-direct-panel"
            aria-expanded={isDirectEntryOpen}
            className="inline-flex items-center gap-1.5 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
            data-testid="agent-respond-to-toggle-direct"
            onClick={onToggleDirectEntry}
            type="button"
          >
            <ChevronDown
              className={cn(
                "h-4 w-4 transition-transform",
                isDirectEntryOpen && "rotate-180",
              )}
            />
            <span>Paste pubkeys</span>
          </button>
          {isDirectEntryOpen ? (
            <div
              className="space-y-2 rounded-lg border border-dashed border-border/80 bg-background/70 p-2.5"
              id="agent-respond-to-direct-panel"
            >
              <p className="text-xs text-muted-foreground">
                One per line, or comma/space-separated. 64-char lowercase hex
                only — npub decoding is not yet supported here.
              </p>
              <Textarea
                className="min-h-20 font-mono text-xs"
                data-testid="agent-respond-to-paste"
                disabled={disabled}
                onChange={(event) => onPasteTextChange(event.target.value)}
                placeholder="abcdef0123…"
                value={pasteText}
              />
              {pasteInvalid.length > 0 ? (
                <p className="text-xs text-destructive">
                  {pasteInvalid.length} entr
                  {pasteInvalid.length === 1 ? "y is" : "ies are"} not 64-char
                  hex and will be ignored.
                </p>
              ) : null}
              <div className="flex items-center justify-between gap-2">
                <span className="text-xs text-muted-foreground">
                  {pasteValidCount > 0
                    ? `${pasteValidCount} valid pubkey${pasteValidCount === 1 ? "" : "s"} ready.`
                    : "No valid pubkeys yet."}
                </span>
                <button
                  className="rounded-md border border-border/80 bg-background px-2.5 py-1 text-xs font-medium transition-colors hover:bg-accent hover:text-accent-foreground disabled:cursor-not-allowed disabled:opacity-50"
                  data-testid="agent-respond-to-paste-add"
                  disabled={disabled || pasteValidCount === 0}
                  onClick={onAddFromPaste}
                  type="button"
                >
                  Add to allowlist
                </button>
              </div>
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
