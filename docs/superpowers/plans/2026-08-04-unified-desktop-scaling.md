# Unified Desktop Scaling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make interface, chat text, and every desktop identity avatar adjustable from 75% through 500% without detached presence indicators, overlapping avatar-dependent layout, or unreachable Appearance controls.

**Architecture:** A shared preset ladder feeds the three persistent scale stores. `UserAvatar` scales identity avatars by default from semantic base metrics, while specialized profile avatars use a pure proportional status-geometry helper; message gutters, rails, stacks, and profile surfaces consume the same resolved values as the rendered avatar. Layout reserves the scaled size instead of using `transform: scale()`.

**Tech Stack:** React 19, TypeScript, Tailwind CSS, Node test runner, Playwright, Vite E2E mock bridge.

## Global Constraints

- All three Appearance controls use exactly `75, 90, 100, 110, 125, 150, 175, 200, 250, 300, 400, 500` percent.
- Interface scale is the base multiplier; chat and avatar scales remain relative multipliers.
- Existing local-storage keys remain `buzz:text-scale`, `buzz:chat-scale`, and `buzz:avatar-scale`.
- Avatar source files, upload/crop/capture resolution, decorative art, emoji previews, non-identity icons, relay, protocol, media, mobile, and dependencies do not change.
- Preserve all pre-existing uncommitted work. Do not commit during execution because the worktree already contains user-owned changes and the user did not request commits.
- Use `pnpm build:e2e` for rendered tests; never use a plain production build as the E2E fixture.

---

### Task 1: Shared 500% Preference Contract

**Files:**
- Create: `desktop/src/shared/lib/appearanceScalePresets.ts`
- Create: `desktop/src/shared/lib/chatScale.test.mjs`
- Modify: `desktop/src/shared/lib/scalePreference.test.mjs`
- Modify: `desktop/src/shared/lib/textScale.test.mjs`
- Modify: `desktop/src/shared/lib/avatarScale.test.mjs`
- Modify: `desktop/src/shared/lib/textScale.ts`
- Modify: `desktop/src/shared/lib/chatScale.ts`
- Modify: `desktop/src/shared/lib/avatarScale.ts`

**Interfaces:**
- Produces: `APPEARANCE_SCALE_PRESETS: readonly [0.75, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3, 4, 5]`.
- Produces: `adjustTextScale(action)` that advances by preset index and reaches both 75% and 500%.
- Preserves: all current exported store names and storage keys.

- [ ] **Step 1: Write failing preference tests**

```js
test("all Appearance scales share the 75%-500% preset ladder", () => {
  assert.deepEqual([...TEXT_SCALE_PRESETS], [...APPEARANCE_SCALE_PRESETS]);
  assert.deepEqual([...CHAT_SCALE_PRESETS], [...APPEARANCE_SCALE_PRESETS]);
  assert.deepEqual([...AVATAR_SCALE_PRESETS], [...APPEARANCE_SCALE_PRESETS]);
  assert.equal(MAX_TEXT_SCALE, 5);
  assert.equal(MAX_CHAT_SCALE, 5);
  assert.equal(MAX_AVATAR_SCALE, 5);
});

test("keyboard increase advances beyond 150% and stops at 500%", async () => {
  setTextScale(1.5);
  assert.equal(adjustTextScale("increase"), 1.75);
  setTextScale(4);
  assert.equal(adjustTextScale("increase"), 5);
  assert.equal(adjustTextScale("increase"), 5);
});
```

- [ ] **Step 2: Run the focused tests and verify the expected failures**

Run: `cd desktop; pnpm test -- src/shared/lib/textScale.test.mjs src/shared/lib/chatScale.test.mjs src/shared/lib/avatarScale.test.mjs`

Expected: failures report the old 150%/200% maxima and inability to advance past 150%.

- [ ] **Step 3: Add the shared ladder and wire all stores to it**

```ts
export const APPEARANCE_SCALE_PRESETS = [
  0.75, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3, 4, 5,
] as const;
```

Import the constant into each store. Replace arithmetic keyboard stepping with adjacent preset lookup:

