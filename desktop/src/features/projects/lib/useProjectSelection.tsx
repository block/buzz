import * as React from "react";

import {
  EMPTY_PROJECT_SELECTION,
  nextProjectGroupSelection,
  nextProjectSelection,
  type ProjectSelectionItem,
  type ProjectSelectionState,
} from "./projectSelection";

type ProjectSelectionContextValue = {
  active: boolean;
  clear: () => void;
  isSelected: (id: string) => boolean;
  items: ProjectSelectionItem[];
  toggleGroup: (items: ProjectSelectionItem[]) => void;
  toggle: (
    item: ProjectSelectionItem,
    options?: { rangeItems?: ProjectSelectionItem[]; shiftKey?: boolean },
  ) => void;
};

const ProjectSelectionContext =
  React.createContext<ProjectSelectionContextValue | null>(null);

export function ProjectSelectionProvider({
  children,
  onClear,
  onSelect,
  resetKey,
}: {
  children: React.ReactNode;
  onClear?: () => void;
  onSelect?: () => void;
  resetKey: string;
}) {
  const [state, setState] = React.useState<ProjectSelectionState>(
    EMPTY_PROJECT_SELECTION,
  );
  const [prevResetKey, setPrevResetKey] = React.useState(resetKey);
  if (resetKey !== prevResetKey) {
    setPrevResetKey(resetKey);
    setState(EMPTY_PROJECT_SELECTION);
  }
  const onSelectRef = React.useRef(onSelect);
  onSelectRef.current = onSelect;
  const onClearRef = React.useRef(onClear);
  onClearRef.current = onClear;
  const wasActiveRef = React.useRef(false);

  const clear = React.useCallback(() => {
    setState(EMPTY_PROJECT_SELECTION);
  }, []);

  const toggle = React.useCallback(
    (
      item: ProjectSelectionItem,
      options?: { rangeItems?: ProjectSelectionItem[]; shiftKey?: boolean },
    ) => {
      setState((current) => nextProjectSelection(current, item, options));
    },
    [],
  );
  const toggleGroup = React.useCallback((items: ProjectSelectionItem[]) => {
    setState((current) => nextProjectGroupSelection(current, items));
  }, []);

  React.useEffect(() => {
    const active = state.items.length > 0;
    if (active && !wasActiveRef.current) onSelectRef.current?.();
    if (!active && wasActiveRef.current) onClearRef.current?.();
    wasActiveRef.current = active;
  }, [state.items.length]);

  React.useEffect(() => {
    if (state.items.length === 0) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented) clear();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [clear, state.items.length]);

  const value = React.useMemo<ProjectSelectionContextValue>(
    () => ({
      active: state.items.length > 0,
      clear,
      isSelected: (id) => state.items.some((item) => item.id === id),
      items: state.items,
      toggle,
      toggleGroup,
    }),
    [clear, state.items, toggle, toggleGroup],
  );

  return (
    <ProjectSelectionContext.Provider value={value}>
      {children}
    </ProjectSelectionContext.Provider>
  );
}

export function useProjectSelection() {
  return React.useContext(ProjectSelectionContext);
}
