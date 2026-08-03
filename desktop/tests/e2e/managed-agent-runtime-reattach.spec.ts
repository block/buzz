import { expect, test, type Page } from "@playwright/test";

import type { ManagedAgentRuntimeStatus } from "@/shared/api/types";
import { installMockBridge } from "../helpers/bridge";

const AGENT_PUBKEY = "57".repeat(32);
const REQUESTER_PUBKEY = "de".repeat(32);
const RELAY_URL = "ws://127.0.0.1:3000";
const CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const JOB_ID = "57557557-5575-4575-8575-575575575575";
const REQUEST_EVENT_ID = "a1".repeat(32);
const RUNTIME_PID = 57_575;

function runtimeStatus(
  lifecycle: ManagedAgentRuntimeStatus["lifecycle"],
): ManagedAgentRuntimeStatus {
  return {
    pubkey: AGENT_PUBKEY,
    relayUrl: RELAY_URL,
    localSetup: true,
    lifecycle,
    pid: RUNTIME_PID,
    error: null,
    logPath: null,
    activeAssignment: {
      assignmentId: "assignment-jac-575",
      channelId: CHANNEL_ID,
      sourceEventId: "b2".repeat(32),
      state: lifecycle === "recovering" ? "recovering" : "working",
      summary: "Run the receipt-verified JAC-575 repair",
      activeJobId: JOB_ID,
      lastProgressAt: "2026-08-02T10:00:30Z",
      blocker: null,
    },
    activeJob: {
      jobId: JOB_ID,
      state: "running",
      summary: "Applying the verified repair",
      lastProgressAt: "2026-08-02T10:00:30Z",
      publicationState: "published",
    },
  };
}

async function waitForMockLiveSubscription(
  page: Page,
  channelName: string,
  kind: number,
) {
  await expect
    .poll(() =>
      page.evaluate(
        ({ kind, name }) =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: name,
            kind,
          }) ?? false,
        { kind, name: channelName },
      ),
    )
    .toBe(true);
}

async function emitRunningJob(page: Page) {
  const createdAt = Math.floor(Date.now() / 1_000) - 20;
  await page.evaluate(
    ({ agentPubkey, createdAt, jobId, requestEventId, requesterPubkey }) => {
      const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      if (!emit) throw new Error("Mock message emitter is unavailable.");
      emit({
        channelName: "general",
        content: JSON.stringify({
          schema: 1,
          driver: "lh",
          argv: ["lockdown", "run", "--issue", "JAC-575"],
          cwd: "/tmp/buzz-runtime-plan",
          summary: "Run the receipt-verified JAC-575 repair",
        }),
        pubkey: requesterPubkey,
        kind: 43001,
        createdAt,
        id: requestEventId,
        extraTags: [
          ["p", agentPubkey],
          ["job", jobId],
        ],
      });
      emit({
        channelName: "general",
        content: JSON.stringify({
          schema: 1,
          job: jobId,
          attempt: 1,
          state: "accepted",
          accepted_at: new Date((createdAt + 1) * 1_000).toISOString(),
        }),
        pubkey: agentPubkey,
        kind: 43002,
        createdAt: createdAt + 1,
        id: "a2".repeat(32),
        extraTags: [
          ["p", requesterPubkey],
          ["job", jobId],
          ["e", requestEventId],
        ],
      });
      for (const [seq, summary] of [
        [1, "Loaded the JAC-575 receipt"],
        [2, "Applying the verified repair"],
      ] as const) {
        emit({
          channelName: "general",
          content: JSON.stringify({
            schema: 1,
            job: jobId,
            attempt: 1,
            seq,
            state: "running",
            summary,
            artifacts: [],
          }),
          pubkey: agentPubkey,
          kind: 43003,
          createdAt: createdAt + 1 + seq,
          id: (seq === 1 ? "a3" : "a4").repeat(32),
          extraTags: [
            ["p", requesterPubkey],
            ["job", jobId],
            ["e", requestEventId],
            ["seq", String(seq)],
          ],
        });
      }
    },
    {
      agentPubkey: AGENT_PUBKEY,
      createdAt,
      jobId: JOB_ID,
      requestEventId: REQUEST_EVENT_ID,
      requesterPubkey: REQUESTER_PUBKEY,
    },
  );
}