```ts
const currentIndex = textScalePresetIndex(textScale);
const direction = action === "increase" ? 1 : -1;
const nextIndex = Math.min(
  Math.max(currentIndex + direction, 0),
  TEXT_SCALE_PRESETS.length - 1,
);
setTextScale(TEXT_SCALE_PRESETS[nextIndex] ?? DEFAULT_TEXT_SCALE);
```

- [ ] **Step 4: Run the focused tests and verify they pass**

Run: `cd desktop; pnpm test -- src/shared/lib/textScale.test.mjs src/shared/lib/chatScale.test.mjs src/shared/lib/avatarScale.test.mjs src/shared/lib/scalePreference.test.mjs`

Expected: all selected tests pass with maximum `5` and formatted label `500%`.

---

### Task 2: Semantic Avatar Metrics and Default App-Wide Scaling

**Files:**
- Modify: `desktop/src/shared/lib/avatarScale.test.mjs`
- Modify: `desktop/src/shared/lib/avatarScale.ts`
- Modify: `desktop/src/shared/ui/UserAvatar.tsx`
- Modify: `desktop/src/features/messages/ui/DirectMessageIntroAvatarStack.tsx`
- Modify: `desktop/src/features/messages/ui/MessageRow.tsx`
- Modify: `desktop/src/features/messages/ui/MessageThreadSummaryRow.tsx`
- Modify: `desktop/src/features/messages/ui/SystemMessageRow.tsx`
- Modify: `desktop/src/features/pulse/ui/NoteCard.tsx`

**Interfaces:**
- Produces: `type AvatarSize = "xs" | "sm" | "md"`.
- Produces: `getAvatarSizeRem(size: AvatarSize, scale?: number): number`.
- Produces: `avatarSizeStyle(size: AvatarSize, scale?: number): AvatarSizeStyle`.
- Produces: `UserAvatar` prop `appearanceScale?: boolean`, defaulting to `true`; `false` is reserved for excluded editor/decorative surfaces.
- Removes: temporary message-only `messageScale` prop and `getScaledMessageAvatarRem` duplication.

- [ ] **Step 1: Extend metric tests with all semantic sizes and boundary scales**

```js
for (const [scale, expected] of [
  [0.75, { xs: 0.9375, sm: 1.125, md: 2.25 }],
  [1, { xs: 1.25, sm: 1.5, md: 3 }],
  [2, { xs: 2.5, sm: 3, md: 6 }],
  [5, { xs: 6.25, sm: 7.5, md: 15 }],
]) {
  test(`semantic avatar metrics resolve at ${scale * 100}%`, () => {
    assert.equal(getAvatarSizeRem("xs", scale), expected.xs);
    assert.equal(getAvatarSizeRem("sm", scale), expected.sm);
    assert.equal(getAvatarSizeRem("md", scale), expected.md);
  });
}
```

- [ ] **Step 2: Run the metric test and verify it fails because the semantic API is absent**

Run: `cd desktop; pnpm test -- src/shared/lib/avatarScale.test.mjs`

Expected: import/export failure for `getAvatarSizeRem`.

- [ ] **Step 3: Implement semantic metrics and make `UserAvatar` scale by default**

```ts
export const AVATAR_BASE_SIZE_REM = { xs: 1.25, sm: 1.5, md: 3 } as const;
export type AvatarSize = keyof typeof AVATAR_BASE_SIZE_REM;

export function getAvatarSizeRem(size: AvatarSize, scale = getAvatarScale()) {
  return AVATAR_BASE_SIZE_REM[size] * normalizeAvatarScale(scale);
}
```

`UserAvatar` subscribes once through `useAvatarScale`, resolves an inline box from `avatarSizeStyle`, and applies it unless `appearanceScale={false}`. Replace message call-site `messageScale` props with the default behavior. Remove the three `!h-* !w-*` overrides in `NoteCard`, because `!important` would defeat the shared inline metric.

- [ ] **Step 4: Run the focused metric and affected message tests**

Run: `cd desktop; pnpm test -- src/shared/lib/avatarScale.test.mjs src/features/messages/lib/threadTreeLayout.test.mjs`

Expected: semantic metric tests and existing avatar-dependent message layout tests pass.

---

### Task 3: Proportional Profile Status Geometry

