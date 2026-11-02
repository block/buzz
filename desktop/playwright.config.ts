import { defineConfig, devices } from "@playwright/test";
import { readFileSync, realpathSync } from "node:fs";
import { isAbsolute, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

// The core journeys CI runs on every change to shared code. The same file maps
// folders to the specs a change needs (see scripts/ci-plan.mjs).
const affectedMap = JSON.parse(
  readFileSync(
    fileURLToPath(new URL("./tests/e2e/affected-map.json", import.meta.url)),
    "utf8",
  ),
) as {
  essential: {
    specs: string[];
    focus: { specs: string[]; titles: string[] };
  };
};
const specMatch = (specs: string[]) => specs.map((spec) => `**/${spec}`);
const titlePattern = (titles: string[]) =>
  new RegExp(
    titles
      .map((title) => title.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"))
      .join("|"),
  );

const canaryEnabled = process.env.BUZZ_E2E_CANARY === "1";
const canaryAccountFile = process.env.BUZZ_E2E_CANARY_ACCOUNT_FILE;
const canaryArtifactDir = process.env.BUZZ_E2E_CANARY_ARTIFACT_DIR;
const appPort = canaryEnabled
  ? 4174
  : Number(process.env.COLONY_E2E_PORT ?? 4173);

function assertOutsideRepository(path: string, label: string) {
  const repositoryRoot = realpathSync(resolve(process.cwd(), ".."));
  const actualPath = realpathSync(path);
  const relativePath = relative(repositoryRoot, actualPath);
  if (
    !relativePath ||
    (relativePath !== ".." && !relativePath.startsWith(`..${sep}`))
  ) {
    throw new Error(`${label} must be outside the repository.`);
  }
}

if (canaryEnabled) {
  if (process.env.CI) {
    throw new Error("The canary Playwright project is local-only.");
  }
  if (!canaryAccountFile || !canaryArtifactDir) {
    throw new Error(
      "The canary project requires its account file and artifact directory environment variables.",
    );
  }
  if (!isAbsolute(canaryAccountFile) || !isAbsolute(canaryArtifactDir)) {
    throw new Error("Canary account and artifact paths must be absolute.");
  }
  const canaryAccountRoot = realpathSync(
    resolve(process.env.HOME ?? "", ".colony-canary"),
  );
  const accountRelativePath = relative(
    canaryAccountRoot,
    realpathSync(canaryAccountFile),
  );
  if (
    !accountRelativePath ||
    accountRelativePath === ".." ||
    accountRelativePath.startsWith(`..${sep}`)
  ) {
    throw new Error("The canary account file must be inside ~/.colony-canary.");
  }
  assertOutsideRepository(canaryAccountFile, "The canary account file");
  assertOutsideRepository(canaryArtifactDir, "The canary artifact directory");
}

const canaryProjects = canaryEnabled
  ? [
      {
        name: "canary",
        testMatch: "**/company-canary.canary.spec.ts",
        retries: 0,
        use: {
          ...devices["Desktop Chrome"],
          viewport: { width: 1440, height: 900 },
          screenshot: "off" as const,
          trace: "off" as const,
          video: "off" as const,
        },
      },
    ]
  : [];

export default defineConfig({
  testDir: "./tests/e2e",
  timeout: 30_000,
  expect: { timeout: canaryEnabled ? 30_000 : 5_000 },
  retries: process.env.CI ? 2 : 0,
  workers: 1,
  outputDir:
    canaryEnabled && canaryArtifactDir
      ? resolve(canaryArtifactDir, "playwright-results")
      : "test-results",
  reporter: canaryEnabled
    ? [["list"]]
    : [
        ["list"],
        ["html", { open: "never", outputFolder: "playwright-report" }],
      ],
  use: {
    baseURL: `http://127.0.0.1:${appPort}`,
    channel: "chrome",
    launchOptions: {
      args: ["--use-mock-keychain", "--password-store=basic"],
    },
    screenshot: "only-on-failure",
    trace: "on-first-retry",
    video: "retain-on-failure",
  },
  projects: [
    {
      name: "smoke",
      testMatch: [
        "**/colony3-*.spec.ts",
        "**/mac-update-cards.spec.ts",
        "**/avatar-discoverability.spec.ts",
        "**/agent-avatar-everywhere.spec.ts",
        "**/activity-row-plain-labels.spec.ts",
        "**/scout-branding.spec.ts",
        "**/workspace-file-links.spec.ts",
        "**/work-area-dock.spec.ts",
        "**/work-area-dock-screenshots.spec.ts",
        "**/work-area-files.spec.ts",
        "**/work-area-canvas.spec.ts",
        "**/work-area-browser.spec.ts",
        "**/browser-profile-forget.spec.ts",
        "**/work-area-browser-agent.spec.ts",
        "**/work-area-browser-agent-auto.spec.ts",
        "**/work-area-browser-agent-show.spec.ts",
        "**/work-area-browser-screenshots.spec.ts",
        "**/work-area-knowledge.spec.ts",
        "**/work-area-pins.spec.ts",
        "**/whatsapp-work-area.spec.ts",
        "**/whatsapp-outreach-desk.spec.ts",
        "**/whatsapp-outreach-desk.visual.spec.ts",
        "**/whatsapp-decided-card.spec.ts",
        "**/whatsapp-polish.spec.ts",
        "**/whatsapp-dock-narrow.spec.ts",
        "**/first-reply-runtime.spec.ts",
        "**/judge-ready-ai.spec.ts",
        "**/connect-polish.spec.ts",
        "**/smoke.spec.ts",
        "**/shell-design.spec.ts",
        "**/account-auth.spec.ts",
        "**/stripe-credits.spec.ts",
        "**/credits-gateway-option.spec.ts",
        "**/start-free-opencode.spec.ts",
        "**/colony-agent-shell-tools.spec.ts",
        "**/start-free-screenshots.spec.ts",
        "**/colony-public-icons.spec.ts",
        "**/openrouter-oauth.spec.ts",
        "**/self-serve-community-onboarding.spec.ts",
        "**/business-website.spec.ts",
        "**/harness-detection.spec.ts",
        "**/owned-agent-discovery.spec.ts",
        "**/agent-profile-instructions.spec.ts",
        "**/thread-head-stale-edit.spec.ts",
        "**/doc-card-collapse.spec.ts",
        "**/doc-card-p10-screenshots.spec.ts",
        "**/sidebar-offcanvas-rail.spec.ts",
        "**/sidebar-full-app.spec.ts",
        "**/workspace-header-icon.spec.ts",
        "**/workspace-switcher.spec.ts",
        "**/sidebar-reference-fixture.spec.ts",
        "**/tooltip-semantics.spec.ts",
        "**/search-scope-screenshots.spec.ts",
        "**/onboarding-docked-cta-screenshots.spec.ts",
        "**/reskin-gate-screenshots.spec.ts",
        "**/onboarding-two-paths-screenshots.spec.ts",
        "**/onboarding-design.spec.ts",
        "**/onboarding-polish.spec.ts",
        "**/onboarding-community-entry.spec.ts",
        "**/identity-key-help.spec.ts",
        "**/advanced-signin.spec.ts",
        "**/exact-key-profile.spec.ts",
        "**/key-import-reveal.spec.ts",
        "**/navigation.spec.ts",
        "**/relay-request-budget.spec.ts",
        "**/channels.spec.ts",
        "**/day-divider-overlap.spec.ts",
        "**/channel-shared-header-backdrop.spec.ts",
        "**/auxiliary-pane-close-visibility.spec.ts",
        "**/channel-composer-overflow.spec.ts",
        "**/badge.spec.ts",
        "**/cofounder-wake-hidden.spec.ts",
        "**/channel-browser.spec.ts",
        "**/channel-add-screenshots.spec.ts",
        "**/add-community-screenshots.spec.ts",
        "**/hosted-communities-settings-screenshots.spec.ts",
        "**/invites-settings-screenshots.spec.ts",
        "**/messaging.spec.ts",
        "**/link-preview-bare-domain.spec.ts",
        "**/bestie.spec.ts",
        "**/message-feedback-snapshots.spec.ts",
        "**/send-feedback-settings.spec.ts",
        "**/message-copy-link.spec.ts",
        "**/custom-emoji.spec.ts",
        "**/profile-custom-emoji-status.spec.ts",
        "**/custom-emoji-ui.spec.ts",
        "**/channel-mute.spec.ts",
        "**/channel-star.spec.ts",
        "**/channel-controls.spec.ts",
        "**/channel-activity-popover.spec.ts",
        "**/agent-working-indicator.spec.ts",
        "**/agent-start-watch.spec.ts",
        "**/mention-wake-after-launch.spec.ts",
        "**/mention-wake-stop-race.spec.ts",
        "**/agent-working-indicator-screenshots.spec.ts",
        "**/thread-open-tint.spec.ts",
        "**/thread-open-tint-screenshots.spec.ts",
        "**/thread-reply-spacing.spec.ts",
        "**/active-turn-resilience.spec.ts",
        "**/profile-active-turn.spec.ts",
        "**/welcome-agent-modal-screenshots.spec.ts",
        "**/voice-note.spec.ts",
        "**/agent-readiness-screenshots.spec.ts",
        "**/edit-agent.spec.ts",
        "**/doctor-cta-screenshots.spec.ts",
        "**/pubkey-display-screenshots.spec.ts",
        "**/file-attachment.spec.ts",
        "**/image-attachment-gallery.spec.ts",
        "**/composer-image-draw.spec.ts",
        "**/video-attachment.spec.ts",
        "**/spoiler.spec.ts",
        "**/composer-link-shortcut.spec.ts",
        "**/entity-link-recipient-cards.spec.ts",
        "**/composer-selection-formatting.spec.ts",
        "**/composer-tooltip-dismiss.spec.ts",
        "**/mentions.spec.ts",
        "**/mention-spacing.spec.ts",
        "**/mention-clipboard.spec.ts",
        "**/cloud-provenance.spec.ts",
        "**/mention-recipients.spec.ts",
        "**/remote-owned-mentions.spec.ts",
        "**/team-mentions.spec.ts",
        "**/persistent-agent-audience.spec.ts",
        "**/relay-reconnect.spec.ts",
        "**/relay-reconnect-affordance.spec.ts",
        "**/company-asks.spec.ts",
        "**/cofounder-customer-record.spec.ts",
        "**/company-hiring.spec.ts",
        "**/company-team.spec.ts",
        "**/company-team-design.spec.ts",
        "**/company-team-scout.spec.ts",
        "**/asks-2.spec.ts",
        "**/identity-archive.spec.ts",
        "**/identity-archive-hide.spec.ts",
        "**/relay-connectivity.spec.ts",
        "**/unread-pill.spec.ts",
        "**/sidebar-more-unread-overlap.spec.ts",
        "**/sidebar-snapshot.spec.ts",
        "**/home-collapsed-top-chrome.spec.ts",
        "**/top-chrome-zoom-clearance.spec.ts",
        "**/thread-unread.spec.ts",
        "**/thread-load-failure.spec.ts",
        "**/workspace-rail.spec.ts",
        "**/community-rail.spec.ts",
        "**/community-apply-error.spec.ts",
        "**/removed-member-escape.spec.ts",
        "**/boot-splash.spec.ts",
        "**/thread-reply-anchor-roleplay.spec.ts",
        "**/threadpane-ultrawide.spec.ts",
        "**/thread-focus-mode.spec.ts",
        "**/animated-avatar.spec.ts",
        "**/virtualization.spec.ts",
        "**/scroll-history.spec.ts",
        "**/channel-dense-second-reach.spec.ts",
        "**/channel-window-mock-paging.spec.ts",
        "**/channel-head-restart.spec.ts",
        "**/live-broadcast-reply-timeline.spec.ts",
        "**/markdown-parse-cache.spec.ts",
        "**/markdown-tables.spec.ts",
        "**/overscroll-boundary.spec.ts",
        "**/terminal-wheel.spec.ts",
        "**/cold-switch-longtask.perf.ts",
        "**/timeline-no-shift.spec.ts",
        "**/human-edit-agent-content.spec.ts",
        "**/empty-edit-delete.spec.ts",
        "**/reaction-order.spec.ts",
        "**/reaction-names.spec.ts",
        "**/inbox-reactions.spec.ts",
        "**/inbox-edit.spec.ts",
        "**/send-channel-binding.spec.ts",
        "**/persona-model-combobox-screenshots.spec.ts",
        "**/drafts-screenshots.spec.ts",
        "**/drafts-all-fix-screenshots.spec.ts",
        "**/inbox-refactor-screenshots.spec.ts",
        "**/inbox-title-overlap.spec.ts",
        "**/message-author-overlap.spec.ts",
        "**/buzz-theme-screenshots.spec.ts",
        "**/appearance-previews.spec.ts",
        "**/appearance-chrome.spec.ts",
        "**/channel-sort.spec.ts",
        "**/identity-lost.spec.ts",
        "**/deep-link-invite.spec.ts",
        "**/invited-onboarding.spec.ts",
        "**/invited-onboarding-screenshots.spec.ts",
        "**/invite-link-copy.spec.ts",
        "**/global-agent-config-screenshots.spec.ts",
        "**/doctor-states.spec.ts",
        "**/onboarding-avatar-skip.spec.ts",
        "**/onboarding-backup.spec.ts",
        "**/onboarding-agent-defaults.spec.ts",
        "**/nostr-bind.spec.ts",
        "**/mobile-pairing-qr.spec.ts",
        "**/profile-nsec-reveal.spec.ts",
        "**/profile-backup-settings.spec.ts",
        "**/signout-confirmation.spec.ts",
        "**/settings-section-layout.spec.ts",
        "**/experimental-features.spec.ts",
        "**/agent-provider-dropdowns.spec.ts",
        "**/agent-lifecycle-feedback.spec.ts",
        "**/agent-access-warning.spec.ts",
        "**/edit-agent-run-on.spec.ts",
        "**/inbox-live-update.spec.ts",
        "**/harness-management.spec.ts",
        "**/harness-catalog-screenshots.spec.ts",
        "**/inline-custom-harness.spec.ts",
        "**/where-to-run-config.spec.ts",
        "**/agent-numeric-tuning.spec.ts",
        "**/needs-restart-screenshots.spec.ts",
        "**/w07-agents-smoke.spec.ts",
        "**/w10-discovery-sales.spec.ts",
        "**/w11-clients-work.spec.ts",
        "**/money.spec.ts",
        "**/cofounder-map.spec.ts",
        "**/cofounder-customer-record.spec.ts",
        "**/cofounder-phase2.spec.ts",
        "**/cofounder-playbooks.spec.ts",
        "**/leads-table-polish.spec.ts",
        "**/whatsapp-outreach.spec.ts",
        "**/whatsapp-outreach.visual.spec.ts",
      ],
      use: {
        ...devices["Desktop Chrome"],
      },
    },
    {
      name: "essential",
      testMatch: specMatch(affectedMap.essential.specs),
      use: {
        ...devices["Desktop Chrome"],
      },
    },
    {
      name: "essential-focus",
      testMatch: specMatch(affectedMap.essential.focus.specs),
      grep: titlePattern(affectedMap.essential.focus.titles),
      use: {
        ...devices["Desktop Chrome"],
      },
    },
    {
      name: "integration",
      testMatch: [
        "**/colony3-discovery-logo-prospect-proof.spec.ts",
        "**/colony3-agents-simple.spec.ts",
        "**/colony3-discovery-v4.spec.ts",
        "**/colony3-discovery-tags.spec.ts",
        "**/day-divider-overlap.spec.ts",
        "**/colony3-lead-reference.spec.ts",
        "**/link-preview-bare-domain.spec.ts",
        "**/doc-card-collapse.spec.ts",
        "**/doc-card-p10-screenshots.spec.ts",
        "**/colony3-dark-mode.spec.ts",
        "**/judge-ready-ai.spec.ts",
        "**/agent-working-indicator.spec.ts",
        "**/agent-start-watch.spec.ts",
        "**/mention-wake-after-launch.spec.ts",
        "**/mention-wake-stop-race.spec.ts",
        "**/thread-open-tint.spec.ts",
        "**/thread-reply-spacing.spec.ts",
        "**/avatar-discoverability.spec.ts",
        "**/agent-avatar-everywhere.spec.ts",
        "**/activity-row-plain-labels.spec.ts",
        "**/workspace-file-links.spec.ts",
        "**/work-area-dock.spec.ts",
        "**/work-area-dock-screenshots.spec.ts",
        "**/work-area-files.spec.ts",
        "**/work-area-canvas.spec.ts",
        "**/work-area-browser.spec.ts",
        "**/browser-profile-forget.spec.ts",
        "**/work-area-browser-agent.spec.ts",
        "**/work-area-browser-agent-auto.spec.ts",
        "**/work-area-browser-agent-show.spec.ts",
        "**/work-area-browser-screenshots.spec.ts",
        "**/work-area-knowledge.spec.ts",
        "**/work-area-pins.spec.ts",
        "**/whatsapp-work-area.integration.spec.ts",
        "**/whatsapp-outreach-desk.spec.ts",
        "**/whatsapp-decided-card.spec.ts",
        "**/whatsapp-polish.spec.ts",
        "**/whatsapp-dock-narrow.spec.ts",
        "**/account-auth.spec.ts",
        "**/advanced-signin.spec.ts",
        "**/stripe-credits.spec.ts",
        "**/credits-gateway-option.spec.ts",
        "**/start-free-opencode.spec.ts",
        "**/colony-agent-shell-tools.spec.ts",
        "**/colony-public-icons.spec.ts",
        "**/onboarding-agent-defaults.spec.ts",
        "**/appearance-chrome.spec.ts",
        "**/deep-link-invite.spec.ts",
        "**/invited-onboarding.spec.ts",
        "**/invited-onboarding-screenshots.spec.ts",
        "**/first-reply-runtime.spec.ts",
        "**/connect-polish.spec.ts",
        "**/agents.spec.ts",
        "**/agent-availability.spec.ts",
        "**/agent-snapshot-recipient.spec.ts",
        "**/onboarding.spec.ts",
        "**/business-website.spec.ts",
        "**/onboarding-design.spec.ts",
        "**/onboarding-polish.spec.ts",
        "**/onboarding-community-entry.spec.ts",
        "**/openrouter-oauth.spec.ts",
        "**/harness-detection.spec.ts",
        "**/onboarding-two-paths-screenshots.spec.ts",
        "**/stream.spec.ts",
        "**/integration.spec.ts",
        "**/work-area-canvas.integration.spec.ts",
        "**/company-asks.live.spec.ts",
        "**/company-team.integration.spec.ts",
        "**/company-team.spec.ts",
        "**/company-team-design.spec.ts",
        "**/company-team-scout.spec.ts",
        "**/company-team.live.spec.ts",
        "**/dm-double-notification.spec.ts",
        "**/profile.spec.ts",
        "**/sidebar.spec.ts",
        "**/workspace-header-icon.spec.ts",
        "**/workspace-switcher.spec.ts",
        "**/mac-update-cards.spec.ts",
        "**/shell-design.spec.ts",
        "**/community-apply-error.spec.ts",
        "**/removed-member-escape.spec.ts",
        "**/sidebar-relay-card.spec.ts",
        "**/tokens.spec.ts",
        "**/persona-env-vars.spec.ts",
        "**/persona-sync.spec.ts",
        "**/agents-everywhere.live.spec.ts",
        "**/relay-restart.live.spec.ts",
        "**/parity-ancestor-island.spec.ts",
        "**/colony3-sales-setup.spec.ts",
        "**/colony3-scout-dupes.spec.ts",
        "**/cofounder-map.spec.ts",
        "**/cofounder-phase2.spec.ts",
        "**/cofounder-playbooks.spec.ts",
        "**/whatsapp-outreach.integration.spec.ts",
      ],
      use: {
        ...devices["Desktop Chrome"],
      },
      expect: {
        timeout: process.env.CI ? 15_000 : 10_000,
      },
    },
    ...canaryProjects,
  ],
  webServer: {
    command: `python3 -m http.server ${appPort} -d dist`,
    cwd: ".",
    reuseExistingServer: !process.env.CI,
    url: `http://127.0.0.1:${appPort}`,
  },
});