test("managed agent runtime reattaches across Desktop relaunch while its job is running", async ({
  page,
}) => {
  await installMockBridge(page, {
    personas: [
      {
        id: "custom:sage-jac-575",
        displayName: "Sage",
        systemPrompt: "Own governed work until verified completion.",
      },
    ],
    managedAgents: [
      {
        pubkey: AGENT_PUBKEY,
        name: "Sage",
        personaId: "custom:sage-jac-575",
        status: "running",
        channelNames: ["general"],
      },
    ],
    managedAgentRuntimes: [runtimeStatus("ready")],
  });

  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await Promise.all(
    [43_001, 43_002, 43_003].map((kind) =>
      waitForMockLiveSubscription(page, "general", kind),
    ),
  );
  await emitRunningJob(page);

  const jobCard = page.getByTestId(`agent-job-${JOB_ID}`);
  await expect(jobCard).toBeVisible();
  await expect(jobCard).toHaveAttribute("data-job-state", "running");
  await expect(jobCard).toHaveAttribute("data-progress-seq", "2");
  await expect(jobCard).toContainText("Applying the verified repair");
  await expect(jobCard).not.toContainText("Loaded the JAC-575 receipt");
  await expect(
    jobCard.getByRole("button", { name: `Cancel job ${JOB_ID}` }),
  ).toBeEnabled();

  await page.getByTestId("open-agents-view").click();
  const agentRow = page.getByTestId("persona-agent-row-custom:sage-jac-575");
  await expect(agentRow).toBeVisible();
  await agentRow.click();
  await expect(page.getByTestId("user-profile-panel")).toBeVisible();
  await page.getByTestId(`user-profile-view-activity-${AGENT_PUBKEY}`).click();
  const runtimePanel = page.getByTestId("agent-session-thread-panel");
  const runtimeSummary = runtimePanel.getByLabel("Persistent runtime status");
  await expect(runtimePanel).toBeVisible();
  await expect(runtimeSummary).toHaveAttribute(
    "data-runtime-pid",
    String(RUNTIME_PID),
  );
  await expect(runtimeSummary).toContainText(JOB_ID);
  await expect(runtimeSummary).toHaveAttribute(
    "data-runtime-pubkey",
    AGENT_PUBKEY,
  );
  await expect(runtimeSummary).toHaveAttribute(
    "data-runtime-relay-url",
    RELAY_URL,
  );
  await expect(runtimeSummary).toContainText("working");

  // Tear down and recreate the renderer connection. The bridge state models the
  // detached runtime that outlives Desktop; signed job events are replayed after
  // the new renderer establishes its subscriptions.
  await page.reload({ waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-general").click();
  await Promise.all(
    [43_001, 43_002, 43_003].map((kind) =>
      waitForMockLiveSubscription(page, "general", kind),
    ),
  );
  await emitRunningJob(page);
  await expect(jobCard).toHaveAttribute("data-job-state", "running");
  await expect(jobCard).toHaveAttribute("data-progress-seq", "2");
  await page.getByTestId("open-agents-view").click();
  await agentRow.click();
  await page.getByTestId(`user-profile-view-activity-${AGENT_PUBKEY}`).click();
  await expect(runtimeSummary).toHaveAttribute(
    "data-runtime-pid",
    String(RUNTIME_PID),
  );
  await expect(runtimeSummary).toContainText(JOB_ID);
  await expect(runtimeSummary).toContainText("working");

  await page.evaluate(() => {
    const win = window as Window & {
      __BUZZ_RUNTIME_REATTACH_SAMPLES__?: Array<{
        lifecycle: string | null;
        text: string;
      }>;
      __BUZZ_RUNTIME_REATTACH_OBSERVER__?: MutationObserver;
    };
    const samples: Array<{ lifecycle: string | null; text: string }> = [];
    const record = () => {
      const runtime = document.querySelector(
        '[aria-label="Persistent runtime status"]',
      );
      if (!runtime) return;
      samples.push({
        lifecycle: runtime.getAttribute("data-runtime-lifecycle"),
        text: runtime.textContent ?? "",
      });
    };
    const observer = new MutationObserver(record);
    observer.observe(document.body, {
      childList: true,
      subtree: true,
      characterData: true,
    });
    win.__BUZZ_RUNTIME_REATTACH_SAMPLES__ = samples;
    win.__BUZZ_RUNTIME_REATTACH_OBSERVER__ = observer;
    record();
  });

  await page.evaluate(async (runtime) => {
    const setRuntime = window.__BUZZ_E2E_SET_MOCK_MANAGED_AGENT_RUNTIME__;
    if (!setRuntime)
      throw new Error("Managed runtime mock control is unavailable.");
    await setRuntime(runtime);
  }, runtimeStatus("recovering"));
  await expect(runtimeSummary).toHaveAttribute(
    "data-runtime-lifecycle",
    "recovering",
  );
  await expect(runtimeSummary).toContainText("Recovering");
  await expect(runtimeSummary).toHaveAttribute(
    "data-runtime-pid",
    String(RUNTIME_PID),
  );
  await expect(runtimeSummary).toContainText(JOB_ID);

  await page.evaluate(async (runtime) => {
    const setRuntime = window.__BUZZ_E2E_SET_MOCK_MANAGED_AGENT_RUNTIME__;
    if (!setRuntime)
      throw new Error("Managed runtime mock control is unavailable.");
    await setRuntime(runtime);
  }, runtimeStatus("ready"));
  await expect(runtimeSummary).toHaveAttribute(
    "data-runtime-lifecycle",
    "ready",
  );
  await expect(runtimeSummary).toContainText("working");
  await expect(runtimeSummary).toHaveAttribute(
    "data-runtime-pid",
    String(RUNTIME_PID),
  );
  await expect(runtimeSummary).toContainText(JOB_ID);
  await expect(runtimeSummary).toHaveAttribute(
    "data-runtime-pubkey",
    AGENT_PUBKEY,
  );
  await expect(runtimeSummary).toHaveAttribute(
    "data-runtime-relay-url",
    RELAY_URL,
  );

  const transitionSamples = await page.evaluate(() => {
    const win = window as Window & {
      __BUZZ_RUNTIME_REATTACH_SAMPLES__?: Array<{
        lifecycle: string | null;
        text: string;
      }>;
      __BUZZ_RUNTIME_REATTACH_OBSERVER__?: MutationObserver;
    };
    win.__BUZZ_RUNTIME_REATTACH_OBSERVER__?.disconnect();
    return win.__BUZZ_RUNTIME_REATTACH_SAMPLES__ ?? [];
  });
  expect(transitionSamples.length).toBeGreaterThan(0);
  expect(
    transitionSamples.some(({ lifecycle }) => lifecycle === "recovering"),
  ).toBe(true);
  expect(transitionSamples.at(-1)?.lifecycle).toBe("ready");
  expect(
    transitionSamples.every(
      ({ lifecycle }) => lifecycle === "ready" || lifecycle === "recovering",
    ),
  ).toBe(true);
  expect(
    transitionSamples.every(({ text }) => !/offline|stopped/i.test(text)),
  ).toBe(true);

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(jobCard).toBeVisible();
  await expect(jobCard).toHaveAttribute("data-progress-seq", "2");
  await expect(jobCard).toContainText(JOB_ID);

  await jobCard.getByRole("button", { name: `Cancel job ${JOB_ID}` }).click();
  await expect
    .poll(() =>
      page.evaluate(
        ({ agentPubkey, channelId, jobId, requestEventId }) =>
          window.__BUZZ_E2E_SIGNED_EVENTS__?.some(
            (event) =>
              event.kind === 43005 &&
              event.content ===
                JSON.stringify({
                  schema: 1,
                  job: jobId,
                  reason: "Cancelled from Buzz Desktop",
                }) &&
              event.tags.some(
                (tag) => tag[0] === "h" && tag[1] === channelId,
              ) &&
              event.tags.some(
                (tag) => tag[0] === "p" && tag[1] === agentPubkey,
              ) &&
              event.tags.some((tag) => tag[0] === "job" && tag[1] === jobId) &&
              event.tags.some(
                (tag) => tag[0] === "e" && tag[1] === requestEventId,
              ),
          ) ?? false,
        {
          agentPubkey: AGENT_PUBKEY,
          channelId: CHANNEL_ID,
          jobId: JOB_ID,
          requestEventId: REQUEST_EVENT_ID,
        },
      ),
    )
    .toBe(true);
});
