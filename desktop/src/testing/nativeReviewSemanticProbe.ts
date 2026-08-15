type SemanticNode = {
  id?: string;
  role?: string;
  name?: string;
  enabled: boolean;
  focused: boolean;
  frame: { x: number; y: number; width: number; height: number };
  viewport: { width: number; height: number };
};

const IMPLICIT_ROLES: Partial<Record<string, string>> = {
  A: "link",
  BUTTON: "button",
  INPUT: "text-field",
  TEXTAREA: "text-area",
};

function accessibleName(element: HTMLElement): string | undefined {
  const labelledBy = element.getAttribute("aria-labelledby");
  const labelledText = labelledBy
    ?.split(/\s+/)
    .map((id) => document.getElementById(id)?.textContent?.trim())
    .filter(Boolean)
    .join(" ");
  return (
    element.getAttribute("aria-label")?.trim() ||
    labelledText ||
    element.getAttribute("title")?.trim() ||
    (element.getAttribute("role") === "tooltip"
      ? element.textContent?.trim()
      : undefined) ||
    undefined
  );
}

function snapshot(): SemanticNode[] {
  const nodes: SemanticNode[] = [];
  for (const candidate of document.querySelectorAll<HTMLElement>(
    "[data-testid], [role], button, textarea, input, a[href]",
  )) {
    const rect = candidate.getBoundingClientRect();
    const style = window.getComputedStyle(candidate);
    if (
      rect.width <= 0 ||
      rect.height <= 0 ||
      style.display === "none" ||
      style.visibility === "hidden"
    ) {
      continue;
    }
    const id = candidate.dataset.testid;
    const role =
      candidate.getAttribute("role") ?? IMPLICIT_ROLES[candidate.tagName];
    const name = accessibleName(candidate);
    if (!id && !role && !name) continue;
    nodes.push({
      ...(id ? { id } : {}),
      ...(role ? { role } : {}),
      ...(name ? { name } : {}),
      enabled:
        !candidate.hasAttribute("disabled") &&
        candidate.getAttribute("aria-disabled") !== "true",
      focused: candidate === document.activeElement,
      frame: {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
      },
      viewport: {
        width: window.innerWidth,
        height: window.innerHeight,
      },
    });
  }
  return nodes;
}

export function installNativeReviewSemanticProbe(): void {
  let scheduled = false;
  const publish = () => {
    scheduled = false;
    const payload = JSON.stringify(snapshot());
    if (
      !navigator.sendBeacon(
        import.meta.env.VITE_NATIVE_REVIEW_PROBE_URL,
        payload,
      )
    ) {
      console.error("native review semantic probe beacon was rejected");
    }
  };
  const schedule = () => {
    if (scheduled) return;
    scheduled = true;
    window.requestAnimationFrame(publish);
  };
  new MutationObserver(schedule).observe(document.documentElement, {
    attributes: true,
    childList: true,
    subtree: true,
  });
  window.addEventListener("focusin", schedule);
  window.addEventListener("focusout", schedule);
  window.addEventListener("resize", schedule);
  window.addEventListener("scroll", schedule, true);
  schedule();
}
