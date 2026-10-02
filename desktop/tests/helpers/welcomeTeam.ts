import { expect, type Page } from "@playwright/test";

/** Invoke the mock command boundary without seeding product query caches. */
export async function invokeMockCommand<T>(
  page: Page,
  command: string,
  payload?: Record<string, unknown>,
) {
  return page.evaluate(
    async ({ command, payload }) => {
      const bridgeWindow = window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
        __TAURI_INTERNALS__?: {
          invoke?: (
            command: string,
            payload?: Record<string, unknown>,
          ) => Promise<unknown>;
        };
      };
      const invoke =
        bridgeWindow.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ ??
        bridgeWindow.__TAURI_INTERNALS__?.invoke;

      if (!invoke) {
        throw new Error("Mock invoke bridge is unavailable.");
      }

      return (await invoke(command, payload)) as T;
    },
    { command, payload },
  );
}

type WelcomeAgent = {
  pubkey: string;
  persona_id: string;
  status: string;
};
// Pollen retains the builtin:bumble persona identity.
const WELCOME_PERSONAS = ["builtin:bumble", "builtin:fizz", "builtin:honey"];

/** Wait for bootstrap identities so a scenario can author their relay events. */
export async function waitForWelcomeTeam(page: Page): Promise<WelcomeAgent[]> {
  let team: WelcomeAgent[] = [];
  await expect
    .poll(async () => {
      const agents = await invokeMockCommand<WelcomeAgent[]>(
        page,
        "list_managed_agents",
      );
      const { channels } = await invokeMockCommand<{
        channels: Array<{ id: string; name: string }>;
      }>(page, "get_channels");
      const welcome = channels.find((channel) => channel.name === "Welcome");
      if (!welcome) return [];
      const { members } = await invokeMockCommand<{
        members: Array<{ pubkey: string }>;
      }>(page, "get_channel_members", { channelId: welcome.id });
      // Select the started Welcome members, not inactive seeded siblings.
      // This only identifies the event authors; Start does not publish presence.
      team = agents.filter(
        (agent) =>
          agent.status === "running" &&
          members.some((member) => member.pubkey === agent.pubkey) &&
          WELCOME_PERSONAS.includes(agent.persona_id),
      );
      return team.map((agent) => `${agent.persona_id}:${agent.status}`).sort();
    })
    .toEqual(WELCOME_PERSONAS.map((persona) => `${persona}:running`));
  return team;
}

/** Author healthy Welcome-team presence explicitly, independently of Start. */
export async function publishWelcomeTeamPresence(page: Page) {
  const team = await waitForWelcomeTeam(page);
  await page.evaluate(
    (pubkeys) => {
      const emit = window.__BUZZ_E2E_EMIT_MOCK_PRESENCE__;
      if (!emit) throw new Error("Mock presence emitter is unavailable.");
      for (const pubkey of pubkeys) emit({ pubkey, status: "online" });
    },
    team.map((agent) => agent.pubkey),
  );
}
