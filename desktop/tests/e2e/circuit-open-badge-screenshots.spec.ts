/**
 * Screenshot spec for the persistent circuit-breaker "Suspended" badge on the
 * real Agents view (block/buzz#5888).
 *
 * An adversarial review of this feature found the badge had first been wired
 * into ManagedAgentRow/AgentGroupRows — a component tree with no reachable
 * route in the shipped app — so it had zero real-world effect. It was
 * relocated to StandaloneAgentCard/AgentPersonaCard in UnifiedAgentsSection,
 * the actual cards this repo's own "Agents" screen renders. This spec proves
 * the badge renders there, not just that the underlying store logic is
 * correct (the unit tests in observerRelayStore.circuitStatus.test.mjs cover
 * that layer already).
 *
 * Exercises:
 *   - Badge absent while the circuit is closed.
 *   - Badge appears on a circuit_open event, with an accessible tooltip
 *     (Tooltip/TooltipTrigger, not a bare `title` attribute — see the
 *     RestartDiffBadge precedent this reuses).
 *   - Badge disappears on a matching circuit_recovered event.
 *   - Same coverage on the persona-linked card variant.
 */

import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const SHOTS = "test-results/circuit-open-badge-screenshots";

const STANDALONE_AGENT = {
  pubkey: TEST_IDENTITIES.alice.pubkey,
  name: "Local Agent",
  status: "running" as const,
};

const PERSONA_AGENT = {
  pubkey: TEST_IDENTITIES.bob.pubkey,
  name: "Persona Agent",
  personaId: "builtin:fizz",
  status: "running" as const,
};

async function waitForSeedHook(page: import("@playwright/test").Page) {
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__ === "function",
    null,
    { timeout: 10_000 },
  );
}

async function gotoAgentsView(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForSeedHook(page);
  await expect(page.getByTestId("open-agents-view")).toBeVisible({
    timeout: 10_000,
  });
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("agents-library-personas")).toBeVisible({
    timeout: 10_000,
  });
}

async function seedObserverEvents(
  page: import("@playwright/test").Page,
  agentPubkey: string,
  events: Array<{
    seq: number;
    timestamp: string;
    kind: string;
    agentIndex: number | null;
    channelId: string | null;
    sessionId: string | null;
    turnId: string | null;
    payload: unknown;
  }>,
) {
  await page.evaluate(
    ({ pubkey, evts }) => {
      window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__?.({
        agentPubkey: pubkey,
        events: evts,
      });
    },
    { pubkey: agentPubkey, evts: events },
  );
  // Let React re-render after the store update.
  await page.waitForTimeout(300);
}

// No cooldown_secs in the payload — keeps the badge on its static
// "Suspended — repeated crashes" label rather than a live countdown, so the
// screenshot and text assertions aren't timing-sensitive.
function circuitOpenEvent(overrides: { channelId?: string | null } = {}) {
  return {
    seq: 1,
    timestamp: new Date().toISOString(),
    kind: "circuit_open",
    agentIndex: 0,
    channelId: overrides.channelId ?? null,
    sessionId: null,
    turnId: null,
    payload: {
      error:
        "Agent slot 0 panicked repeatedly and its circuit breaker is now open.",
    },
  };
}

function circuitRecoveredEvent() {
  return {
    seq: 2,
    timestamp: new Date(Date.now() + 1000).toISOString(),
    kind: "circuit_recovered",
    agentIndex: 0,
    channelId: null,
    sessionId: null,
    turnId: null,
    payload: { error: "Agent slot 0 recovered." },
  };
}

test.describe("circuit-open badge screenshots", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (err) => {
      console.error(
        "PAGE ERROR:",
        err.message,
        err.stack?.split("\n").slice(0, 5).join("\n"),
      );
    });
  });

  test("01-standalone-card-circuit-open-and-recover", async ({ page }) => {
    await installMockBridge(page, { managedAgents: [STANDALONE_AGENT] });
    await gotoAgentsView(page);

    const agentCard = page.getByTestId(
      `managed-agent-${STANDALONE_AGENT.pubkey}`,
    );
    await expect(agentCard).toBeVisible({ timeout: 10_000 });
    const badge = agentCard.getByTestId(
      `managed-agent-circuit-open-${STANDALONE_AGENT.pubkey}`,
    );

    // Closed by default — no badge.
    await expect(badge).toHaveCount(0);

    await seedObserverEvents(page, STANDALONE_AGENT.pubkey, [
      circuitOpenEvent(),
    ]);

    await expect(badge).toBeVisible({ timeout: 5_000 });
    await expect(badge).toHaveText("Suspended — repeated crashes");

    // Accessible tooltip: keyboard-focusable (tabIndex), Tooltip primitive
    // rather than a bare `title` attribute.
    await badge.hover();
    const tooltip = page.locator("[role=tooltip]");
    await expect(tooltip).toBeVisible({ timeout: 5_000 });
    await expect(tooltip).toHaveText(
      "Agent slot 0 panicked repeatedly and its circuit breaker is now open.",
    );

    await waitForAnimations(page);
    await agentCard.screenshot({
      path: `${SHOTS}/01-standalone-card-circuit-open.png`,
    });

    await seedObserverEvents(page, STANDALONE_AGENT.pubkey, [
      circuitRecoveredEvent(),
    ]);

    await expect(badge).toHaveCount(0);
  });

  test("02-persona-card-circuit-open-and-recover", async ({ page }) => {
    await installMockBridge(page, {
      activePersonaIds: [PERSONA_AGENT.personaId],
      managedAgents: [PERSONA_AGENT],
    });
    await gotoAgentsView(page);

    const personaCard = page.getByTestId(
      `persona-agent-row-${PERSONA_AGENT.personaId}`,
    );
    await expect(personaCard).toBeVisible({ timeout: 10_000 });
    const badge = personaCard.getByTestId(
      `managed-agent-circuit-open-${PERSONA_AGENT.pubkey}`,
    );

    await expect(badge).toHaveCount(0);

    await seedObserverEvents(page, PERSONA_AGENT.pubkey, [
      circuitOpenEvent({ channelId: "94a444a4-c0a3-5966-ab05-530c6ddc2301" }),
    ]);

    await expect(badge).toBeVisible({ timeout: 5_000 });
    await expect(badge).toHaveText("Suspended — repeated crashes");

    await waitForAnimations(page);
    await personaCard.screenshot({
      path: `${SHOTS}/02-persona-card-circuit-open.png`,
    });

    await seedObserverEvents(page, PERSONA_AGENT.pubkey, [
      circuitRecoveredEvent(),
    ]);

    await expect(badge).toHaveCount(0);
  });
});
