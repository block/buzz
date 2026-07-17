import { expect, test } from "@playwright/test";

test("announcement demo loads its workspace, people, and projects", async ({
  page,
}) => {
  test.setTimeout(75_000);
  const agentReply =
    "I’d lead with the handoff moment, then land on the shared launch room. That gives the story a clear before-and-after.";
  const fizzReply =
    "Three beats: hold on the send, cut on the mobile notification, then land in the shared launch room. @Honey, can you make that camera-ready? ✨";
  const honeyReply =
    "Frame it as one thought moving with you: send, surface, arrive together. @Bumble, can you sanity-check the sequence? 🍯";
  const bumbleReply =
    "The sequence tracks. Keep the notification visible for a full beat before the cut, and the handoff will read without narration. 🐝🔎";
  const engineeringBumbleReply =
    "The duplicate points to the previous subscription generation delivering once after reconnect. @Fizz, can you turn that into the smallest safe guard?";
  const engineeringFizzReply =
    "I’d reject callbacks whose generation no longer matches the active subscription, then add a sleep-wake regression case. @Honey, can you package the rollout check?";
  const engineeringHoneyReply =
    "Ship the guard behind the existing reconnect path, run the twenty-cycle soak, and call out duplicate delivery in the release checklist. That closes the loop cleanly.";
  const engineeringHumanClose =
    "Perfect. That gives me the patch and the verification path — I’m on it.";
  const humanClose =
    "That’s the version. I can cut to that — nice swarm work 🐝";
  const fizzReplyVisible = fizzReply.replace("@Honey", "Honey");
  const honeyReplyVisible = honeyReply.replace("@Bumble", "Bumble");
  await page.route("**/__announcement-demo/agent-response", async (route) => {
    const body = route.request().postDataJSON() as {
      apiKey?: string;
      messages?: Array<{ content?: string }>;
      model?: string;
      provider?: string;
      systemPrompt?: string;
    };
    expect(body.provider).toBe("openai");
    expect(body.apiKey).toBe("e2e-demo-key");
    expect(body.model).toBe("gpt-5.4-mini");
    expect(body.systemPrompt).toMatch(/You are (Fizz|Honey|Bumble)/);
    const isCollaborativeTurn = body.systemPrompt?.includes(
      "collaborative team turn",
    );
    const isFinalTurn = body.systemPrompt?.includes(
      "final turn in a short agent collaboration",
    );
    const isEngineering = body.systemPrompt?.includes("in #engineering");
    const text = isEngineering
      ? body.systemPrompt?.includes("You are Bumble") && isCollaborativeTurn
        ? engineeringBumbleReply
        : body.systemPrompt?.includes("You are Fizz") && isCollaborativeTurn
          ? engineeringFizzReply
          : engineeringHoneyReply
      : body.systemPrompt?.includes("You are Fizz") && isCollaborativeTurn
        ? fizzReply
        : body.systemPrompt?.includes("You are Honey") && isCollaborativeTurn
          ? honeyReply
          : body.systemPrompt?.includes("You are Bumble") && isFinalTurn
            ? bumbleReply
            : agentReply;
    await route.fulfill({
      json: { text },
    });
  });
  await page.goto("/?demo=announcement");
  await page.evaluate(() => {
    const config = (
      window as Window & {
        __BUZZ_E2E__?: { mock?: { announcementDemoStory?: boolean } };
      }
    ).__BUZZ_E2E__;
    if (config?.mock) config.mock.announcementDemoStory = false;
  });
  await page.waitForFunction(
    () =>
      typeof (
        window as Window & {
          __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown;
        }
      ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__ === "function",
  );

  const relayHttpUrl = await page.evaluate(async () => {
    const invoke = (
      window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload?: unknown,
        ) => Promise<unknown>;
      }
    ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
    if (!invoke) {
      throw new Error("Announcement demo command bridge is unavailable.");
    }
    await invoke("set_global_agent_config", {
      config: {
        env_vars: { OPENAI_COMPAT_API_KEY: "e2e-demo-key" },
        provider: "openai",
        model: "gpt-5.4-mini",
      },
    });
    return invoke("get_relay_http_url");
  });
  expect(relayHttpUrl).toBe(new URL(page.url()).origin);

  await expect(page.getByText("The Hive", { exact: true })).toBeVisible();
  await expect(page.getByText("Product", { exact: true })).toBeVisible();
  await expect(page.getByText("Launch Swarm", { exact: true })).toBeVisible();
  await expect(
    page.getByText("Honeycomb Studios", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Alex Rivera", { exact: true }).last(),
  ).toBeVisible();
  await expect(
    page.getByTestId("channel-DM").filter({ hasText: "Maya Chen" }),
  ).toBeVisible();
  await expect(
    page.getByTestId("channel-DM").filter({ hasText: "Jordan Brooks" }),
  ).toBeVisible();
  await expect(
    page.getByTestId("channel-DM").filter({ hasText: "Priya Shah" }),
  ).toBeVisible();
  await expect(page.getByTestId("channel-DM")).toHaveCount(3);

  const unreadChannels = [
    "announcements",
    "engineering",
    "design",
    "marketing",
    "queen-bee-launch",
  ] as const;
  const readChannels = [
    "general",
    "flight-path",
    "mobile",
    "product-ideas",
    "launch-notes",
  ] as const;
  for (const channel of unreadChannels) {
    await expect(
      page
        .getByTestId(`channel-${channel}`)
        .locator("[data-testid^='channel-unread-']"),
    ).toBeVisible();
  }
  for (const channel of readChannels) {
    await expect(
      page
        .getByTestId(`channel-${channel}`)
        .locator("[data-testid^='channel-unread-']"),
    ).toHaveCount(0);
  }

  const mayaDm = page
    .getByTestId("channel-DM")
    .filter({ hasText: "Maya Chen" });
  const jordanDm = page
    .getByTestId("channel-DM")
    .filter({ hasText: "Jordan Brooks" });
  const priyaDm = page
    .getByTestId("channel-DM")
    .filter({ hasText: "Priya Shah" });
  await expect(
    mayaDm.locator("xpath=..").getByTestId("channel-unread-DM"),
  ).toBeVisible();
  await expect(
    jordanDm.locator("xpath=..").getByTestId("channel-unread-DM"),
  ).toHaveCount(0);
  await expect(
    priyaDm.locator("xpath=..").getByTestId("channel-unread-DM"),
  ).toHaveCount(0);

  await page.getByTestId("channel-flight-path").click();
  const channelTimeline = page.getByTestId("message-timeline");
  await expect(channelTimeline).toContainText("Marcus Reed");
  await expect(channelTimeline).toContainText("Elena Torres");
  await expect(channelTimeline).toContainText("Perfect. That’s the move.");
  const demoBuildRow = page
    .getByTestId("message-row")
    .filter({ hasText: "Demo build is running" })
    .last();
  await expect(
    demoBuildRow.locator('[data-link-preview="github-pull-request"]'),
  ).toBeVisible();
  await expect(demoBuildRow.getByTestId("message-reactions")).toContainText(
    "✅",
  );
  await expect(
    channelTimeline.locator('[data-link-preview="linear-issue"]').last(),
  ).toBeVisible();
  await expect(channelTimeline).toContainText(
    "the desktop-to-mobile handoff still feels a little fast",
    { timeout: 12_000 },
  );
  await expect(channelTimeline).toContainText(
    "one extra beat on the sent message",
  );
  await expect(channelTimeline).toContainText(
    "give the camera somewhere to land",
  );
  await expect(channelTimeline).toContainText(
    "Fizz can you turn that into a clean three-beat capture plan?",
  );
  const liveAgentRequestRow = page
    .getByTestId("message-row")
    .filter({ hasText: "three-beat capture plan" })
    .last();
  await expect(
    liveAgentRequestRow.getByTestId("message-reactions"),
  ).toContainText("👀");
  await expect(
    liveAgentRequestRow.getByTestId("message-reactions"),
  ).toContainText("💬");
  await expect(channelTimeline).toContainText(fizzReplyVisible, {
    timeout: 10_000,
  });
  const fizzReplyRow = page
    .getByTestId("message-row")
    .filter({ hasText: fizzReplyVisible })
    .last();
  await expect(
    fizzReplyRow.locator('img[src$="/demo/agents/fizz.png"]'),
  ).toBeVisible();
  await expect(fizzReplyRow.getByTestId("message-reactions")).toContainText(
    "👀",
  );
  await expect(fizzReplyRow.getByTestId("message-reactions")).toContainText(
    "💬",
  );
  await expect(channelTimeline).toContainText(honeyReplyVisible, {
    timeout: 10_000,
  });
  const honeyReplyRow = page
    .getByTestId("message-row")
    .filter({ hasText: honeyReplyVisible })
    .last();
  await expect(
    honeyReplyRow.locator('img[src$="/demo/agents/honey.png"]'),
  ).toBeVisible();
  await expect(honeyReplyRow.getByTestId("message-reactions")).toContainText(
    "👀",
  );
  await expect(honeyReplyRow.getByTestId("message-reactions")).toContainText(
    "💬",
  );
  await expect(channelTimeline).toContainText(bumbleReply, {
    timeout: 10_000,
  });
  const bumbleReplyRow = page
    .getByTestId("message-row")
    .filter({ hasText: bumbleReply })
    .last();
  await expect(
    bumbleReplyRow.locator('img[src$="/demo/agents/bumble.png"]'),
  ).toBeVisible();
  await expect(channelTimeline).toContainText(humanClose, { timeout: 5_000 });
  const humanCloseRow = page
    .getByTestId("message-row")
    .filter({ hasText: humanClose })
    .last();
  await expect(humanCloseRow).toBeInViewport({ ratio: 0.1 });
  await expect(
    liveAgentRequestRow.getByTestId("message-reactions"),
  ).toHaveCount(0, { timeout: 5_000 });

  const channelMessage = `The recording pass is ready ${Date.now()}`;
  await page.getByTestId("message-input").fill(channelMessage);
  await page.getByTestId("send-message").click();
  await expect(channelTimeline).toContainText(channelMessage);

  const messageInput = page.getByTestId("message-input");
  await messageInput.fill("Could ");
  await messageInput.pressSequentially("@Fiz");
  const agentMention = page
    .getByTestId("message-composer")
    .getByTestId("mention-autocomplete")
    .locator("button", { hasText: "Fizz" });
  await expect(agentMention).toBeVisible();
  await agentMention.click();
  await messageInput.pressSequentially(" suggest the strongest story beat?");
  await page.getByTestId("send-message").click();
  await expect(channelTimeline).toContainText(agentReply, { timeout: 10_000 });

  await page.getByTestId("channel-engineering").click();
  await expect(channelTimeline).toContainText(
    "Release build is green. I’m doing one last sleep / wake pass.",
  );
  await expect(
    channelTimeline.locator('[data-link-preview="github-pull-request"]'),
  ).toBeVisible();
  await expect(
    channelTimeline.locator('[data-link-preview="linear-issue"]'),
  ).toBeVisible();
  await expect(channelTimeline).toContainText(
    "I can still reproduce one duplicate message after waking the laptop.",
    { timeout: 5_000 },
  );
  await expect(channelTimeline).toContainText("Only on the first reconnect?");
  await expect(channelTimeline).toContainText(
    "One duplicate, then the subscription settles.",
  );
  await expect(channelTimeline).toContainText(
    "Bumble can you trace the likely path and pull Fizz and Honey into a fix plan?",
    { timeout: 10_000 },
  );
  await expect(channelTimeline).toContainText(
    engineeringBumbleReply.replace("@Fizz", "Fizz"),
    { timeout: 5_000 },
  );
  await expect(channelTimeline).toContainText(
    engineeringFizzReply.replace("@Honey", "Honey"),
    { timeout: 5_000 },
  );
  await expect(channelTimeline).toContainText(engineeringHoneyReply, {
    timeout: 5_000,
  });
  await expect(channelTimeline).toContainText(engineeringHumanClose, {
    timeout: 5_000,
  });
  const engineeringAgentRows = page
    .getByTestId("message-row")
    .filter({ has: page.locator('img[src^="/demo/agents/"]') });
  await expect(engineeringAgentRows).toHaveCount(3);

  const populatedChannels = [
    ["announcements", "Final smoke pass is clean"],
    ["general", "Please nobody breathe on main"],
    ["design", "Looks great on camera"],
    ["mobile", "The draft follows you now"],
    ["marketing", "No copy-paste script"],
    ["queen-bee-launch", "Sound mix is approved"],
  ] as const;
  for (const [channel, excerpt] of populatedChannels) {
    await page.getByTestId(`channel-${channel}`).click();
    await expect(channelTimeline).toContainText(excerpt);
  }

  await page.getByTestId("channel-design").click();
  await expect(channelTimeline.getByAltText("image").last()).toBeVisible();
  await expect(
    channelTimeline.locator('[data-link-preview="google-docs-document"]'),
  ).toBeVisible();

  await page.getByTestId("channel-marketing").click();
  await expect(
    channelTimeline
      .getByTestId("file-card")
      .filter({ hasText: "launch-social-crops.zip" }),
  ).toBeVisible();
  await expect(
    channelTimeline.locator('[data-link-preview="google-sheets-spreadsheet"]'),
  ).toBeVisible();

  await page.getByTestId("channel-DM").filter({ hasText: "Maya Chen" }).click();
  const dmMessage = `Can you join the capture review? ${Date.now()}`;
  await page.getByTestId("message-input").fill(dmMessage);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(dmMessage);

  await page.goto("/?demo=announcement#/projects");
  await page.locator('button[aria-label="Repositories"]').click();
  for (const project of ["flight-path", "nectar", "comb-kit", "swarm-launch"]) {
    await expect(
      page.locator(
        `[data-testid="project-card-${project}"], [data-testid="project-row-${project}"]`,
      ),
    ).toBeVisible();
  }
});

test("announcement story shows background work and pivots through a mention", async ({
  context,
  page,
}) => {
  test.setTimeout(30_000);
  await context.grantPermissions(["notifications"]);
  // The E2E bridge exposes Tauri internals, but Chromium reports Linux. Force
  // the browser notification branch so its click target is inspectable here;
  // the packaged Linux build uses the equivalent native backend action.
  await page.addInitScript(() => {
    const notificationLog: Array<{
      body: string | null;
      title: string;
    }> = [];
    const notifications: Array<{
      body: string | null;
      instance: Notification;
      title: string;
    }> = [];
    class DemoNotification extends EventTarget {
      static permission: NotificationPermission = "granted";
      static async requestPermission() {
        return DemoNotification.permission;
      }
      body: string | null;
      onclick: ((event: Event) => void) | null = null;
      title: string;
      constructor(title: string, options?: NotificationOptions) {
        super();
        this.title = title;
        this.body = options?.body ?? null;
        notifications.push({ body: this.body, instance: this, title });
        notificationLog.push({ body: this.body, title });
      }
      close() {}
    }
    Object.defineProperty(window, "Notification", {
      configurable: true,
      value: DemoNotification,
    });
    Object.assign(window, {
      __BUZZ_E2E_CLICK_NOTIFICATION__: (index: number) => {
        const notification = notifications[index]?.instance;
        if (!notification) return false;
        const event = new Event("click");
        notification.dispatchEvent(event);
        notification.onclick?.(event);
        return true;
      },
      __BUZZ_E2E_NOTIFICATIONS__: notificationLog,
    });
    Object.defineProperty(navigator, "platform", {
      configurable: true,
      value: "MacIntel",
    });
  });
  await page.goto("/?demo=announcement");
  expect(
    await page.evaluate(() => ({
      permission: Notification.permission,
      platform: navigator.platform,
    })),
  ).toEqual({ permission: "granted", platform: "MacIntel" });
  await expect(page.getByText("The Hive", { exact: true })).toBeVisible();

  // Startup may restore or default to flight-path and subscribe immediately;
  // that must not start the film before the recorder's explicit sidebar click.
  for (const channel of ["design", "engineering", "marketing"] as const) {
    await expect(page.getByTestId(`channel-working-${channel}`)).toHaveCount(0);
  }
  await page.waitForTimeout(1_000);
  for (const channel of ["design", "engineering", "marketing"] as const) {
    await expect(page.getByTestId(`channel-working-${channel}`)).toHaveCount(0);
  }

  await page.getByTestId("channel-flight-path").click();
  await expect(page.getByTestId("chat-title")).toHaveText("flight-path");

  for (const channel of ["design", "engineering", "marketing"] as const) {
    const badge = page.getByTestId(`channel-working-${channel}`);
    await expect(badge).toBeVisible();
    await expect(badge).toContainText(/^\d+s$/);
  }

  await expect
    .poll(
      () =>
        page.evaluate(() => {
          const win = window as Window & {
            __BUZZ_E2E_NOTIFICATIONS__?: Array<{
              body: string | null;
              title: string;
            }>;
          };
          return win.__BUZZ_E2E_NOTIFICATIONS__ ?? [];
        }),
      { timeout: 12_000 },
    )
    .toEqual([
      {
        body: expect.stringContaining("before this prototype gets any bigger"),
        title: expect.stringContaining("mentioned you in #engineering"),
      },
    ]);

  const clickedNotification = await page.evaluate(() => {
    const win = window as Window & {
      __BUZZ_E2E_CLICK_NOTIFICATION__?: (index: number) => boolean;
    };
    return win.__BUZZ_E2E_CLICK_NOTIFICATION__?.(0) ?? false;
  });
  expect(clickedNotification).toBe(true);

  await expect(page.getByTestId("chat-title")).toHaveText("engineering");
  await expect(page.getByTestId("message-timeline")).toContainText(
    "it’s already drifting",
  );
  await expect(page.getByTestId("channel-working-engineering")).toHaveCount(0);
  await expect(page.getByTestId("message-timeline")).toContainText(
    "Move it to Flutter now",
    { timeout: 5_000 },
  );
  await expect(page.getByTestId("message-timeline")).toContainText(
    "we’re moving the prototype to Flutter",
    { timeout: 7_000 },
  );
  await expect(page.getByTestId("message-timeline")).toContainText(
    "mapping the current React views to Flutter widgets",
    { timeout: 6_000 },
  );
  await expect(page.getByTestId("message-timeline")).toContainText(
    "porting the game loop to Flutter",
    { timeout: 6_000 },
  );
  await expect(page.getByTestId("message-timeline")).toContainText(
    "Building the Flutter widget tree",
    { timeout: 6_000 },
  );
  await expect(page.getByTestId("message-timeline")).not.toContainText(
    "connection hiccup",
  );
  await expect(
    page
      .getByTestId("message-row")
      .filter({ hasText: "One stack, both platforms" }),
  ).toHaveCount(1);
});
