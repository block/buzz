import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import React from "react";
import { act, cleanup, fireEvent, render } from "@testing-library/react";
import {
  i18n,
  readStoredLanguagePreference,
  setAppLanguage,
} from "../../i18n.ts";
import { LanguageSetting } from "../../features/settings/ui/LanguageSetting.tsx";
import { uiMessage, uiT, useUiLanguage } from "./uiTranslation.ts";

afterEach(async () => {
  cleanup();
  await setAppLanguage("en");
  localStorage.clear();
});

test("the real selector changes mounted labels without remounting or editing a draft", async () => {
  await setAppLanguage("en");
  const descriptor = {
    get label() {
      return uiT("Settings");
    },
  };
  let mounts = 0;
  function Screen() {
    useUiLanguage();
    React.useEffect(() => {
      mounts += 1;
    }, []);
    const [draft, setDraft] = React.useState("");
    return React.createElement(
      React.Fragment,
      null,
      React.createElement(LanguageSetting),
      React.createElement("h1", null, descriptor.label),
      React.createElement("textarea", {
        "aria-label": "draft",
        value: draft,
        onChange: (event) => setDraft(event.target.value),
      }),
      React.createElement("pre", null, draft),
    );
  }
  const screen = render(React.createElement(Screen));
  const draft = "Settings 파일.txt / 한글 입력 {{p0}} <b>원문</b> $t(sample)";
  const input = screen.getByRole("textbox", { name: "draft" });
  fireEvent.change(input, { target: { value: draft } });
  await act(async () =>
    fireEvent.change(screen.getByTestId("app-language"), {
      target: { value: "ko" },
    }),
  );
  assert.equal(screen.getByRole("heading").textContent, "설정");
  assert.equal(document.documentElement.lang, "ko");
  assert.equal(readStoredLanguagePreference(), "ko");
  assert.equal(input.value, draft);
  assert.equal(screen.container.querySelector("pre").textContent, draft);
  assert.equal(mounts, 1);
  await act(async () =>
    fireEvent.change(screen.getByTestId("app-language"), {
      target: { value: "en" },
    }),
  );
  assert.equal(screen.getByRole("heading").textContent, "Settings");
  assert.equal(input.value, draft);
  assert.equal(mounts, 1);
});

test("rich translations preserve an inline link and its callback", async () => {
  await setAppLanguage("ko");
  let clicks = 0;
  const link = React.createElement(
    "button",
    {
      type: "button",
      onClick: () => {
        clicks += 1;
      },
    },
    "원문 이름 <script>",
  );
  const screen = render(
    React.createElement(
      "p",
      null,
      uiMessage("This is the beginning of the {0}.", { p0: link }),
    ),
  );
  assert.equal(
    screen.container.textContent,
    "원문 이름 <script>의 시작입니다.",
  );
  fireEvent.click(screen.getByRole("button"));
  assert.equal(clicks, 1);
  assert.equal(screen.container.querySelector("script"), null);
});

test("interpolation preserves user values, counts, fallback and literal unknown text", async () => {
  await setAppLanguage("ko");
  const userValue = "<b>{{p1}} $t(sample) Settings 한글.txt</b>";
  assert.ok(uiT("Profile sync: {0}", { p0: userValue }).includes(userValue));
  for (const count of [0, 1, 2, 100]) {
    assert.equal(
      uiT("Remove {0} member{1}?", { p0: count, p1: count === 1 ? "" : "s" }),
      `멤버 ${count}명을 제거할까요?`,
    );
  }
  assert.equal(
    uiT("Unregistered original user text"),
    "Unregistered original user text",
  );
  await i18n.changeLanguage("zh-Hans");
  assert.equal(
    uiT("Profile sync: {0}", { p0: userValue }),
    `Profile sync: ${userValue}`,
  );
});

