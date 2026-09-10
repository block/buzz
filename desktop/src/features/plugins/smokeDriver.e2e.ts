/**
 * Native-smoke UI driver. A side-effect module imported only when
 * `plugin_browser_smoke_enabled()` returns true (see `main.tsx`) — a
 * production build never reaches this file. It walks the real Settings →
 * Plugins UI through the same production commands and DOM a person would
 * use: install (native directory picker and consent are auto-answered on
 * the Rust side under `cfg(debug_assertions)`), open the contributed
 * surface, submit a refused and an admitted address, then disable and
 * uninstall. Every step is asserted against real `plugin-browser-*` events,
 * never against its own marker alone.
 *
 * Every listener is registered and its native registration awaited *before*
 * the action that could produce the event is triggered — a fast native
 * response can otherwise land before a listener registered afterward would
 * ever see it.
 *
 * The fixture server's origin is discovered from the resolved home URL
 * (`--home-url http://127.0.0.1:<port>/home`) rather than hardcoded, so this
 * file needs no separate coordination constant with the native orchestration
 * script beyond the marker path shape below.
 */
import { subscribeOnce } from "./subscribeOnce";

type BrowserLocationEventPayload = {
  sessionId: string;
  generation: number;
  url: string;
};
type BrowserErrorEventPayload = {
  sessionId: string;
  generation: number;
  code: string;
};

function log(message: string) {
  // eslint-disable-next-line no-console
  console.log(`buzz-plugin-browser-smoke: ${message}`);
}

async function postMarker(origin: string, step: string): Promise<void> {
  try {
    await fetch(`${origin}/driver/${step}`, { method: "POST" });
  } catch (cause) {
    log(`marker post failed for ${step}: ${String(cause)}`);
  }
}

type PermissionReport = { count: number; outcomes: string[] };
type DriverStatus = {
  permissions?: Record<string, PermissionReport>;
};

async function fetchDriverStatus(origin: string): Promise<DriverStatus | null> {
  try {
    const response = await fetch(`${origin}/driver-status`);
    if (!response.ok) return null;
    return (await response.json()) as DriverStatus;
  } catch (cause) {
    log(`driver-status poll failed: ${String(cause)}`);
    return null;
  }
}

/**
 * Every `/home` load starts audio+video `getUserMedia` probes that report to
 * `/driver-status`'s `permissions` map, keyed by kind to a cumulative
 * `{count, outcomes}` that accumulates across the whole run and is never
 * reset — so a stale report from an earlier load can't be mistaken for
 * evidence of the current one. This driver has exactly two `/home` loads
 * (the initial one, then the Reload step), so `expectedCountPerKind` is 1
 * before the reload and 2 before the destination navigation; an unexpected
 * count fails explicitly rather than silently waiting on the wrong evidence.
 *
 * Navigating away (a reload or a real navigation) before a load's own probes
 * report cancels them mid-flight, and a cancelled probe never reports at
 * all — so this must be a positive wait, never a timeout treated as an
 * implicit denial.
 */
