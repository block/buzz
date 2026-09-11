import { expect, type Page } from "@playwright/test";
import { TEST_IDENTITIES } from "./bridge";

export const WELCOME_COLLISION = {
  pubkey: "c".repeat(64),
  name: "Fizz",
  personaId: "builtin:fizz",
  status: "stopped" as const,
};

/** Add a deliberate same-name member and deliver the relay event mock IPC omits. */
export async function addWelcomeCollision(page: Page, channelId: string) {
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: "Welcome",
          }) ?? false,
      ),
    )
    .toBe(true);
  await page.evaluate(
    async ({ channelId, pubkey, actor }) => {
      const invoke = window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
      const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      if (!invoke || !emit)
        throw new Error("Mock membership seams unavailable");
      const result = (await invoke("add_channel_members", {
        channelId,
        pubkeys: [pubkey],
        role: "bot",
      })) as { added: string[]; errors: unknown[] };
      if (result.errors.length || !result.added.includes(pubkey)) {
        throw new Error("Collision fixture membership was not added");
      }
      // Real relay side_effects emits member_joined after kind:9000. Mock
      // add_channel_members only mutates backend arrays. Model delivery, not
      // the resulting QueryClient state: production owns roster invalidation.
      emit({
        channelName: "Welcome",
        kind: 40099,
        content: JSON.stringify({
          type: "member_joined",
          actor,
          target: pubkey,
        }),
      });
    },
    {
      channelId,
      pubkey: WELCOME_COLLISION.pubkey,
      actor: TEST_IDENTITIES.tyler.pubkey,
    },
  );
  await expect
    .poll(() =>
      page.evaluate(
        ({ channelId, pubkey }) => {
          const client = window.__BUZZ_E2E_QUERY_CLIENT__ as unknown as {
            getQueryData: (key: string[]) => { pubkey: string }[] | undefined;
          };
          return client
            .getQueryData(["channels", channelId, "members"])
            ?.some((member) => member.pubkey === pubkey);
        },
        { channelId, pubkey: WELCOME_COLLISION.pubkey },
      ),
    )
    .toBe(true);
}
