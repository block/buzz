import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { loadKoreanCatalog } from "./korean-catalog-input.mjs";

const args = process.argv.slice(2);
const check = args.includes("--check");
const inputs = args.filter((arg) => arg !== "--check");
if (inputs.length > 1 || inputs[0]?.startsWith("--"))
  throw new Error(
    "Usage: import-korean-catalog.mjs [pinned-source.patch] [--check]",
  );
const { table, source: provenance } = loadKoreanCatalog(inputs[0]);
const outputs = new Map();
function output(name, data) {
  outputs.set(name + ".json", JSON.stringify(data, null, 2) + "\n");
}
const keys = {},
  en = {},
  ko = {};
for (const [english, korean] of [
  ...Object.entries(table.exact),
  ...table.patterns,
]) {
  const source = english.replace(/\s+/g, " ").trim();
  const key =
    "m" + crypto.createHash("sha256").update(source).digest("hex").slice(0, 16);
  if (en[key] && keys[source] !== key)
    throw new Error("Catalogue key collision");
  keys[source] = key;
  en[key] = source.replace(/\{(\d+)\}/g, (_, n) => `{{p${n}}}`);
  ko[key] = korean
    .replaceAll("인박스", "받은 편지함")
    .replace(/\{(\d+)\}/g, (_, n) => `{{p${n}}}`);
}
const root = path.resolve(import.meta.dirname, "../src/locales");
const additional = JSON.parse(
  fs.readFileSync(path.join(root, "ko.additional.json"), "utf8"),
);
for (const [english, korean] of Object.entries(additional)) {
  const source = english.replace(/\s+/g, " ").trim();
  const key =
    "m" + crypto.createHash("sha256").update(source).digest("hex").slice(0, 16);
  keys[source] = key;
  en[key] = source.replace(/\{(\d+)\}/g, (_, n) => `{{p${n}}}`);
  ko[key] = korean
    .replaceAll("인박스", "받은 편지함")
    .replace(/\{(\d+)\}/g, (_, n) => `{{p${n}}}`);
}
for (const [name, data] of [
  ["ui-keys", keys],
  ["en.ui", en],
  ["ko.ui", ko],
])
  output(name, data);
const base = JSON.parse(fs.readFileSync(path.join(root, "en.json"), "utf8"));
base.settings.language.retry = "Try again";
base.settings.language.unsaved =
  "Applied for this session. The language preference could not be saved.";
base.settings.language.error = "The language could not be changed. Try again.";
base.settings.language.nativeError =
  "The app language changed, but the native menus could not be updated. Choose the language again to retry.";
output("en", base);
for (const locale of ["zh-Hans", "zh-Hant"]) {
  const p = path.join(root, locale + ".json");
  const data = JSON.parse(fs.readFileSync(p, "utf8"));
  data.settings.language.retry = base.settings.language.retry;
  data.settings.language.unsaved = base.settings.language.unsaved;
  data.settings.language.error = base.settings.language.error;
  data.settings.language.nativeError = base.settings.language.nativeError;
  output(locale, data);
}
const manual = {
  Language: "언어",
  "Choose the language used by Buzz.": "Buzz에서 사용할 언어를 선택합니다.",
  System: "시스템 설정 따름",
  English: "English",
  "Traditional Chinese": "繁體中文",
  "Simplified Chinese": "简体中文",
  Korean: "한국어",
};
manual[base.settings.language.unsaved] =
  "현재 실행 중에는 적용되지만 언어 설정을 저장하지 못했습니다.";
manual[base.settings.language.error] =
  "언어를 변경하지 못했습니다. 다시 시도하세요.";
manual[base.settings.language.nativeError] =
  "앱 언어는 변경됐지만 macOS 메뉴를 갱신하지 못했습니다. 언어를 다시 선택해 재시도하세요.";
const missing = [];
function translate(x) {
  if (typeof x === "string") {
    if (manual[x]) return manual[x];
    if (additional[x]) return additional[x];
    if (table.exact[x]) return table.exact[x];
    missing.push(x);
    return x;
  }
  return Object.fromEntries(
    Object.entries(x).map(([k, v]) => [k, translate(v)]),
  );
}
output("ko", translate(base));
output("korean-source", {
  ...provenance,
  exact: Object.keys(table.exact).length,
  patterns: table.patterns.length,
  method: "Literal catalogue only; DOM observer code is not imported.",
});
const changed = [...outputs].filter(([name, content]) => {
  const file = path.join(root, name);
  return !fs.existsSync(file) || fs.readFileSync(file, "utf8") !== content;
});
if (check && changed.length)
  throw new Error(
    `Stale Korean catalogue outputs: ${changed.map(([name]) => name).join(", ")}`,
  );
if (!check)
  for (const [name, content] of changed)
    fs.writeFileSync(path.join(root, name), content);
console.log(
  JSON.stringify(
    {
      exact: Object.keys(table.exact).length,
      patterns: table.patterns.length,
      messages: Object.keys(keys).length,
      baseMissing: missing,
      check,
      changed: changed.length,
    },
    null,
    2,
  ),
);
