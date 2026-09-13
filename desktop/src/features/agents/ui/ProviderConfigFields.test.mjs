import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { coerceConfigValues } from "./ProviderConfigFields.tsx";
import { providerSchemaUsesExtendedPresentation } from "./ProviderConfigSchemaFields.tsx";
import {
  providerConfigFieldVisible,
  providerConfigOptions,
  reconcileProviderConfig,
} from "./providerConfigSchema.ts";

const schema = {
  properties: {
    inactivity_seconds: { type: "integer" },
    threshold: { type: "number" },
    label: { type: "string" },
  },
};

describe("coerceConfigValues", () => {
  it("omits cleared numeric fields without losing explicit zero", () => {
    assert.deepEqual(
      coerceConfigValues(
        { inactivity_seconds: "", threshold: "0", label: "" },
        schema,
      ),
      { threshold: 0, label: "" },
    );
  });

  it("preserves nonempty invalid numeric input for provider validation", () => {
    assert.deepEqual(
      coerceConfigValues({ inactivity_seconds: "not-a-number" }, schema),
      { inactivity_seconds: "not-a-number" },
    );
  });
});

describe("providerConfigOptions", () => {
  it("renders simple enum values without provider-specific logic", () => {
    assert.deepEqual(providerConfigOptions({ enum: ["codex", "claude"] }), [
      { label: "codex", value: "codex" },
      { label: "claude", value: "claude" },
    ]);
  });

  it("uses oneOf titles as bounded option labels", () => {
    assert.deepEqual(
      providerConfigOptions({
        oneOf: [
          { const: "auto", title: "Automatic" },
          { const: "opus", title: "Claude Opus" },
        ],
      }),
      [
        { label: "Automatic", value: "auto" },
        { label: "Claude Opus", value: "opus" },
      ],
    );
  });

  it("provides a bounded boolean choice", () => {
    assert.deepEqual(providerConfigOptions({ type: "boolean" }), [
      { label: "Yes", value: "true" },
      { label: "No", value: "false" },
    ]);
  });

  it("uses provider-owned labels for enum values", () => {
    assert.deepEqual(
      providerConfigOptions({
        enum: ["codex", "claude-code"],
        "x-enum-labels": {
          codex: "Codex CLI",
          "claude-code": "Claude Code",
        },
      }),
      [
        { label: "Codex CLI", value: "codex" },
        { label: "Claude Code", value: "claude-code" },
      ],
    );
  });

  it("resolves and filters dependent provider-owned options", () => {
    const property = {
      "x-options-by-field": {
        field: "harness",
        options: {
          codex: [
            { value: "gpt", label: "GPT", selector_kind: "version" },
            { value: "stable", label: "Stable", selector_kind: "track" },
          ],
        },
      },
      "x-option-filter": {
        field: "model_mode",
        option_property: "selector_kind",
      },
    };
    assert.deepEqual(
      providerConfigOptions(property, {
        harness: "codex",
        model_mode: "track",
      }),
      [{ value: "stable", label: "Stable", selector_kind: "track" }],
    );
  });

  it("hides fields when their bounded option set is empty", () => {
    const property = {
      "x-hide-when-no-options": true,
      "x-options-by-fields": {
        fields: ["harness", "model_selector"],
        options: { "codex|": [{ value: "auto", label: "Automatic" }] },
      },
    };
    assert.equal(
      providerConfigFieldVisible(property, {
        harness: "claude-code",
        model_selector: "",
      }),
      false,
    );
  });

  it("clears a dependent value that is invalid for the new selection", () => {
    const entries = [
      ["harness", { enum: ["codex", "claude-code"] }],
      [
        "model",
        {
          default: "auto",
          "x-options-by-field": {
            field: "harness",
            options: {
              codex: [{ value: "gpt", label: "GPT" }],
              "claude-code": [{ value: "opus", label: "Opus" }],
            },
          },
        },
      ],
    ];
    assert.deepEqual(
      reconcileProviderConfig(
        entries,
        { harness: "codex", model: "gpt" },
        "harness",
        "claude-code",
      ),
      { harness: "claude-code", model: "" },
    );
  });
});

describe("providerSchemaUsesExtendedPresentation", () => {
  it("leaves upstream string-only schemas on the upstream renderer", () => {
    assert.equal(
      providerSchemaUsesExtendedPresentation({
        properties: { label: { type: "string", default: "" } },
      }),
      false,
    );
  });

  it("routes bounded and dependent schemas through the generic extension", () => {
    assert.equal(
      providerSchemaUsesExtendedPresentation({
        properties: { harness: { enum: ["codex", "claude-code"] } },
      }),
      true,
    );
    assert.equal(
      providerSchemaUsesExtendedPresentation({
        properties: {
          model: { "x-visible-when": { field: "harness", equals: "codex" } },
        },
      }),
      true,
    );
  });
});