**Files:**
- Create: `desktop/src/features/profile/lib/avatarStatusGeometry.ts`
- Create: `desktop/src/features/profile/lib/avatarStatusGeometry.test.mjs`
- Modify: `desktop/src/features/profile/ui/ProfileAvatarWithStatus.tsx`
- Modify: `desktop/src/features/profile/ui/UserProfilePopover.tsx`
- Modify: `desktop/src/features/profile/ui/UserProfilePanelSections.tsx`

**Interfaces:**
- Produces: `type AvatarStatusGeometryRatios = { centerX: number; centerY: number; cutoutDiameter: number; dotDiameter: number }`.
- Produces: `resolveAvatarStatusGeometry(size: number, ratios: AvatarStatusGeometryRatios): { cutout: AvatarBadgeCircle; badgeBox: AvatarBadgeBox }`.
- Invariant: `badgeBox.width === badgeBox.height === dotDiameter * size`, and the visible badge uses `h-full w-full`.

- [ ] **Step 1: Write a failing pure geometry regression test**

```js
test("500% profile status geometry stays proportional and inside the avatar", () => {
  const result = resolveAvatarStatusGeometry(480, {
    centerX: 0.85,
    centerY: 0.85,
    cutoutDiameter: 0.375,
    dotDiameter: 0.3,
  });
  assert.deepEqual(result.badgeBox, {
    bottom: 0,
    height: 144,
    right: 0,
    width: 144,
  });
  assert.deepEqual(result.cutout, { cx: 408, cy: 408, r: 90 });
});
```

- [ ] **Step 2: Run the geometry test and verify the helper is missing**

Run: `cd desktop; pnpm test -- src/features/profile/lib/avatarStatusGeometry.test.mjs`

Expected: import/export failure for `resolveAvatarStatusGeometry`.

- [ ] **Step 3: Implement the helper and replace fixed badge classes**

```ts
export function resolveAvatarStatusGeometry(
  size: number,
  ratios: AvatarStatusGeometryRatios,
) {
  const dotSize = size * ratios.dotDiameter;
  const centerX = size * ratios.centerX;
  const centerY = size * ratios.centerY;
  return {
    cutout: {
      cx: centerX,
      cy: centerY,
      r: (size * ratios.cutoutDiameter) / 2,
    },
    badgeBox: {
      bottom: size - centerY - dotSize / 2,
      height: dotSize,
      right: size - centerX - dotSize / 2,
      width: dotSize,
    },
  };
}
```

Both hover and panel heroes pass ratio presets to this helper. Their badge wrappers and `PresenceDot` use `h-full w-full`; no fixed `h-5`, `h-7`, or equivalent remains inside a scaled badge slot.

- [ ] **Step 4: Run geometry and profile-related unit tests**

Run: `cd desktop; pnpm test -- src/features/profile/lib/avatarStatusGeometry.test.mjs src/shared/lib/avatarScale.test.mjs`

Expected: all selected tests pass, including the 500% geometry case.

---

### Task 4: Synchronize Avatar-Dependent Message Layout

**Files:**
- Modify: `desktop/src/features/messages/lib/threadTreeLayout.test.mjs`
- Modify: `desktop/src/features/messages/lib/threadTreeLayout.ts`
- Modify: `desktop/src/features/messages/ui/MessageRow.tsx`
- Modify: `desktop/src/features/messages/ui/MessageThreadSummaryRow.tsx`
- Modify: `desktop/src/features/messages/ui/SystemMessageRow.tsx`
- Modify: `desktop/src/features/messages/ui/DirectMessageIntroAvatarStack.tsx`

**Interfaces:**
- Consumes: `getAvatarSizeRem("md", scale)` and `useAvatarScale()` from Task 2.
- Produces: thread rail, depth indent, row gutter, and avatar-stack positions derived from the same resolved avatar size.
- Invariant: every rail center equals the corresponding avatar center at 75%, 100%, 200%, and 500%.

- [ ] **Step 1: Add 500% layout cases that fail against the current partial implementation**

```js
test("thread rail centers remain aligned at 500% avatar scale", () => {
  const avatarSize = 15;
  assert.equal(getThreadReplyAvatarSizeRem(5), avatarSize);
  assert.equal(getThreadRailCenterRem(5), THREAD_REPLY_ROW_PADDING_TOP_REM + avatarSize / 2);
  assert.equal(getThreadDepthIndentRem(5), avatarSize);
});
```

