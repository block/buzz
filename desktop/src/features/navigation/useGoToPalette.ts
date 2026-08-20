import * as React from "react";

import {
  filterGoToDestinations,
  resolveDigitAccelerator,
  resolveMnemonicAccelerator,
  selectEnabledDestinations,
} from "@/features/navigation/lib/goToAccelerators";
import {
  GO_TO_DESTINATIONS,
  type GoToDestination,
  type GoToDestinationId,
} from "@/features/navigation/lib/goToDestinations";
import {
  getFeature,
  resolveEnabled,
  useFeatureSnapshot,
} from "@/shared/features";
import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";

type UseGoToPaletteOptions = {
  /** When true the ⌘G leader is inert and any open palette is force-closed. */
  disabled: boolean;
  onNavigate: (id: GoToDestinationId) => void;
};

export type GoToPaletteResult = {
  destination: GoToDestination;
  /** 1-based position in the visible list — the bare-digit accelerator. */
  position: number;
};

export type GoToPaletteState = {
  open: boolean;
  query: string;
  results: GoToPaletteResult[];
  selectedIndex: number;
  onOpenChange: (open: boolean) => void;
  onQueryChange: (query: string) => void;
  onHoverIndex: (index: number) => void;
  onSelect: (id: GoToDestinationId) => void;
};

/**
 * Owns the ⌘G "Go to" palette: a persistent capture-phase key listener acts as
 * a leader (⌘G opens from anywhere), then routes accelerators while open — bare
 * digits jump by visible position, ⌘/Ctrl+letter jumps by global mnemonic, and
 * arrows/Enter drive the highlighted row. Plain typing falls through to the
 * filter input. Feature-gated areas are hidden so the palette never dead-ends.
 */
export function useGoToPalette({
  disabled,
  onNavigate,
}: UseGoToPaletteOptions): GoToPaletteState {
  const overrides = useFeatureSnapshot();
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const [selectedIndex, setSelectedIndex] = React.useState(0);

  const isFeatureEnabled = React.useCallback(
    (feature: string) => {
      const def = getFeature(feature);
      if (!def) return true;
      return resolveEnabled(feature, overrides, def.defaultEnabled);
    },
    [overrides],
  );

  const enabledDestinations = React.useMemo(
    () => selectEnabledDestinations(GO_TO_DESTINATIONS, isFeatureEnabled),
    [isFeatureEnabled],
  );

  const results = React.useMemo<GoToPaletteResult[]>(
    () =>
      filterGoToDestinations(enabledDestinations, query).map(
        (destination, index) => ({ destination, position: index + 1 }),
      ),
    [enabledDestinations, query],
  );

  const clampedSelectedIndex =
    results.length === 0 ? 0 : Math.min(selectedIndex, results.length - 1);

  const close = React.useCallback(() => {
    setOpen(false);
    setQuery("");
    setSelectedIndex(0);
  }, []);

  const jump = React.useCallback(
    (id: GoToDestinationId) => {
      close();
      onNavigate(id);
    },
    [close, onNavigate],
  );

  const onOpenChange = React.useCallback(
    (next: boolean) => {
      if (next) {
        setQuery("");
        setSelectedIndex(0);
        setOpen(true);
        return;
      }
      close();
    },
    [close],
  );

  const onQueryChange = React.useCallback((next: string) => {
    setQuery(next);
    setSelectedIndex(0);
  }, []);

  // Force-close if the palette becomes unavailable (e.g. entering a huddle).
  React.useEffect(() => {
    if (disabled && open) {
      close();
    }
  }, [disabled, open, close]);

  // Mirror the latest render values so the single persistent listener never
  // needs rebinding on each keystroke.
  const stateRef = React.useRef({
    open,
    disabled,
    results,
    enabledDestinations,
    selectedIndex: clampedSelectedIndex,
  });
  stateRef.current = {
    open,
    disabled,
    results,
    enabledDestinations,
    selectedIndex: clampedSelectedIndex,
  };
  const jumpRef = React.useRef(jump);
  jumpRef.current = jump;

  React.useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      const {
        open: isOpen,
        disabled: isDisabled,
        results: currentResults,
        enabledDestinations: currentEnabled,
        selectedIndex: currentSelected,
      } = stateRef.current;
      const isPrimaryModifier = hasPrimaryShortcutModifier(event);

      // Leader: ⌘/Ctrl+G opens from anywhere. Capture phase so it wins even
      // when a composer has focus; synchronous state flush means the very next
      // keydown is already an accelerator (works before the modal paints).
      if (
        !isOpen &&
        isPrimaryModifier &&
        !event.shiftKey &&
        !event.altKey &&
        !event.repeat &&
        event.key.toLowerCase() === "g"
      ) {
        if (isDisabled) return;
        event.preventDefault();
        event.stopPropagation();
        onOpenChange(true);
        return;
      }

      if (!isOpen) return;

      // ⌘/Ctrl+letter → global mnemonic jump (ignores the current filter).
      if (isPrimaryModifier && !event.altKey) {
        if (event.key.toLowerCase() === "g") {
          // Re-pressing the leader while open is inert (Esc/selection closes).
          event.preventDefault();
          event.stopPropagation();
          return;
        }
        const match = resolveMnemonicAccelerator(currentEnabled, event.key);
        if (match) {
          event.preventDefault();
          event.stopPropagation();
          jumpRef.current(match.id);
        }
        return;
      }

      // Bare digit → jump by position in the visible list.
      if (!event.shiftKey) {
        const digitMatch = resolveDigitAccelerator(
          currentResults.map((result) => result.destination),
          event.key,
        );
        if (digitMatch) {
          event.preventDefault();
          event.stopPropagation();
          jumpRef.current(digitMatch.id);
          return;
        }
      }

      if (event.key === "ArrowDown") {
        event.preventDefault();
        event.stopPropagation();
        setSelectedIndex((index) =>
          currentResults.length === 0
            ? 0
            : Math.min(index + 1, currentResults.length - 1),
        );
        return;
      }

      if (event.key === "ArrowUp") {
        event.preventDefault();
        event.stopPropagation();
        setSelectedIndex((index) => Math.max(index - 1, 0));
        return;
      }

      if (event.key === "Enter") {
        const selected = currentResults[currentSelected];
        if (selected) {
          event.preventDefault();
          event.stopPropagation();
          jumpRef.current(selected.destination.id);
        }
        return;
      }
      // Letters, Backspace, Escape, etc. fall through to the input / Radix.
    }

    window.addEventListener("keydown", handleKeyDown, { capture: true });
    return () => {
      window.removeEventListener("keydown", handleKeyDown, { capture: true });
    };
  }, [onOpenChange]);

  return {
    open,
    query,
    results,
    selectedIndex: clampedSelectedIndex,
    onOpenChange,
    onQueryChange,
    onHoverIndex: setSelectedIndex,
    onSelect: jump,
  };
}
