import { expect, test, type Page } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

const TERM = 'section[aria-label="Buzz Term"]';
const NAMED = 0x0100_0000;
const FG = NAMED | 256;
const BG = NAMED | 257;

/**
 * The shipped mock bridge throws on every `terminal_*` command, so Buzz Term is
 * unreachable through `installMockBridge` alone. This pre-creates
 * `__TAURI_INTERNALS__` and traps the `invoke` assignment `mockIPC` makes:
 * terminal commands are answered here, everything else falls through to the
 * real mock bridge.
 */
async function installTerminalBackend(page: Page) {
  await page.addInitScript(() => {
    const w = window as typeof window & {
      isTauri?: boolean;
      __TAURI_INTERNALS__?: Record<string, unknown>;
      __SAMI_TERM__?: unknown;
    };
    w.isTauri = true;
    const state = {
      deliver: null as ((message: unknown) => void) | null,
      scrolls: [] as number[],
      inputs: [] as string[],
      subscriptionId: "sami-sub-1",
      columns: 80,
      screenLines: 24,
      sequence: 0,
    };
    w.__SAMI_TERM__ = state;

    const internals: Record<string, unknown> = {};
    let inner: ((cmd: string, args: unknown, opts: unknown) => unknown) | null =
      null;
    Object.defineProperty(internals, "invoke", {
      configurable: true,
      get:
        () => (cmd: string, args: Record<string, unknown>, opts: unknown) => {
          switch (cmd) {
            case "terminal_attach": {
              const channel = args.onFrame as {
                onmessage: (m: unknown) => void;
              };
              state.deliver = (message) => channel.onmessage(message);
              return Promise.resolve({
                sessionId: `sami-session-${state.sequence}`,
                subscriptionId: state.subscriptionId,
                viewport: {
                  generation: 1,
                  columns: state.columns,
                  screenLines: state.screenLines,
                },
              });
            }
            case "terminal_resize":
              state.columns = args.columns as number;
              state.screenLines = args.rows as number;
              return Promise.resolve({
                generation: 1,
                columns: state.columns,
                screenLines: state.screenLines,
              });
            case "terminal_input":
              state.inputs.push(args.data as string);
              return Promise.resolve(null);
            case "terminal_scroll":
              state.scrolls.push(args.lines as number);
              return Promise.resolve(null);
            case "terminal_viewport_ready":
            case "terminal_ack":
            case "terminal_focus":
            case "terminal_detach":
            case "terminal_close":
              return Promise.resolve(null);
            default:
              if (!inner) throw new Error(`no mock bridge for ${cmd}`);
              return inner(cmd, args, opts);
          }
        },
      set: (fn: (cmd: string, args: unknown, opts: unknown) => unknown) => {
        inner = fn;
      },
    });
    w.__TAURI_INTERNALS__ = internals;
  });
}

async function pushFrame(
  page: Page,
  rows: { line: number; text: string }[],
  cursor: { line: number; column: number },
) {
  await page.evaluate(
    ({ rows, cursor, fg, bg }) => {
      const state = (
        window as typeof window & {
          __SAMI_TERM__: {
            deliver: ((m: unknown) => void) | null;
            subscriptionId: string;
            columns: number;
            screenLines: number;
            sequence: number;
          };
        }
      ).__SAMI_TERM__;
      state.sequence += 1;
      state.deliver?.({
        type: "frame",
        payload: {
          subscriptionId: state.subscriptionId,
          sequence: state.sequence,
          bracketedPaste: false,
          focusReporting: false,
          full: true,
          viewport: {
            generation: 1,
            columns: state.columns,
            screenLines: state.screenLines,
          },
          cursor: { ...cursor, visible: true },
          rows: rows.map(({ line, text }) => ({
            line,
            spans: [
              {
                style: { fg, bg, flags: 0 },
                clusters: [...text].map((ch, index) => ({
                  column: index,
                  text: ch,
                  width: 1,
                })),
              },
            ],
          })),
        },
      });
    },
    { rows, cursor, fg: FG, bg: BG },
  );
}

