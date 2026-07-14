/**
 * Unit tests for the McpServersEditor state helpers.
 *
 * Tests four invariants:
 *
 *   1. validateMcpServerName / validateMcpServerRow mirror the Rust
 *      `validate_mcp_servers` grammar (name charset, length, `__`,
 *      reserved name, uniqueness, command-required-when-enabled).
 *   2. toRows / toServers round-trip a server list, and toServers skips
 *      empty-name rows and blank arg/env rows (no phantom empty args or
 *      env entries reach the emitted config).
 *   3. serversEqual detects the resync-guard transitions the effect relies
 *      on (reordering, field-level diffs, arg/env diffs).
 *   4. mergeMcpServersByName replaces whole entries by name across layers,
 *      in the same order the Rust `merge_mcp_servers` BTreeMap produces.
 *
 * These are pure-logic tests — no React renderer needed.
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  validateMcpServerName,
  validateMcpServerRow,
  validateMcpServerArg,
  validateMcpServerEnvEntry,
  validateMcpServerListPayload,
  toRows,
  toServers,
  serversEqual,
  mergeMcpServersByName,
  effectiveEnabledCount,
  MAX_USER_MCP_SERVERS,
  MAX_ENV_VALUE_BYTES,
  MAX_ENV_TOTAL_BYTES,
  MCP_SERVER_NAME_MAX_LEN,
  MCP_RESERVED_SERVER_NAME,
  RESERVED_ENV_KEYS,
} from "./McpServersEditor.tsx";

// ── Invariant 1: name grammar mirrors validate_mcp_servers ─────────────────

test("validateMcpServerName_empty_name_is_required", () => {
  assert.equal(validateMcpServerName(""), "Name is required.");
});

test("validateMcpServerName_valid_name_returns_null", () => {
  assert.equal(validateMcpServerName("my-server_1"), null);
});

test("validateMcpServerName_over_max_length_rejected", () => {
  const tooLong = "a".repeat(MCP_SERVER_NAME_MAX_LEN + 1);
  const error = validateMcpServerName(tooLong);
  assert.match(error, /exceeds the maximum length/);
});

test("validateMcpServerName_at_max_length_accepted", () => {
  const atMax = "a".repeat(MCP_SERVER_NAME_MAX_LEN);
  assert.equal(validateMcpServerName(atMax), null);
});

test("validateMcpServerName_rejects_non_ascii_alnum_characters", () => {
  // Rust grammar: ASCII alphanumeric, `_`, `-` only. Space, dot, and
  // non-ASCII letters must all be rejected.
  assert.match(validateMcpServerName("my server"), /Only letters/);
  assert.match(validateMcpServerName("my.server"), /Only letters/);
  assert.match(validateMcpServerName("café"), /Only letters/);
});

test("validateMcpServerName_rejects_double_underscore", () => {
  assert.match(validateMcpServerName("my__server"), /cannot contain/);
});

test("validateMcpServerName_rejects_reserved_name", () => {
  const error = validateMcpServerName(MCP_RESERVED_SERVER_NAME);
  assert.match(error, /is reserved/);
});

test("validateMcpServerRow_flags_duplicate_name_in_layer", () => {
  const row = { name: "shared", command: "npx", enabled: true };
  const error = validateMcpServerRow(row, new Set(["shared"]));
  assert.equal(error, "Server names must be unique.");
});

test("validateMcpServerRow_enabled_without_command_is_rejected", () => {
  const row = { name: "srv", command: "", enabled: true };
  const error = validateMcpServerRow(row, new Set());
  assert.equal(error, "Command is required for an enabled server.");
});

test("validateMcpServerRow_disabled_without_command_is_valid", () => {
  // A disabled row's only purpose may be to mask a lower-precedence
  // server by name, so it may omit command (mirrors Rust: the
  // command-required check is gated on `server.enabled`).
  const row = { name: "srv", command: "", enabled: false };
  assert.equal(validateMcpServerRow(row, new Set()), null);
});

test("validateMcpServerRow_valid_enabled_row_returns_null", () => {
  const row = { name: "srv", command: "npx", enabled: true };
  assert.equal(validateMcpServerRow(row, new Set(["other"])), null);
});

// ── Invariant 2: toRows / toServers round-trip and blank-row skipping ──────

test("toRows_produces_one_row_per_server_with_stable_shape", () => {
  const servers = [
    {
      name: "fs",
      command: "npx",
      args: ["-y", "@modelcontextprotocol/server-filesystem"],
      env: [{ name: "ROOT", value: "/tmp" }],
      enabled: true,
    },
  ];
  const rows = toRows(servers);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].name, "fs");
  assert.equal(rows[0].command, "npx");
  assert.deepEqual(
    rows[0].args.map((a) => a.value),
    ["-y", "@modelcontextprotocol/server-filesystem"],
  );
  assert.deepEqual(
    rows[0].env.map((e) => [e.name, e.value]),
    [["ROOT", "/tmp"]],
  );
  assert.equal(rows[0].enabled, true);
  assert.equal(typeof rows[0].id, "string");
});

test("toServers_round_trips_toRows_output", () => {
  const servers = [
    {
      name: "fs",
      command: "npx",
      args: ["-y", "server"],
      env: [{ name: "ROOT", value: "/tmp" }],
      enabled: true,
    },
    { name: "masked", command: "", args: [], env: [], enabled: false },
  ];
  const rows = toRows(servers);
  assert.ok(serversEqual(toServers(rows), servers));
});

test("toServers_skips_empty_name_rows", () => {
  // A freshly-added row with no name typed yet must not become a
  // phantom entry in the emitted config.
  const rows = toRows([]);
  rows.push({
    id: "new",
    name: "",
    command: "npx",
    args: [],
    env: [],
    enabled: true,
  });
  assert.deepEqual(toServers(rows), []);
});

test("toServers_skips_blank_arg_rows", () => {
  // An unfilled "Add argument" row must not inject an empty-string arg
  // into the spawned command.
  const rows = toRows([
    { name: "srv", command: "npx", args: ["-y"], env: [], enabled: true },
  ]);
  rows[0].args.push({ id: "blank", value: "" });
  const [server] = toServers(rows);
  assert.deepEqual(server.args, ["-y"]);
});

test("toServers_skips_blank_env_rows_but_keeps_named_ones", () => {
  const rows = toRows([
    { name: "srv", command: "npx", args: [], env: [], enabled: true },
  ]);
  rows[0].env.push({ id: "blank", name: "", value: "leftover" });
  rows[0].env.push({ id: "named", name: "TOKEN", value: "abc" });
  const [server] = toServers(rows);
  assert.deepEqual(server.env, [{ name: "TOKEN", value: "abc" }]);
});

test("toServers_duplicate_env_names_collapse_last_write_wins", () => {
  // The backend's per-server env is a Vec, not a map, so it rejects
  // duplicate names outright (`mcp_servers.rs` "has duplicate env var").
  // The editor must dedupe client-side before that round-trip, the same
  // way EnvVarsEditor's object-keyed `toRecord` does implicitly.
  const rows = toRows([
    { name: "srv", command: "npx", args: [], env: [], enabled: true },
  ]);
  rows[0].env.push({ id: "a", name: "TOKEN", value: "first" });
  rows[0].env.push({ id: "b", name: "TOKEN", value: "second" });
  const [server] = toServers(rows);
  assert.deepEqual(server.env, [{ name: "TOKEN", value: "second" }]);
});

// ── Invariant 3: serversEqual — resync-guard transitions ───────────────────

test("serversEqual_identical_lists_are_equal", () => {
  const servers = [
    { name: "a", command: "npx", args: ["-y"], env: [], enabled: true },
  ];
  assert.ok(serversEqual(servers, structuredClone(servers)));
});

test("serversEqual_different_lengths_are_not_equal", () => {
  const a = [{ name: "a", command: "npx", args: [], env: [], enabled: true }];
  assert.equal(serversEqual(a, []), false);
});

test("serversEqual_detects_arg_diff", () => {
  const a = [
    { name: "a", command: "npx", args: ["-y"], env: [], enabled: true },
  ];
  const b = [
    { name: "a", command: "npx", args: ["-x"], env: [], enabled: true },
  ];
  assert.equal(serversEqual(a, b), false);
});

test("serversEqual_detects_env_diff", () => {
  const a = [
    {
      name: "a",
      command: "npx",
      args: [],
      env: [{ name: "K", value: "1" }],
      enabled: true,
    },
  ];
  const b = [
    {
      name: "a",
      command: "npx",
      args: [],
      env: [{ name: "K", value: "2" }],
      enabled: true,
    },
  ];
  assert.equal(serversEqual(a, b), false);
});

test("serversEqual_detects_enabled_diff", () => {
  const a = [{ name: "a", command: "npx", args: [], env: [], enabled: true }];
  const b = [{ name: "a", command: "npx", args: [], env: [], enabled: false }];
  assert.equal(serversEqual(a, b), false);
});

// ── Invariant 4: mergeMcpServersByName — layered replace-by-name ──────────

test("mergeMcpServersByName_higher_layer_replaces_whole_entry", () => {
  const global = [
    {
      name: "fs",
      command: "npx",
      args: ["-y", "global-pkg"],
      env: [],
      enabled: true,
    },
  ];
  const agent = [
    {
      name: "fs",
      command: "npx",
      args: ["-y", "agent-pkg"],
      env: [],
      enabled: true,
    },
  ];
  const merged = mergeMcpServersByName(global, agent);
  assert.equal(merged.length, 1);
  assert.deepEqual(merged[0].args, ["-y", "agent-pkg"]);
});

test("mergeMcpServersByName_disabled_higher_layer_masks_lower_layer", () => {
  const global = [
    { name: "fs", command: "npx", args: ["-y"], env: [], enabled: true },
  ];
  const definition = [
    { name: "fs", command: "", args: [], env: [], enabled: false },
  ];
  const merged = mergeMcpServersByName(global, definition);
  assert.equal(merged.length, 1);
  assert.equal(
    merged[0].enabled,
    false,
    "disabled override must mask, not disappear",
  );
});

test("mergeMcpServersByName_distinct_names_from_all_layers_are_kept", () => {
  const global = [
    { name: "g", command: "npx", args: [], env: [], enabled: true },
  ];
  const definition = [
    { name: "d", command: "npx", args: [], env: [], enabled: true },
  ];
  const agent = [
    { name: "a", command: "npx", args: [], env: [], enabled: true },
  ];
  const merged = mergeMcpServersByName(global, definition, agent);
  assert.deepEqual(
    merged.map((s) => s.name),
    ["a", "d", "g"],
  );
});

test("mergeMcpServersByName_empty_layers_produce_empty_result", () => {
  assert.deepEqual(mergeMcpServersByName([], [], []), []);
});

test("mergeMcpServersByName_sorts_by_ascii_byte_order_not_locale", () => {
  // Rust BTreeMap<String, _> orders by byte value: '_' (0x5F) > 'Z' (0x5A)
  // > 'A' (0x41), and uppercase < lowercase. localeCompare would put
  // these in a different order (e.g. case-insensitive or locale-dependent).
  const servers = [
    { name: "beta", command: "", args: [], env: [], enabled: true },
    { name: "Alpha", command: "", args: [], env: [], enabled: true },
    { name: "_underscore", command: "", args: [], env: [], enabled: true },
    { name: "alpha", command: "", args: [], env: [], enabled: true },
  ];
  const merged = mergeMcpServersByName(servers);
  // ASCII: 'A'(65) < '_'(95) < 'a'(97) < 'b'(98)
  assert.deepEqual(
    merged.map((s) => s.name),
    ["Alpha", "_underscore", "alpha", "beta"],
  );
});

// ── Invariant 5: effectiveEnabledCount — cap across layers ──────────────

function makeServer(name, enabled = true) {
  return { name, command: "npx", args: [], env: [], enabled };
}

test("effectiveEnabledCount_local_only", () => {
  const local = [makeServer("a"), makeServer("b", false)];
  assert.equal(effectiveEnabledCount(local, []), 1);
});

test("effectiveEnabledCount_inherited_not_overridden", () => {
  const local = [makeServer("a")];
  const inherited = [makeServer("b"), makeServer("c")];
  assert.equal(effectiveEnabledCount(local, inherited), 3);
});

test("effectiveEnabledCount_inherited_overridden_by_local_not_counted", () => {
  // Local row overrides inherited — only the local row's enabled state
  // counts, not the inherited one.
  const local = [makeServer("a", false)];
  const inherited = [makeServer("a"), makeServer("b")];
  // "a" overridden (local disabled → 0), "b" inherited enabled → 1
  assert.equal(effectiveEnabledCount(local, inherited), 1);
});

test("effectiveEnabledCount_disabled_inherited_not_counted", () => {
  const local = [];
  const inherited = [makeServer("a", false), makeServer("b")];
  assert.equal(effectiveEnabledCount(local, inherited), 1);
});

test("effectiveEnabledCount_15_inherited_plus_1_local_hits_cap", () => {
  const inherited = Array.from({ length: MAX_USER_MCP_SERVERS }, (_, i) =>
    makeServer(`inh-${i}`),
  );
  const local = [makeServer("local-1")];
  assert.equal(
    effectiveEnabledCount(local, inherited),
    MAX_USER_MCP_SERVERS + 1,
  );
});

test("effectiveEnabledCount_toggling_disabled_to_16th_hits_cap", () => {
  // Simulates toggling a disabled local row to enabled when 14 inherited +
  // 1 other local are already enabled = 15. The 16th would exceed the cap.
  const inherited = Array.from({ length: 14 }, (_, i) =>
    makeServer(`inh-${i}`),
  );
  const local = [makeServer("local-1"), makeServer("local-2", false)];
  // Before toggle: 1 local enabled + 14 inherited = 15 (at cap)
  assert.equal(effectiveEnabledCount(local, inherited), MAX_USER_MCP_SERVERS);
  // After toggle: 2 local enabled + 14 inherited = 16 (exceeds cap)
  const toggled = local.map((s) =>
    s.name === "local-2" ? { ...s, enabled: true } : s,
  );
  assert.equal(
    effectiveEnabledCount(toggled, inherited),
    MAX_USER_MCP_SERVERS + 1,
  );
});

// ── Invariant 6: validateMcpServerEnvEntry — per-server env boundary ────

test("validateMcpServerEnvEntry_valid_entry_returns_null", () => {
  assert.equal(
    validateMcpServerEnvEntry({ name: "MY_TOKEN", value: "abc" }),
    null,
  );
});

test("validateMcpServerEnvEntry_empty_name_returns_null_skipped", () => {
  // Blank rows are skipped by toServers, so no error shown.
  assert.equal(
    validateMcpServerEnvEntry({ name: "", value: "anything" }),
    null,
  );
});

test("validateMcpServerEnvEntry_malformed_key_rejected", () => {
  const err = validateMcpServerEnvEntry({ name: "1BAD", value: "" });
  assert.match(err, /must match/);
});

test("validateMcpServerEnvEntry_key_with_equals_rejected", () => {
  const err = validateMcpServerEnvEntry({ name: "KEY=val", value: "" });
  assert.match(err, /must match/);
});

test("validateMcpServerEnvEntry_key_with_spaces_rejected", () => {
  const err = validateMcpServerEnvEntry({ name: "MY KEY", value: "" });
  assert.match(err, /must match/);
});

test("validateMcpServerEnvEntry_reserved_key_rejected", () => {
  const err = validateMcpServerEnvEntry({
    name: "BUZZ_ACP_MCP_SERVERS",
    value: "",
  });
  assert.match(err, /reserved/);
});

test("validateMcpServerEnvEntry_reserved_key_case_insensitive", () => {
  const err = validateMcpServerEnvEntry({
    name: "buzz_private_key",
    value: "",
  });
  assert.match(err, /reserved/);
});

test("validateMcpServerEnvEntry_nul_in_value_rejected", () => {
  const err = validateMcpServerEnvEntry({ name: "TOKEN", value: "abc\0def" });
  assert.match(err, /NUL/);
});

test("validateMcpServerEnvEntry_oversized_value_rejected", () => {
  const err = validateMcpServerEnvEntry({
    name: "TOKEN",
    value: "x".repeat(MAX_ENV_VALUE_BYTES + 1),
  });
  assert.match(err, /exceeds/);
});

test("validateMcpServerEnvEntry_at_limit_value_accepted", () => {
  assert.equal(
    validateMcpServerEnvEntry({
      name: "TOKEN",
      value: "x".repeat(MAX_ENV_VALUE_BYTES),
    }),
    null,
  );
});

test("validateMcpServerEnvEntry_all_reserved_keys_are_rejected", () => {
  for (const key of RESERVED_ENV_KEYS) {
    const err = validateMcpServerEnvEntry({ name: key, value: "" });
    assert.ok(err, `Expected "${key}" to be rejected as reserved`);
  }
});

// ── Invariant 7: validateMcpServerRow — command NUL + size ──────────────

test("validateMcpServerRow_command_nul_rejected", () => {
  const err = validateMcpServerRow(
    { name: "srv", command: "npx\0bad", enabled: true },
    new Set(),
  );
  assert.match(err, /NUL/);
});

test("validateMcpServerRow_command_oversize_rejected", () => {
  const err = validateMcpServerRow(
    {
      name: "srv",
      command: "x".repeat(MAX_ENV_VALUE_BYTES + 1),
      enabled: true,
    },
    new Set(),
  );
  assert.match(err, /exceeds/);
});

test("validateMcpServerRow_command_at_limit_accepted", () => {
  assert.equal(
    validateMcpServerRow(
      { name: "srv", command: "x".repeat(MAX_ENV_VALUE_BYTES), enabled: true },
      new Set(),
    ),
    null,
  );
});

// ── Invariant 8: validateMcpServerArg — arg NUL + size ──────────────────

test("validateMcpServerArg_nul_rejected", () => {
  assert.match(validateMcpServerArg("arg\0val"), /NUL/);
});

test("validateMcpServerArg_oversize_rejected", () => {
  assert.match(
    validateMcpServerArg("x".repeat(MAX_ENV_VALUE_BYTES + 1)),
    /exceeds/,
  );
});

test("validateMcpServerArg_valid_accepted", () => {
  assert.equal(validateMcpServerArg("--flag"), null);
});

// ── Invariant 9: validateMcpServerListPayload — aggregate cap ───────────

test("validateMcpServerListPayload_under_limit_accepted", () => {
  assert.equal(
    validateMcpServerListPayload([
      { name: "srv", command: "npx", args: ["-y"], env: [], enabled: true },
    ]),
    null,
  );
});

test("validateMcpServerListPayload_over_limit_rejected", () => {
  // Nine servers, each with an env value just under 32KB → total > 256KB.
  const servers = Array.from({ length: 9 }, (_, i) => ({
    name: `srv-${i}`,
    command: "npx",
    args: [],
    env: [{ name: "DATA", value: "x".repeat(MAX_ENV_VALUE_BYTES - 1) }],
    enabled: true,
  }));
  const err = validateMcpServerListPayload(servers);
  assert.ok(err, "aggregate payload should be rejected");
  assert.match(err, /limit/);
});

test("validateMcpServerListPayload_at_limit_accepted", () => {
  // A single server whose total payload is exactly at the limit.
  // Total = name + command + env-key + env-value bytes.
  const budget = MAX_ENV_TOTAL_BYTES;
  const name = "s";
  const command = "c";
  const envKey = "K";
  const overhead = name.length + command.length + envKey.length;
  const value = "x".repeat(budget - overhead);
  assert.equal(
    validateMcpServerListPayload([
      {
        name,
        command,
        args: [],
        env: [{ name: envKey, value }],
        enabled: true,
      },
    ]),
    null,
  );
});
