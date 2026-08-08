import { X } from "lucide-react";
import { useState } from "react";

import { useReplyMentionSettings } from "@/features/messages/useReplyMentionSettings";
import { parsePubkeyInput } from "@/shared/lib/nostrUtils";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { PubKey } from "@/shared/ui/PubKey";
import { Switch } from "@/shared/ui/switch";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

/**
 * Per-account "reply auto-mention" preferences, backed by
 * `useReplyMentionSettings` (localStorage; see
 * features/messages/lib/replyMentionSettings.ts). The message send path reads
 * the same storage at send time, so changes here apply to the next reply.
 */
export function ReplyMentionSettingsCard({
  currentPubkey,
}: {
  currentPubkey?: string;
}) {
  const { settings, setAutoMentionRepliedTo, setMentionPrefixPubkeys } =
    useReplyMentionSettings(currentPubkey);
  const [prefixInput, setPrefixInput] = useState("");

  const parsedPrefix = parsePubkeyInput(prefixInput);
  const alreadyAdded =
    parsedPrefix != null &&
    settings.mentionPrefixPubkeys.includes(parsedPrefix);
  const showInvalidInput =
    prefixInput.trim().length > 0 && parsedPrefix == null;

  const addPrefix = () => {
    if (!parsedPrefix || alreadyAdded) {
      return;
    }
    setMentionPrefixPubkeys([...settings.mentionPrefixPubkeys, parsedPrefix]);
    setPrefixInput("");
  };

  const removePrefix = (pubkey: string) => {
    setMentionPrefixPubkeys(
      settings.mentionPrefixPubkeys.filter((entry) => entry !== pubkey),
    );
  };

  return (
    <section className="min-w-0" data-testid="settings-reply-mentions">
      <SettingsSectionHeader
        title="Reply mentions"
        description="Control who gets woken when you reply in a thread."
      />

      <div className="flex flex-col gap-4">
        <SettingsOptionGroup>
          <SettingsOptionRow>
            <div className="min-w-0">
              <label
                className="text-sm font-medium"
                htmlFor="reply-auto-mention-switch"
              >
                Mention the author you reply to
              </label>
              <p className="text-sm font-normal text-muted-foreground">
                Replying folds the replied-to author into the reply's mentions
                so they get notified, even without a literal @mention. Turn off
                to reply without waking them.
              </p>
            </div>
            <Switch
              checked={settings.autoMentionRepliedTo}
              data-testid="reply-auto-mention-toggle"
              id="reply-auto-mention-switch"
              onCheckedChange={(checked) => {
                setAutoMentionRepliedTo(checked);
              }}
            />
          </SettingsOptionRow>
        </SettingsOptionGroup>

        <SettingsOptionGroup>
          <SettingsOptionRow className="items-start">
            <div className="min-w-0">
              <span className="text-sm font-medium">Always mention</span>
              <p className="text-sm font-normal text-muted-foreground">
                These accounts are folded into every reply you send — e.g. a
                teammate or agent group that should wake on each reply.
              </p>
            </div>
          </SettingsOptionRow>

          {settings.mentionPrefixPubkeys.map((pubkey) => (
            <SettingsOptionRow key={pubkey} className="min-h-12 py-2">
              <PubKey
                className="text-sm"
                pubkey={pubkey}
                testId={`reply-mention-prefix-${pubkey.slice(0, 8)}`}
              />
              <Button
                aria-label="Remove always-mention entry"
                data-testid={`reply-mention-prefix-remove-${pubkey.slice(0, 8)}`}
                onClick={() => removePrefix(pubkey)}
                size="icon-xs"
                type="button"
                variant="ghost"
              >
                <X />
              </Button>
            </SettingsOptionRow>
          ))}

          <SettingsOptionRow className="items-start">
            <div className="flex w-full flex-col gap-2">
              <div className="flex items-center gap-2">
                <Input
                  aria-label="Add always-mention pubkey"
                  data-testid="reply-mention-prefix-input"
                  onChange={(event) => setPrefixInput(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      addPrefix();
                    }
                  }}
                  placeholder="hex pubkey or npub1…"
                  value={prefixInput}
                />
                <Button
                  data-testid="reply-mention-prefix-add"
                  disabled={!parsedPrefix || alreadyAdded}
                  onClick={addPrefix}
                  size="sm"
                  type="button"
                  variant="secondary"
                >
                  Add
                </Button>
              </div>
              {showInvalidInput ? (
                <p className="text-sm text-destructive">
                  Enter a 64-character hex pubkey or an npub1… address.
                </p>
              ) : alreadyAdded ? (
                <p className="text-sm text-muted-foreground">
                  Already in the list.
                </p>
              ) : null}
            </div>
          </SettingsOptionRow>
        </SettingsOptionGroup>
      </div>
    </section>
  );
}
