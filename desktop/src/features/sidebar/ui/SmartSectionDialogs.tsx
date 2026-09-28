import { CaseSensitive, Hash, Regex, Search } from "lucide-react";
import * as React from "react";

import type {
  ChannelSection,
  SmartSectionRule,
} from "@/features/sidebar/lib/channelSectionsStorage";
import {
  getSmartSectionRuleError,
  matchingChannelsForSection,
} from "@/features/sidebar/lib/smartChannelSections";
import type { Channel } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Toggle } from "@/shared/ui/toggle";

export type SmartSectionDialogValue = {
  name: string;
  smartRule: SmartSectionRule;
};

export function SmartSectionDialog({
  open,
  onOpenChange,
  section,
  onConfirm,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  section?: ChannelSection | null;
  onConfirm: (value: SmartSectionDialogValue) => void;
}) {
  const [name, setName] = React.useState("");
  const [pattern, setPattern] = React.useState("");
  const [isRegex, setIsRegex] = React.useState(false);
  const [caseSensitive, setCaseSensitive] = React.useState(false);
  const nameInputRef = React.useRef<HTMLInputElement>(null);
  const isEditing = Boolean(section);
  const smartRule = React.useMemo(
    () => ({ pattern, isRegex, caseSensitive }),
    [pattern, isRegex, caseSensitive],
  );
  const ruleError = getSmartSectionRuleError(smartRule);

  React.useEffect(() => {
    if (!open) return;
    setName(section?.name ?? "");
    setPattern(section?.smartRule?.pattern ?? "");
    setIsRegex(section?.smartRule?.isRegex ?? false);
    setCaseSensitive(section?.smartRule?.caseSensitive ?? false);
    const timerId = globalThis.setTimeout(
      () => nameInputRef.current?.focus(),
      50,
    );
    return () => globalThis.clearTimeout(timerId);
  }, [open, section]);

  function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmedName = name.trim();
    const trimmedPattern = pattern.trim();
    const nextRule = { ...smartRule, pattern: trimmedPattern };
    if (!trimmedName || getSmartSectionRuleError(nextRule)) return;
    onConfirm({ name: trimmedName, smartRule: nextRule });
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>
            {isEditing ? "Edit smart section" : "Create smart section"}
          </DialogTitle>
          <DialogDescription>
            New matching channels are placed here automatically. Manual moves
            always take precedence.
          </DialogDescription>
        </DialogHeader>
        <form className="space-y-4" onSubmit={handleSubmit}>
          <div className="space-y-2">
            <label className="text-sm font-medium" htmlFor="smart-section-name">
              Section name
            </label>
            <Input
              autoComplete="off"
              id="smart-section-name"
              onChange={(event) => setName(event.target.value)}
              placeholder="Engineering"
              ref={nameInputRef}
              value={name}
            />
          </div>
          <div className="space-y-2">
            <label
              className="text-sm font-medium"
              htmlFor="smart-section-pattern"
            >
              Channel name pattern
            </label>
            <div className="relative">
              <Input
                aria-describedby="smart-section-pattern-help"
                aria-invalid={Boolean(ruleError)}
                autoCapitalize="none"
                autoComplete="off"
                autoCorrect="off"
                className="pr-20"
                id="smart-section-pattern"
                onChange={(event) => setPattern(event.target.value)}
                placeholder={isRegex ? "^eng-.*" : "eng-"}
                spellCheck={false}
                value={pattern}
              />
              <div className="absolute inset-y-0 right-1 flex items-center gap-0.5">
                <Toggle
                  aria-label="Use regular expression"
                  className="h-7 min-w-7 rounded-md px-1.5"
                  data-testid="smart-section-regex-toggle"
                  onPressedChange={setIsRegex}
                  pressed={isRegex}
                  size="sm"
                  title="Regular expression"
                  type="button"
                  variant="ghost"
                >
                  <Regex />
                </Toggle>
                <Toggle
                  aria-label="Match case"
                  className="h-7 min-w-7 rounded-md px-1.5"
                  data-testid="smart-section-case-toggle"
                  onPressedChange={setCaseSensitive}
                  pressed={caseSensitive}
                  size="sm"
                  title="Case sensitive"
                  type="button"
                  variant="ghost"
                >
                  <CaseSensitive />
                </Toggle>
              </div>
            </div>
            <p
              className={
                ruleError
                  ? "text-xs text-destructive"
                  : "text-xs text-muted-foreground"
              }
              id="smart-section-pattern-help"
            >
              {ruleError ??
                (isRegex
                  ? "Matches channel names using a regular expression."
                  : "Matches channel names containing this text.")}
            </p>
          </div>
          <DialogFooter>
            <DialogClose asChild>
              <Button type="button" variant="ghost">
                Cancel
              </Button>
            </DialogClose>
            <Button disabled={!name.trim() || Boolean(ruleError)} type="submit">
              {isEditing ? "Save" : "Create"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function SmartSectionMatchesDialog({
  open,
  onOpenChange,
  section,
  sections,
  channels,
  assignments,
  onConfirm,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  section: ChannelSection | null;
  sections: ChannelSection[];
  channels: Channel[];
  assignments: Record<string, string>;
  onConfirm: (channelIds: string[]) => void;
}) {
  const matches = React.useMemo(
    () => (section ? matchingChannelsForSection(section, channels) : []),
    [section, channels],
  );
  const movableIds = React.useMemo(
    () =>
      matches
        .filter((channel) => assignments[channel.id] !== section?.id)
        .map((channel) => channel.id),
    [matches, assignments, section?.id],
  );
  const [selectedIds, setSelectedIds] = React.useState<Set<string>>(new Set());
  const sectionNames = React.useMemo(
    () => new Map(sections.map((candidate) => [candidate.id, candidate.name])),
    [sections],
  );

  React.useEffect(() => {
    if (open) setSelectedIds(new Set(movableIds));
  }, [open, movableIds]);

  function toggleChannel(channelId: string, checked: boolean) {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (checked) next.add(channelId);
      else next.delete(channelId);
      return next;
    });
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Matching channels</DialogTitle>
          <DialogDescription>
            Review every channel matching{" "}
            {section?.name ?? "this smart section"}
            {section?.smartRule ? ` (${section.smartRule.pattern})` : ""}.
            Selected channels will be moved into the section.
          </DialogDescription>
        </DialogHeader>
        {matches.length > 0 ? (
          <>
            <div className="flex items-center justify-between text-xs text-muted-foreground">
              <span>{matches.length} matching channels</span>
              {movableIds.length > 0 ? (
                <button
                  className="font-medium text-foreground hover:underline"
                  onClick={() =>
                    setSelectedIds(
                      selectedIds.size === movableIds.length
                        ? new Set()
                        : new Set(movableIds),
                    )
                  }
                  type="button"
                >
                  {selectedIds.size === movableIds.length
                    ? "Clear selection"
                    : "Select all"}
                </button>
              ) : null}
            </div>
            <div className="max-h-72 space-y-1 overflow-y-auto rounded-lg border border-border p-1">
              {matches.map((channel) => {
                const currentSectionId = assignments[channel.id];
                const alreadyInSection = currentSectionId === section?.id;
                const location = alreadyInSection
                  ? "Already in section"
                  : currentSectionId
                    ? (sectionNames.get(currentSectionId) ?? "Another section")
                    : "Channels";
                return (
                  <label
                    className="flex cursor-pointer items-center gap-3 rounded-md px-2 py-2 hover:bg-muted/70"
                    htmlFor={`smart-section-match-${channel.id}`}
                    key={channel.id}
                  >
                    <Checkbox
                      checked={alreadyInSection || selectedIds.has(channel.id)}
                      disabled={alreadyInSection}
                      id={`smart-section-match-${channel.id}`}
                      onCheckedChange={(checked) =>
                        toggleChannel(channel.id, checked === true)
                      }
                    />
                    <Hash className="h-4 w-4 shrink-0 text-muted-foreground" />
                    <span className="min-w-0 flex-1 truncate text-sm">
                      {channel.name}
                    </span>
                    <span className="shrink-0 text-xs text-muted-foreground">
                      {location}
                    </span>
                  </label>
                );
              })}
            </div>
          </>
        ) : (
          <div className="flex flex-col items-center gap-2 rounded-lg border border-dashed border-border py-10 text-center">
            <Search className="h-6 w-6 text-muted-foreground" />
            <p className="text-sm font-medium">No matching channels</p>
            <p className="text-xs text-muted-foreground">
              Edit the smart section to broaden its pattern.
            </p>
          </div>
        )}
        <DialogFooter>
          <DialogClose asChild>
            <Button type="button" variant="ghost">
              Cancel
            </Button>
          </DialogClose>
          <Button
            disabled={selectedIds.size === 0}
            onClick={() => onConfirm(Array.from(selectedIds))}
            type="button"
          >
            Move {selectedIds.size}{" "}
            {selectedIds.size === 1 ? "channel" : "channels"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
