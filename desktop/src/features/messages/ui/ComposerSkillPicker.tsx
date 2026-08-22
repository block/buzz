import { Search, Zap } from "lucide-react";
import * as React from "react";

import type { ComposerAgentSkill } from "@/features/messages/lib/composerAgentSkills";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

type ComposerSkillPickerProps = {
  agentDisplayName: string;
  disabled: boolean;
  onClose: () => void;
  onSelect: (skill: ComposerAgentSkill) => void;
  onTriggerMouseDown: () => void;
  skills: readonly ComposerAgentSkill[];
};

export const ComposerSkillPicker = React.memo(function ComposerSkillPicker({
  agentDisplayName,
  disabled,
  onClose,
  onSelect,
  onTriggerMouseDown,
  skills,
}: ComposerSkillPickerProps) {
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const [highlightedIndex, setHighlightedIndex] = React.useState(0);

  const filteredSkills = React.useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase();
    if (!normalizedQuery) return skills;
    return skills.filter(
      (skill) =>
        skill.name.toLowerCase().includes(normalizedQuery) ||
        skill.description.toLowerCase().includes(normalizedQuery),
    );
  }, [query, skills]);

  const activeHighlightedIndex = Math.min(
    highlightedIndex,
    Math.max(filteredSkills.length - 1, 0),
  );

  const handleOpenChange = React.useCallback(
    (nextOpen: boolean) => {
      setOpen(nextOpen);
      if (!nextOpen) {
        setQuery("");
        setHighlightedIndex(0);
        requestAnimationFrame(onClose);
      }
    },
    [onClose],
  );

  const selectSkill = React.useCallback(
    (skill: ComposerAgentSkill) => {
      onSelect(skill);
      handleOpenChange(false);
    },
    [handleOpenChange, onSelect],
  );

  const handleKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLInputElement>) => {
      if (event.key === "ArrowDown" && filteredSkills.length > 0) {
        event.preventDefault();
        setHighlightedIndex((current) =>
          Math.min(current + 1, filteredSkills.length - 1),
        );
      } else if (event.key === "ArrowUp" && filteredSkills.length > 0) {
        event.preventDefault();
        setHighlightedIndex((current) => Math.max(current - 1, 0));
      } else if (event.key === "Enter") {
        const skill = filteredSkills[activeHighlightedIndex];
        if (skill) {
          event.preventDefault();
          selectSkill(skill);
        }
      } else if (event.key === "Escape") {
        event.preventDefault();
        handleOpenChange(false);
      }
    },
    [activeHighlightedIndex, filteredSkills, handleOpenChange, selectSkill],
  );

  if (skills.length === 0) return null;

  return (
    <Popover onOpenChange={handleOpenChange} open={open}>
      <Tooltip disableHoverableContent>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <Button
              aria-label={`Insert a skill for ${agentDisplayName}`}
              aria-expanded={open}
              data-testid="composer-skill-picker"
              disabled={disabled}
              onMouseDown={onTriggerMouseDown}
              size="icon"
              type="button"
              variant="ghost"
            >
              <Zap />
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent>Insert skill</TooltipContent>
      </Tooltip>
      <PopoverContent
        align="start"
        className="w-80 overflow-hidden p-0"
        onCloseAutoFocus={(event) => event.preventDefault()}
        onOpenAutoFocus={(event) => event.preventDefault()}
        side="top"
        sideOffset={10}
      >
        <div className="border-b border-border/50 px-3 py-2.5">
          <p className="text-sm font-medium">Skills for {agentDisplayName}</p>
        </div>
        <div className="flex items-center gap-2 border-b border-border/50 px-3 py-2">
          <Search className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          <input
            aria-label="Search skills"
            autoCapitalize="none"
            autoComplete="off"
            autoCorrect="off"
            className="min-w-0 flex-1 border-0 bg-transparent p-0 text-sm outline-none placeholder:text-muted-foreground"
            onChange={(event) => {
              setQuery(event.target.value);
              setHighlightedIndex(0);
            }}
            onKeyDown={handleKeyDown}
            placeholder="Search skills…"
            ref={(element) => element?.focus()}
            spellCheck={false}
            value={query}
          />
        </div>
        <div
          aria-label={`Available skills for ${agentDisplayName}`}
          className="max-h-64 overflow-y-auto overscroll-contain p-1"
          onTouchMoveCapture={(event) => event.stopPropagation()}
          onWheelCapture={(event) => event.stopPropagation()}
          role="listbox"
        >
          {filteredSkills.length > 0 ? (
            filteredSkills.map((skill, index) => (
              <button
                aria-selected={index === activeHighlightedIndex}
                className={cn(
                  "flex w-full flex-col rounded-lg px-3 py-2 text-left outline-none transition-colors",
                  index === activeHighlightedIndex
                    ? "bg-muted/70 text-foreground"
                    : "hover:bg-muted/50",
                )}
                key={skill.name}
                onClick={() => selectSkill(skill)}
                onMouseEnter={() => setHighlightedIndex(index)}
                role="option"
                type="button"
              >
                <span className="text-sm font-medium">/{skill.name}</span>
                {skill.description ? (
                  <span className="line-clamp-2 text-xs text-muted-foreground">
                    {skill.description}
                  </span>
                ) : null}
              </button>
            ))
          ) : (
            <p className="px-3 py-6 text-center text-sm text-muted-foreground">
              No skills match
            </p>
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
});
