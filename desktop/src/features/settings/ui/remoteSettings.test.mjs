import assert from "node:assert/strict";
import test from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { FixedCommunityProvider } from "@/features/communities/useCommunities";
import { bindNativeIdentity } from "@/shared/api/nativeIdentitySession";
import { canonicalNpub } from "@/shared/lib/pubkey";
import { remoteSettingsSectionAvailable } from "../lib/remoteSettings.ts";
import { renderSettingsSection, settingsSections } from "./SettingsPanels.tsx";
import { ProfileSettingsCard } from "./ProfileSettingsCard.tsx";

// Mount the real profile editor without an EncryptedBackupProvider. Remote
// custody must not replace the editor or hide unrelated settings sections.
test("remote settings retain the profile editor and all non-custody sections", () => {
  const pubkey = "a".repeat(64);
  const props = { currentPubkey: pubkey };
  assert.equal(
    renderSettingsSection("profile", props).type,
    ProfileSettingsCard,
  );
  bindNativeIdentity({
    mode: "remote",
    authState: "authenticated",
    generation: 9,
    workspaceActive: true,
    publicIdentity: pubkey,
  });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  const html = renderToStaticMarkup(
    createElement(
      QueryClientProvider,
      { client },
      createElement(
        FixedCommunityProvider,
        {
          community: {
            id: "staging",
            name: "Staging",
            relayUrl: "wss://example.test",
            pubkey,
            addedAt: "2026-09-13",
          },
        },
        renderSettingsSection("profile", props),
      ),
    ),
  );
  assert.match(html, /remote-key-custody/);
  assert.ok(html.includes(canonicalNpub(pubkey)));
  assert.match(html, /<form/);
  assert.match(html, /Edit profile/);
  assert.doesNotMatch(html, /Delete my data|settings-signout/);
  for (const { value } of settingsSections) {
    assert.equal(
      remoteSettingsSectionAvailable(value),
      value !== "mobile",
      value,
    );
  }
  assert.match(
    renderToStaticMarkup(renderSettingsSection("mobile", props)),
    /transfers your private key/,
  );
  client.clear();
});