async function reveal(page: Page) {
  await page.setViewportSize({ width: 1280, height: 800 });
  await installTerminalBackend(page);
  await installMockBridge(page);
  await page.goto("/");
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  // Buzz Term needs a channel: TerminalBootstrap's context is null on Home, so
  // no session spawns and the chord is inert.
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.keyboard.press("Meta+j");
  await expect(page.locator(TERM)).toHaveAttribute(
    "data-terminal-owner",
    "terminal",
  );
  // The dock opens in the normal app surface now; unlike the removed
  // full-screen takeover, it must not fade that surface away. Wait for the
  // dock's own height transition before interacting with its viewport.
  await expect(page.locator(TERM)).toHaveAttribute(
    "data-terminal-mode",
    "docked",
  );
  await expect(page.locator(TERM)).toBeVisible();
  await expect
    .poll(async () => page.locator(TERM).evaluate((el) => el.clientHeight))
    .toBeGreaterThanOrEqual(180);
}

test("project terminal button opens Buzz Term for the repository", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await installTerminalBackend(page);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByTestId("projects-section-projects").click();
  const projectEntry = page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first();
  await expect(projectEntry).toBeVisible({ timeout: 10_000 });
  await projectEntry.click();
  await page.getByTestId("project-home-context-repo-buzz").click();

  const terminalButton = page.getByTestId("project-terminal-toggle");
  await expect(terminalButton).toBeEnabled();
  await terminalButton.click();
  await expect(page.locator(TERM)).toHaveAttribute(
    "data-terminal-mode",
    "docked",
  );
  await expect(page.locator(TERM)).toBeVisible();
});

test("scrollback: wheel over Buzz Term reaches terminal_scroll", async ({
  page,
}) => {
  await reveal(page);
  await pushFrame(page, [{ line: 0, text: "buzz@term:~$ " }], {
    line: 0,
    column: 13,
  });
  await page.mouse.move(640, 500);
  await page.mouse.wheel(0, -17 * 5);
  await page.mouse.wheel(0, 17 * 2);
  const scrolls = await page.evaluate(
    () =>
      (window as typeof window & { __SAMI_TERM__: { scrolls: number[] } })
        .__SAMI_TERM__.scrolls,
  );
  console.log("SCROLLS", JSON.stringify(scrolls));
  expect(scrolls).toEqual([-5, 2]);
});

test("terminal viewport click retains keyboard input ownership", async ({
  page,
}) => {
  await reveal(page);
  const input = page.getByLabel("Terminal input");
  await expect(input).toBeFocused();

  await page
    .locator(".buzz-terminal-viewport")
    .click({ position: { x: 40, y: 40 } });
  await expect(input).toBeFocused();

  await page.keyboard.type("FOCUS_KEYSTROKE");
  await expect
    .poll(async () =>
      page.evaluate(() =>
        (
          window as typeof window & { __SAMI_TERM__: { inputs: string[] } }
        ).__SAMI_TERM__.inputs.join(""),
      ),
    )
    .toContain("FOCUS_KEYSTROKE");
});

test("concealed terminal viewport does not steal Buzz focus", async ({
  page,
}) => {
  await reveal(page);
  await page.keyboard.press("Meta+j");
  await expect(page.locator(TERM)).toHaveCount(0);

  const input = page.getByLabel("Terminal input");
  await expect(input).toHaveCount(0);
  await page.getByTestId("chat-title").click();
  await page.keyboard.type("BUZZ_KEYSTROKE");
  const terminalInputs = await page.evaluate(
    () =>
      (window as typeof window & { __SAMI_TERM__: { inputs: string[] } })
        .__SAMI_TERM__.inputs,
  );
  expect(terminalInputs.join("")).not.toContain("BUZZ_KEYSTROKE");
});
