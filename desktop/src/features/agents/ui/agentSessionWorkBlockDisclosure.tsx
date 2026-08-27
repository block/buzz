import * as React from "react";

/**
 * The reader's fold/unfold choices for focus-mode work blocks, keyed by the
 * STEP ids a choice was taken on rather than by block id.
 *
 * ## Why this cannot be `useState` inside the block
 *
 * A work block's id is derived from its first step (`work-block:${items[0].id}`,
 * see `groupConversationWorkBlocks`), and block membership is recomputed from
 * scratch on every render. `findFinalAnswerId` exempts only the LAST assistant
 * message from the block, so when a second assistant message arrives the first
 * one stops being the answer, becomes work, and the run of steps around it
 * merges: two blocks become one.
 *
 *     frame 2  work-block:th:1[th:1,tool:1]  msg:1  work-block:th:2[th:2,tool:2]
 *     frame 3  work-block:th:1[th:1,tool:1,msg:1,th:2,tool:2]  msg:2
 *
 * `work-block:th:2` ceases to exist, so React unmounts it and any state it
 * owned goes with it. A reader who had opened it to read the steps was folded
 * shut by an event they did not cause — the agent posting another message.
 *
 * Keying on step ids survives that, because the steps are what the reader's
 * intent was actually about: they are still on screen after the merge, just
 * inside a different block.
 */
export type WorkBlockDisclosureChoices = ReadonlyMap<string, boolean>;

/**
 * The reader's choice for a block, or `null` when they have not taken one and
 * policy still owns the fold.
 *
 * An open choice wins over a folded one. After a merge the block can carry both
 * — one constituent opened, another folded — and the two are not symmetric:
 * showing steps a reader asked to see costs them a scroll, while hiding steps a
 * reader asked to see loses the thing they were reading. Same reason the
 * absorbed block's own choice cannot simply be dropped.
 */
export function readWorkBlockChoice(
  choices: WorkBlockDisclosureChoices,
  itemIds: readonly string[],
): boolean | null {
  let folded: boolean | null = null;
  for (const itemId of itemIds) {
    const choice = choices.get(itemId);
    if (choice === true) return true;
    if (choice === false) folded = false;
  }
  return folded;
}

/**
 * Record one choice against every step the block currently holds.
 *
 * Written across all of them, not just the first, because the block this choice
 * was taken on may later be absorbed into a block that begins with a different
 * step — and the reader's intent has to be findable from whichever step the
 * merged block happens to start with.
 */
export function recordWorkBlockChoice(
  choices: WorkBlockDisclosureChoices,
  itemIds: readonly string[],
  choice: boolean,
): WorkBlockDisclosureChoices {
  const next = new Map(choices);
  for (const itemId of itemIds) next.set(itemId, choice);
  return next;
}

const EMPTY_CHOICES: WorkBlockDisclosureChoices = new Map();

export type WorkBlockDisclosureStore = {
  choices: WorkBlockDisclosureChoices;
  choose: (itemIds: readonly string[], choice: boolean) => void;
};

/**
 * `null` when no transcript is providing a store.
 *
 * Deliberately not a no-op store: a work block rendered on its own — which is
 * how most of its tests mount it — would then swallow every click silently and
 * look like a component whose disclosure is broken. `useWorkBlockDisclosure`
 * falls back to component-local state instead, which is correct in isolation
 * (nothing is regrouping the block) and is the behaviour a reader of that
 * component would expect. A provider emits no DOM, so `default` and
 * `compactPreview` markup is unaffected either way.
 */
const WorkBlockDisclosureContext =
  React.createContext<WorkBlockDisclosureStore | null>(null);

export function AgentSessionWorkBlockDisclosureProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const store = useWorkBlockDisclosureState();

  return (
    <WorkBlockDisclosureContext.Provider value={store}>
      {children}
    </WorkBlockDisclosureContext.Provider>
  );
}

/** The store's state and updater, so the fallback path can reuse it verbatim. */
export function useWorkBlockDisclosureState(): WorkBlockDisclosureStore {
  const [choices, setChoices] =
    React.useState<WorkBlockDisclosureChoices>(EMPTY_CHOICES);
  const choose = React.useCallback(
    (itemIds: readonly string[], choice: boolean) => {
      setChoices((current) => recordWorkBlockChoice(current, itemIds, choice));
    },
    [],
  );

  return React.useMemo(() => ({ choices, choose }), [choices, choose]);
}

export function useWorkBlockDisclosureStore() {
  return React.useContext(WorkBlockDisclosureContext);
}