async function waitForPermissionReports(
  origin: string,
  expectedCountPerKind: number,
  timeoutMs = 8_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const status = await fetchDriverStatus(origin);
    const permissions = status?.permissions ?? {};
    for (const kind of ["audio", "video"] as const) {
      const report = permissions[kind];
      if (report && report.count > expectedCountPerKind) {
        throw new Error(
          `/driver-status reported ${report.count} ${kind} outcomes, expected exactly ${expectedCountPerKind} at this point in the flow`,
        );
      }
    }
    const audio = permissions.audio;
    const video = permissions.video;
    if (
      audio?.count === expectedCountPerKind &&
      video?.count === expectedCountPerKind &&
      audio.outcomes.every((outcome) => outcome === "rejected") &&
      video.outcomes.every((outcome) => outcome === "rejected")
    ) {
      return;
    }
    if (Date.now() > deadline) {
      throw new Error(
        `timed out waiting for /driver-status to report ${expectedCountPerKind} rejected audio+video outcomes each`,
      );
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
}

async function waitFor<T extends Element>(
  selector: string,
  timeoutMs = 15_000,
): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const found = document.querySelector<T>(selector);
    if (found) return found;
    if (Date.now() > deadline) {
      throw new Error(`timed out waiting for "${selector}"`);
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
}

async function waitForCondition(
  description: string,
  predicate: () => boolean,
  timeoutMs = 15_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    if (predicate()) return;
    if (Date.now() > deadline) {
      throw new Error(`timed out waiting for ${description}`);
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
}

function click(el: Element) {
  el.dispatchEvent(
    new MouseEvent("click", { bubbles: true, cancelable: true }),
  );
}

function setInputValue(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(
    window.HTMLInputElement.prototype,
    "value",
  )?.set;
  setter?.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

async function run(): Promise<void> {
  log("starting");

  // `open-settings` only toggles the profile popover open — the actual
  // Settings dialog is opened by the "Settings" item inside it.
  click(await waitFor("[data-testid='open-settings']"));
  click(await waitFor("[data-testid='profile-popover-settings']"));
  click(await waitFor("[data-testid='settings-nav-plugins']"));
  await waitFor("[data-testid='settings-panel-plugins']");

  click(await waitFor("[data-testid='plugins-install-button']"));
  const openButton = await waitFor<HTMLButtonElement>(
    "[data-testid='plugin-row'] button",
    30_000,
  );

  const homeLoadedSub = subscribeOnce<BrowserLocationEventPayload>(
    "plugin-browser-loaded",
    () => true,
  );
  await homeLoadedSub.ready;
  click(openButton);
  await waitFor("[data-testid='plugin-browser-surface']");
  const homeLoaded = await homeLoadedSub.result;
  const origin = new URL(homeLoaded.url).origin;
  log(`home loaded: ${homeLoaded.url}`);
  await postMarker(origin, "home-loaded");

  const session = {
    sessionId: homeLoaded.sessionId,
    generation: homeLoaded.generation,
  };

  // A refused address: the example plugin itself rejects a non-web address,
  // proving the user-facing rejection flow. Per the public contract (`docs/
  // plugin-browser-prototype.md`), a refused address stays visible in the
  // field — it is not reverted to home — so the display assertion below
  // checks for the typed text, not a forced rewrite back to `homeLoaded.url`.
  const addressInput = await waitFor<HTMLInputElement>(
    "[aria-label='Address']",
  );
  const refusalSub = subscribeOnce<BrowserErrorEventPayload>(
    "plugin-browser-error",
    (payload) =>
      payload.sessionId === session.sessionId &&
      payload.generation === session.generation &&
      payload.code === "navigation-denied",
  );
  await refusalSub.ready;
  const refusedInput = "not a web address";
  setInputValue(addressInput, refusedInput);
  addressInput
    .closest("form")
    ?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  await refusalSub.result;
  if (addressInput.value !== refusedInput) {
    throw new Error(
      `typed address was not left visible after a refusal: ${addressInput.value}`,
    );
  }
  await postMarker(origin, "refusal-visible-and-preserved");

  // Positive proof the refusal never actually navigated: Reload re-loads
  // whatever page is currently committed, so if it lands back on
  // `homeLoaded.url` the native view was still on the home page all along.
  // Subscribed before the click, per the same listener-before-action rule
  // every other step in this driver follows.
  const reloadButton = await waitFor<HTMLButtonElement>(
    "[aria-label='Reload']",
  );
  const reloadSub = subscribeOnce<BrowserLocationEventPayload>(
    "plugin-browser-loaded",
    (payload) =>
      payload.sessionId === session.sessionId && payload.url === homeLoaded.url,
  );
  await reloadSub.ready;
  // The initial /home load's audio+video probes must settle before this
  // reload unloads the page and cancels them mid-flight.
  await waitForPermissionReports(origin, 1);
  click(reloadButton);
  await reloadSub.result;
  await postMarker(origin, "refusal-unchanged-url");

  // An admitted address on the same fixture origin.
  const destinationUrl = `${origin}/destination`;
  const destinationSub = subscribeOnce<BrowserLocationEventPayload>(
    "plugin-browser-loaded",
    (payload) =>
      payload.sessionId === session.sessionId && payload.url === destinationUrl,
  );
  await destinationSub.ready;
  // The reloaded /home load's own probes must settle before this navigation
  // unloads it and cancels them mid-flight — two loads total by now.
  await waitForPermissionReports(origin, 2);
  setInputValue(addressInput, destinationUrl);
  addressInput
    .closest("form")
    ?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  const destinationLoaded = await destinationSub.result;
  log(`destination loaded: ${destinationLoaded.url}`);
  await postMarker(origin, "destination-loaded");

  // Back to the Plugins panel to disable, then uninstall. Both mutations are
  // async (invoke + registry refresh) and disable the row's own controls
  // while in flight — the marker and the next click must wait for the
  // authoritative post-refresh state, not fire immediately after the click,
  // or the uninstall click can land on a still-disabled button and silently
  // no-op.
  click(await waitFor("[data-testid='open-settings']"));
  click(await waitFor("[data-testid='profile-popover-settings']"));
  click(await waitFor("[data-testid='settings-nav-plugins']"));
  const disableSwitch = await waitFor<HTMLElement>(
    "[data-testid='plugin-row'] button[role='switch']",
  );
  click(disableSwitch);
  await waitForCondition(
    "the plugin row to report disabled",
    () => disableSwitch.getAttribute("aria-checked") === "false",
  );
  await postMarker(origin, "disabled");

  const uninstallButton = await waitFor<HTMLButtonElement>(
    "[data-testid='plugin-row'] button:last-of-type",
  );
  await waitForCondition(
    "the uninstall button to become clickable",
    () => !uninstallButton.disabled,
  );
  click(uninstallButton);
  await waitForCondition(
    "the plugin row to be removed after uninstall",
    () => document.querySelector("[data-testid='plugin-row']") === null,
  );
  await postMarker(origin, "uninstalled");

  log("complete");
}

run().catch((cause) => {
  // WebKit's `Error.stack` is just `fn@url:line:col` frames — it never
  // includes the message the way V8's does, so falling back to `stack` on
  // its own (as this used to) silently drops the actual failure reason.
  // Always log the message; append the stack only as extra context.
  if (cause instanceof Error) {
    log(`failed: ${cause.message}${cause.stack ? `\n${cause.stack}` : ""}`);
  } else {
    log(`failed: ${String(cause)}`);
  }
});
