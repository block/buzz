type SemanticNode = {
  id?: string;
  role?: string;
  name?: string;
  value?: string;
  scrollY: number;
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
    const value =
      candidate instanceof HTMLInputElement ||
      candidate instanceof HTMLTextAreaElement
        ? candidate.value
        : candidate.isContentEditable
          ? candidate.innerText.replace(/\r\n?/g, "\n").replace(/\n$/, "")
          : undefined;
    if (!id && !role && !name) continue;
    nodes.push({
      ...(id ? { id } : {}),
      ...(role ? { role } : {}),
      ...(name ? { name } : {}),
      ...(value !== undefined ? { value } : {}),
      scrollY: candidate.scrollTop,
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

export function installNativeReviewSemanticProbe(config: {
  probeUrl: string;
  probeToken: string;
}): void {
  const { probeUrl, probeToken } = config;
  const parsed = new URL(probeUrl);
  if (
    parsed.protocol !== "http:" ||
    parsed.hostname !== "127.0.0.1" ||
    !parsed.port ||
    parsed.pathname !== "/snapshot" ||
    parsed.search ||
    parsed.hash ||
    !probeToken
  ) {
    throw new Error("native review semantic probe destination is invalid");
  }
  let scheduled = false;
  let probeInFlight = false;
  let publishPending = false;
  const schedule = () => {
    if (probeInFlight) {
      publishPending = true;
      return;
    }
    if (scheduled) return;
    scheduled = true;
    window.requestAnimationFrame(publish);
  };
  const publish = () => {
    scheduled = false;
    probeInFlight = true;
    const payload = JSON.stringify(snapshot());
    void fetch(probeUrl, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-Buzz-Native-Review-Token": probeToken,
      },
      body: payload,
    })
      .catch((error) => {
        console.error("native review semantic probe publish failed", error);
      })
      .finally(() => {
        probeInFlight = false;
        if (publishPending) {
          publishPending = false;
          schedule();
        }
      });
  };
  new MutationObserver(schedule).observe(document.documentElement, {
    attributes: true,
    childList: true,
    subtree: true,
  });
  window.addEventListener("input", schedule, true);
  window.addEventListener("change", schedule, true);
  window.addEventListener("focusin", schedule);
  window.addEventListener("focusout", schedule);
  window.addEventListener("resize", schedule);
  window.addEventListener("scroll", schedule, true);
  schedule();
}
