import * as React from "react";

export type ControlledDisclosure = {
  /** Whether the disclosure should currently render open. */
  open: boolean;
  /** Pass straight to a controlled `<details>` / `ActivityRow` change handler. */
  onOpenChange: (open: boolean) => void;
};

/**
 * Disclosure state for a `<details>` whose open state has a *policy* — some
 * rule that opens or closes it as data changes (a tool run opens while it is
 * live and closes when it settles; a thought opens while the agent is still
 * thinking) — but where the reader's own click must win from then on.
 *
 * Pass the policy's current answer as `policyOpen`. Until the reader touches
 * the row, that answer is what renders; afterwards their choice does, for as
 * long as the component stays mounted.
 *
 * ## The echo trap this exists to guard
 *
 * `<details>` fires `toggle` for programmatic `open` changes as well as for
 * clicks, and the event carries no way to tell the two apart. So when policy
 * opens the row, the DOM echoes a `toggle` back that looks exactly like a
 * reader opening it — and naively recording that as a reader choice pins the
 * row to its first policy state forever, silently disabling every later policy
 * transition (a completed tool run would never collapse again).
 *
 * The discriminator is agreement: an echo always reports the state we just
 * rendered, so only a `toggle` that DISAGREES with the last rendered state can
 * have come from the reader. jsdom does not emit the echo, so a test for this
 * must inject the agreeing `toggle` itself or it passes vacuously.
 */
export function useControlledDisclosure(
  policyOpen: boolean,
): ControlledDisclosure {
  const [readerChoice, setReaderChoice] = React.useState<boolean | null>(null);
  const open = readerChoice ?? policyOpen;

  const renderedRef = React.useRef(open);
  React.useLayoutEffect(() => {
    renderedRef.current = open;
  }, [open]);

  const onOpenChange = React.useCallback((next: boolean) => {
    if (next === renderedRef.current) return;
    setReaderChoice(next);
  }, []);

  return { onOpenChange, open };
}
