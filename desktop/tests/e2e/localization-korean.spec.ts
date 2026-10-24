import { expect, test, type Page } from "@playwright/test";
import { LANGUAGE_STORAGE_KEY } from "../../src/language";
import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

async function installLocalizedBridge(page: Page, language = "en") {
  await page.addInitScript(
    ({ key, language }) => {
      if (localStorage.getItem(key) === null)
        localStorage.setItem(key, language);
    },
    { key: LANGUAGE_STORAGE_KEY, language },
  );
  await installMockBridge(page);
}

test("language changes preserve a literal message draft and experimental feature IDs", async ({
  page,
}) => {
  await installLocalizedBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const input = page.getByTestId("message-input");
  const draft = "한글 입력 — Settings {{p0}} 파일.txt <b>원문</b>";
  await input.fill(draft);
  await openSettings(page, "appearance");
  await page.getByTestId("app-language").scrollIntoViewIfNeeded();
  await waitForAnimations(page);
  await page.screenshot({
    path: "test-results/korean-localization/language-en.png",
  });
  await page.getByTestId("app-language").selectOption("ko");
  await waitForAnimations(page);
  await page.screenshot({
    path: "test-results/korean-localization/language-ko.png",
  });
  await expect(page.locator("html")).toHaveAttribute("lang", "ko");
  await page.getByTestId("settings-nav-experimental").click();
  const workflow = page.getByRole("switch", { name: "워크플로", exact: true });
  await expect(workflow).toHaveAttribute("aria-checked", "true");
  await expect(
    page.getByText("승인 단계를 포함한 YAML 기반 자동화", { exact: true }),
  ).toBeVisible();
  await page.getByTestId("settings-back-to-app").click();
  await expect(input).toHaveText(draft);
  await page.reload();
  await page.getByTestId("channel-general").click();
  await expect(input).toHaveText(draft);
  await expect(page.locator("html")).toHaveAttribute("lang", "ko");
  await openSettings(page, "appearance");
  await page.getByTestId("app-language").selectOption("en");
  await page.getByTestId("settings-back-to-app").click();
  await expect(input).toHaveText(draft);
});

test("workflow dialog titles and project descriptions use Korean authored copy", async ({
  page,
}) => {
  await page.addInitScript(() => {
    window.__BUZZ_E2E_EMPTY_PROJECTS__ = true;
  });
  await installLocalizedBridge(page, "ko");
  await page.goto("/");
  await page.getByTestId("open-workflows-view").click();
  await page
    .getByRole("button", { name: "워크플로 만들기", exact: true })
    .click();
  const workflow = page.getByRole("dialog", {
    name: "워크플로 만들기",
    exact: true,
  });
  await expect(workflow).toBeVisible();
  await expect(
    workflow.getByRole("heading", { name: "워크플로 만들기", exact: true }),
  ).toBeVisible();
  await workflow.getByRole("button", { name: "닫기", exact: true }).click();
  await page.getByTestId("open-projects-view").click();
  await page
    .getByRole("button", { name: "프로젝트 만들기", exact: true })
    .click();
  const project = page.getByTestId("create-project-dialog");
  await expect(project).toContainText(
    "프로젝트는 저장소가 연결된 채널로 시작합니다.",
  );
  await expect(page.getByTestId("create-project-team")).toHaveAccessibleName(
    "팀: 없음",
  );
});

for (const [width, height] of [
  [900, 650],
  [1280, 800],
]) {
  for (const colorScheme of ["light", "dark"] as const) {
    test(`Korean appearance controls fit ${width}x${height} with ${colorScheme} colors`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height });
      await page.emulateMedia({ colorScheme });
      await installLocalizedBridge(page, "ko");
      await page.goto("/");
      await openSettings(page, "appearance");
      const language = page.getByTestId("app-language");
      await expect(language).toHaveAccessibleName("언어");
      await language.scrollIntoViewIfNeeded();
      const box = await language.boundingBox();
      expect(box).not.toBeNull();
      if (!box) throw new Error("Language control has no layout box");
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width).toBeLessThanOrEqual(width);
      await waitForAnimations(page);
      await page.screenshot({
        path: `test-results/korean-localization/appearance-${width}-${colorScheme}.png`,
      });
    });
  }
}
