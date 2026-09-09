import * as React from "react";
import type { EditAgentFocusTarget } from "@/features/agents/openEditAgentEvent";

/**
 * Deep-link focus coordination for the merged agent edit dialog.
 *
 * A card can open the dialog with a target field to focus. Two of those
 * targets are not focusable at open time: the provider/model combobox trigger
 * is `disabled` while model discovery is in flight, and an `env_key` row lives
 * inside the Advanced accordion, which seeds collapsed so its editor is
 * unmounted. This hook lands focus on the requested target once it becomes
 * reachable, and returns the `onOpenAutoFocus` handler that keeps Radix from
 * racing (or stranding) that focus.
 */
export function useAgentEditDeepLinkFocus({
  open,
  initialFocus,
  contextKey,
  llmProviderFieldVisible,
  modelDiscoveryLoading,
  setShowAdvancedFields,
}: {
  open: boolean;
  initialFocus?: EditAgentFocusTarget;
  /** Reset the one-shot guard when the edited agent/definition changes. */
  contextKey: string | undefined;
  llmProviderFieldVisible: boolean;
  modelDiscoveryLoading: boolean;
  setShowAdvancedFields: (next: boolean) => void;
}): {
  onOpenAutoFocus: ((event: Event) => void) | undefined;
} {
  const normalizedFieldFocusFiredRef = React.useRef(false);
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — reset guard on open/context-switch; pure ref mutation is correct
  React.useEffect(() => {
    normalizedFieldFocusFiredRef.current = false;
  }, [open, contextKey]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — llmProviderFieldVisible and modelDiscoveryLoading are the availability signals; contextKey handles agent-switch
  React.useEffect(() => {
    if (!open || initialFocus?.type !== "normalized_field") return;
    if (normalizedFieldFocusFiredRef.current) return;
    const el = document.getElementById(normalizedFieldId(initialFocus));
    if (!(el instanceof HTMLElement)) return;
    // The model/provider combobox trigger is disabled while model discovery is
    // in flight; focusing a disabled control is a silent no-op. Wait for it to
    // become focusable — this effect re-runs when modelDiscoveryLoading flips —
    // and only latch the one-shot guard once focus can actually land.
    if (el.hasAttribute("disabled")) return;
    normalizedFieldFocusFiredRef.current = true;
    const id = requestAnimationFrame(() => {
      el.scrollIntoView({ block: "nearest" });
      el.focus();
    });
    return () => cancelAnimationFrame(id);
  }, [
    open,
    initialFocus,
    contextKey,
    llmProviderFieldVisible,
    modelDiscoveryLoading,
  ]);

  // env_key deep links live inside the Advanced accordion, which seeds
  // collapsed on open — so EnvVarsEditor (which consumes the focus key) is
  // unmounted and can never receive focus. Expand Advanced when an env_key
  // target is requested so the editor mounts and its own focus effect can run.
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — re-expand on open/target-change; contextKey handles agent-switch
  React.useEffect(() => {
    if (!open || initialFocus?.type !== "env_key") return;
    setShowAdvancedFields(true);
  }, [open, initialFocus, contextKey]);

  const onOpenAutoFocus = initialFocus
    ? (event: Event) => {
        // When a target is supplied we cannot let Radix's open-autofocus race
        // the targeted focus effects. But blanket suppression strands focus on
        // <body> whenever the target isn't focusable yet (model combobox
        // disabled during discovery; env-key row unmounted while Advanced is
        // collapsed). So suppress ONLY when the normalized target is already
        // focusable and claim it here; otherwise let Radix autofocus a sane
        // first control as the baseline and let the targeted effects move focus
        // once the target materializes.
        if (initialFocus.type !== "normalized_field") return;
        const target = document.getElementById(normalizedFieldId(initialFocus));
        if (target instanceof HTMLElement && !target.hasAttribute("disabled")) {
          event.preventDefault();
          target.focus();
        }
      }
    : undefined;

  return { onOpenAutoFocus };
}

function normalizedFieldId(
  focus: Extract<EditAgentFocusTarget, { type: "normalized_field" }>,
): string {
  return focus.field === "provider"
    ? "edit-agent-llm-provider"
    : "edit-agent-model";
}
