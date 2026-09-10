import {
  IconBold,
  IconCode,
  IconItalic,
  IconLink,
  IconList,
  IconListNumbers,
  IconQuote,
  IconStrikethrough,
  IconX,
} from "@tabler/icons-react";
import { type AnimationEvent, useEffect, useRef } from "react";
import { IconButton } from "@/shared/ui/IconButton";

export type ComposerFormat =
  | "bold"
  | "italic"
  | "strike"
  | "code"
  | "quote"
  | "bullet"
  | "number"
  | "link";

const COMMANDS = [
  { format: "bold", label: "Bold", icon: IconBold },
  { format: "italic", label: "Italic", icon: IconItalic },
  { format: "strike", label: "Strikethrough", icon: IconStrikethrough },
  { format: "code", label: "Code", icon: IconCode },
  { format: "quote", label: "Quote", icon: IconQuote },
  { format: "bullet", label: "Bulleted list", icon: IconList },
  { format: "number", label: "Numbered list", icon: IconListNumbers },
  { format: "link", label: "Link", icon: IconLink },
] as const;

/** Composer-owned commands. The editor owns selection, active state, and edits. */
export function ComposerFormattingBar({
  activeFormats = [],
  className,
  disabled,
  onFormat,
  onCommandPointerDown,
  onClose,
  onAnimationEnd,
  focusOnMount = false,
}: {
  activeFormats?: readonly ComposerFormat[];
  className?: string;
  focusOnMount?: boolean;
  onAnimationEnd?: (event: AnimationEvent<HTMLFieldSetElement>) => void;
  disabled?: boolean;
  onFormat?: (format: ComposerFormat) => void;
  onCommandPointerDown?: () => void;
  onClose: () => void;
}) {
  const closeButton = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (focusOnMount) closeButton.current?.focus();
  }, [focusOnMount]);
  return (
    <fieldset
      className={`composer-formatting-bar ${className ?? ""}`}
      aria-label="Formatting"
      onAnimationEnd={onAnimationEnd}
    >
      <IconButton
        ref={closeButton}
        aria-label="Close formatting"
        icon={<IconX size={16} aria-hidden="true" />}
        size="toolbar"
        variant="ghost"
        onClick={onClose}
      />
      <span className="composer-formatting-divider" aria-hidden="true" />
      {COMMANDS.map(({ format, label, icon: Icon }) => (
        <IconButton
          key={format}
          aria-label={label}
          aria-pressed={activeFormats.includes(format)}
          icon={<Icon size={16} stroke={2} aria-hidden="true" />}
          size="toolbar"
          variant="ghost"
          disabled={disabled || !onFormat}
          onMouseDown={(event) => {
            onCommandPointerDown?.();
            event.preventDefault();
          }}
          onClick={() => onFormat?.(format)}
        />
      ))}
    </fieldset>
  );
}