- [ ] **Step 2: Run the thread layout test and verify the 500% case fails**

Run: `cd desktop; pnpm test -- src/features/messages/lib/threadTreeLayout.test.mjs`

Expected: failure shows a stale default-scale or 200%-bounded calculation.

- [ ] **Step 3: Route every dependent calculation through the explicit scale parameter**

Pure layout helpers accept `scale = getAvatarScale()` and use `getAvatarSizeRem("md", scale)`. React rows subscribe with `useAvatarScale()` and pass the current value to all geometry helpers during one render. Avatar-stack offsets use the same resolved rem value and reserve the complete box.

- [ ] **Step 4: Run message tests and the full desktop unit suite**

Run: `cd desktop; pnpm test -- src/features/messages/lib/threadTreeLayout.test.mjs`

Run: `cd desktop; pnpm test`

Expected: focused and full unit suites pass with no warnings or failures.

---

### Task 5: Adaptive Appearance Controls and Rendered Regression Coverage

**Files:**
- Modify: `desktop/src/features/settings/ui/AppearanceScaleSettings.tsx`
- Modify: `desktop/src/features/settings/ui/SettingsPanels.tsx`
- Create: `desktop/tests/e2e/appearance-scaling.spec.ts`
- Modify: `desktop/playwright.config.ts`

**Interfaces:**
- Consumes: shared preset ladder and all three scale stores from Task 1.
- Produces: wrapping Appearance rows with reachable sliders, min/max labels, current percentage, and Reset at 500% interface scale.
- Produces: E2E coverage for live updates, persistence, profile status geometry, and representative app-wide avatar scaling.

- [ ] **Step 1: Write the failing E2E behavior test**

```ts
test("Appearance scales reach 500% and remain usable", async ({ page }) => {
  await installMockBridge(page);
  await openAppearance(page);
  for (const prefix of ["interface-scale", "chat-scale", "avatar-scale"]) {
    const slider = page.getByTestId(`${prefix}-slider`);
    await slider.fill("11");
    await expect(page.getByTestId(`${prefix}-value`)).toHaveText("500%");
    await expect(slider).toHaveAttribute("aria-valuetext", "500%");
  }
  await expect(page.getByTestId("settings-content-scroll")).not.toHaveCSS(
    "overflow-x",
    "hidden",
  );
});
```

The real test also reloads after setting values, reopens Appearance, verifies all three `500%` labels, returns interface scale to 100% before navigating, and asserts representative avatar/status bounding-box ratios so the test remains operable.

- [ ] **Step 2: Build the E2E fixture and verify the new test fails for the intended reasons**

Run: `cd desktop; pnpm build:e2e; pnpm exec playwright test tests/e2e/appearance-scaling.spec.ts --project=smoke`

Expected: failures show the old maxima, detached badge geometry, or non-wrapping controls rather than bridge/bootstrap errors.

- [ ] **Step 3: Implement the adaptive settings row**

Use `flex-wrap`, `basis-*`, `min-w-0`, and a full-width control group at constrained content widths. Give the range input a 44px-high transparent interaction box while keeping the visible track slim. Copy states that chat/avatar percentages are relative to Interface scale.

- [ ] **Step 4: Register the E2E spec and verify rendered behavior**

Add `**/appearance-scaling.spec.ts` to the `smoke` project. Run:

`cd desktop; pnpm build:e2e; pnpm exec playwright test tests/e2e/appearance-scaling.spec.ts --project=smoke`

Expected: the test passes at the desktop viewport and its constrained viewport block; screenshots show no detached status-dot void, overlap, clipping, or unreachable control.

- [ ] **Step 5: Run static and broad verification**

Run: `cd desktop; pnpm typecheck`

Run: `cd desktop; pnpm check`

Run: `cd desktop; pnpm test`

Run: `cd desktop; pnpm build:e2e`

Expected: every command exits `0`. Review `git diff --check` and `git diff --stat`; confirm no relay/mobile/dependency changes and no generated screenshots or reports were added to the repository.

