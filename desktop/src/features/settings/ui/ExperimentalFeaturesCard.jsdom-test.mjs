import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import React from "react";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { setAppLanguage } from "../../../i18n.ts";
import { desktopFeatures } from "../../../shared/features/manifest.ts";
import { getOverrides } from "../../../shared/features/store.ts";
import { ExperimentalFeaturesCard } from "./ExperimentalFeaturesCard.tsx";

afterEach(async () => {
  cleanup();
  await setAppLanguage("en");
  localStorage.clear();
});

test("manifest copy follows the language while feature IDs and overrides stay literal", async () => {
  await setAppLanguage("en");
  const before = JSON.stringify(desktopFeatures);
  const screen = render(React.createElement(ExperimentalFeaturesCard));
  const workflows = screen.getByRole("switch", { name: "Workflows" });
  fireEvent.click(workflows);
  assert.deepEqual(getOverrides(), { workflows: true });

  await act(() => setAppLanguage("ko"));
  assert.equal(screen.getByRole("switch", { name: "워크플로" }), workflows);
  assert.equal(workflows.getAttribute("aria-checked"), "true");
  screen.getByText("승인 단계를 포함한 YAML 기반 자동화");
  screen.getByRole("switch", { name: "포럼 채널" });
  screen.getByRole("switch", { name: "에이전트가 관리하는 프로필" });
  assert.deepEqual(getOverrides(), { workflows: true });
  assert.equal(JSON.stringify(desktopFeatures), before);

  await act(() => setAppLanguage("en"));
  assert.equal(screen.getByRole("switch", { name: "Workflows" }), workflows);
  screen.getByText("YAML-defined automations with approval gates");
});