test("failed storage applies the language and reports that it was not saved", async () => {
  await setAppLanguage("en");
  const prototype = Object.getPrototypeOf(localStorage);
  const original = prototype.setItem;
  prototype.setItem = () => {
    throw new Error("storage denied");
  };
  try {
    const screen = render(React.createElement(LanguageSetting));
    await act(async () =>
      fireEvent.change(screen.getByTestId("app-language"), {
        target: { value: "ko" },
      }),
    );
    assert.ok(screen.getByRole("status").textContent.includes("저장"));
    assert.equal(uiT("Settings"), "설정");
    assert.equal(document.documentElement.lang, "ko");
    assert.equal(readStoredLanguagePreference(), "en");
    screen.unmount();
    const reopened = render(React.createElement(LanguageSetting));
    assert.equal(reopened.getByTestId("app-language").value, "ko");
    assert.ok(reopened.getByRole("status").textContent.includes("저장"));
  } finally {
    prototype.setItem = original;
  }
});

test("date and notification helpers follow the selected language without changing user content", async () => {
  const { formatDayGroupLabel, formatItemTimestamp } = await import(
    "../lib/datetime.ts"
  );
  const { formatMessageNotification } = await import(
    "../../features/notifications/lib/notificationFormat.ts"
  );
  const now = new Date(2026, 9, 23, 14, 30).getTime() / 1000;
  const content = "Settings 한글.txt {{p0}} $t(sample)";
  await setAppLanguage("ko");
  assert.equal(formatDayGroupLabel(now, now), "오늘");
  assert.match(
    formatItemTimestamp(now - 86400, { nowSeconds: now, withTime: true }),
    /어제.*(?:오후|PM)/,
  );
  assert.equal(
    formatMessageNotification({ source: "dm", senderName: "Settings", content })
      .body,
    content,
  );
  assert.equal(
    formatMessageNotification({ source: "dm", content: "" }).title,
    "다이렉트 메시지",
  );
  assert.equal(
    formatMessageNotification({ source: "dm", content: "" }).body,
    "새 메시지",
  );
  await setAppLanguage("en");
  assert.equal(formatDayGroupLabel(now, now), "Today");
  assert.match(
    formatItemTimestamp(now - 86400, { nowSeconds: now, withTime: true }),
    /Yesterday at .*PM/,
  );
});

test("Korean workflow conditions preserve quoted content and separate generated author attribution", async () => {
  const { workflowTriggerDescription, splitWorkflowAuthorDescription } =
    await import("../../features/workflows/ui/workflowTriggerDescription.ts");
  const { agentCommunityAvailability, managedAgentPairAction } = await import(
    "../../features/agents/managedAgentRuntimeStatus.ts"
  );
  await setAppLanguage("ko");
  const description = workflowTriggerDescription(
    {
      on: "message_posted",
      filter: `str_contains(trigger_text, "Carl") && trigger_author == "${"a".repeat(64)}"`,
    },
    { authorLabel: "Carl" },
  );
  assert.ok(description.includes("“Carl”"));
  assert.ok(!description.includes(" by "));
  assert.deepEqual(splitWorkflowAuthorDescription(description, "Carl"), {
    prefix: "",
    suffix: "이(가) 메시지에 “Carl” 포함",
  });
  const runtime = { localSetup: true, lifecycle: "ready" };
  assert.equal(
    agentCommunityAvailability(runtime),
    "Here",
    "state discriminators stay stable in Korean",
  );
  assert.equal(managedAgentPairAction(runtime), "stop");
});

test("multiple slots never reinterpret interpolation tokens inside inserted values", async () => {
  const values = { p0: "Alice {{p1}}", p1: "general {{p0}} $t(Settings)" };
  for (const language of ["en", "ko", "zh-Hans"]) {
    await i18n.changeLanguage(language);
    const result = uiT("Reply to {0} in #{1}", values);
    assert.ok(result.includes(values.p0), result);
    assert.ok(result.includes(values.p1), result);
    if (language !== "ko") {
      assert.equal(result, `Reply to ${values.p0} in #${values.p1}`);
    }
  }
});
