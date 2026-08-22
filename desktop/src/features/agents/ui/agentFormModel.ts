/**
 * agentFormModel.ts — Canonical field model for the merged agent edit surface.
 *
 * One typed AgentFormModel is consumed by every edit route (Phase 1). The
 * create path extends the same model in Phase 2.
 *
 * Sections per Artifact 4:
 *   Identity        — name, avatar
 *   Behavior        — prompt, respond-to+allowlist, parallelism
 *   Runtime         — harness, model, provider, env vars, tuning
 *   Device policy   — auto-restart, start-on-launch (local backend only)
 *   Definition admin— name pool, share action, and definition-level actions
 *
 * Field ownership tags per Artifact 1:
 *   definition     — D-field: shared across linked siblings via definition
 *   instance       — I-field: this agent only
 *   local-policy   — L-field: dedicated setter, never on the relay
 *   create-only    — C-field: expressible at create time only
 *
 * Editability predicate:
 *   team-managed D-fields → read-only (D-fields render read-only)
 *   built-in → fully editable D-fields (only actions differ, not field editability)
 *   unlinked agent → definition sections absent
 *   zero-instance definition → instance + policy sections absent
 *   identity-archived agent → fully editable, Save never unarchives
 */

import type {
  AgentPersona,
  ManagedAgent,
  UpdatePersonaInput,
  UpdateManagedAgentInput,
} from "@/shared/api/types";

// ── Field ownership ─────────────────────────────────────────────────────────

export type FieldOwner =
  | "definition"
  | "instance"
  | "local-policy"
  | "create-only";

// ── Context shape that drives editability ───────────────────────────────────

/**
 * Describes which combination of definition / instance is being edited.
 *
 * - definition-only: zero-instance definition (from agents library)
 * - instance-with-definition: linked agent (most common case, R1–R7)
 * - instance-only: unlinked agent (no definition)
 */
export type AgentEditContext =
  | { kind: "definition-only"; definition: AgentPersona }
  | {
      kind: "instance-with-definition";
      definition: AgentPersona;
      instance: ManagedAgent;
    }
  | { kind: "instance-only"; instance: ManagedAgent };

export function editContextDefinition(
  ctx: AgentEditContext,
): AgentPersona | null {
  if (ctx.kind === "instance-only") return null;
  return ctx.definition;
}

export function editContextInstance(
  ctx: AgentEditContext,
): ManagedAgent | null {
  if (ctx.kind === "definition-only") return null;
  return ctx.instance;
}

/** True when D-fields should render read-only (team-managed). */
export function isDefinitionReadOnly(ctx: AgentEditContext): boolean {
  const def = editContextDefinition(ctx);
  return def?.sourceTeam != null && def.sourceTeam.length > 0;
}

/** True when definition sections should be rendered at all. */
export function hasDefinitionContext(ctx: AgentEditContext): boolean {
  return ctx.kind !== "instance-only";
}

/** True when instance+policy sections should be rendered. */
export function hasInstanceContext(ctx: AgentEditContext): boolean {
  return ctx.kind !== "definition-only";
}

// ── AgentFormModel ──────────────────────────────────────────────────────────

/**
 * Flat, typed field model covering every editable field on the merged surface.
 *
 * Fields absent for a given context are undefined. Editability is determined
 * by AgentEditContext + the isDefinitionReadOnly predicate, NOT by AgentFormModel
 * itself — the model just holds current values.
 *
 * Ownership by context (rows 9–10 contract):
 *   respondTo / allowlist / parallelism:
 *     instance-with-definition / instance-only → I-owned; edits go to agentInput only.
 *     definition-only (zero-instance)          → D-owned; edits go to personaInput.
 */
export type AgentFormModel = {
  // Identity (D when linked, I when unlinked)
  displayName: string;
  avatarUrl: string;

  // Behavior — D when linked for prompt; respondTo/parallelism always I when instance present
  systemPrompt: string;
  /** Rows 9–10: seeds from instance when instance is present; definition default only for definition-only. */
  respondTo: ManagedAgent["respondTo"] | AgentPersona["respondTo"] | null;
  respondToAllowlist: string[];

  // Runtime — D preferred-id field (definition runtime); harness pin is I (tracked separately in dialog)
  /** Preferred runtime id from the definition. Only used in definition-only context for definition runtime edits. */
  runtime: string | undefined;
  model: string | null;
  provider: string | null;
  envVars: Record<string, string>;

  // Definition-only fields
  /** Name pool for definition. Undefined when no definition context. */
  namePool: string[] | undefined;

  // Instance-level fields
  /** Instance name (per-pool override of displayName). */
  instanceName: string | undefined;
  /**
   * Instance-level env-var overlay (row 8 contract). When a definition is
   * present the definition env is inherited; this field holds the per-instance
   * additions/overrides. Undefined when no instance context.
   */
  instanceEnvVars: Record<string, string> | undefined;
  /** Rows 9–10: I-owned when instance present; D-owned (definition default) in definition-only context. */
  parallelism: number | null;

  // Instance harness pin (I-fields) — undefined when no instance context.
  /** True when the harness is inherited from the definition/default (not pinned). */
  harnessInherit: boolean | undefined;
  /**
   * Resolved effective harness command to pin. The dialog resolves this from
   * the selected runtime's command (or the manual entry) before it reaches the
   * model; ignored when harnessInherit is true.
   */
  harnessCommand: string | undefined;
  /** Resolved harness args; ignored when harnessInherit is true. */
  harnessArgs: string[] | undefined;
  /** ACP command override. */
  acpCommand: string | undefined;

  // Device policy (L-fields)
  autoRestartOnConfigChange: boolean | undefined;
  startOnAppLaunch: boolean | undefined;
};

/**
 * Source-of-truth field ownership map (Artifact 4).
 *
 * Declares the base FieldOwner for every AgentFormModel field. Two fields
 * (respondTo, parallelism) have context-dependent ownership per rows 9–10:
 * they are I-owned when an instance is in context, and D-owned only in
 * definition-only (zero-instance) context. `fieldOwner()` resolves this
 * context dependence.
 *
 * The ownership map is the single authority for BOTH routing and editability:
 *   - `emitAgentFormDiff` consults `fieldOwner` to route every field's change.
 *   - `fieldEditable` derives per-field enabled/disabled from `fieldOwner` +
 *     `isDefinitionReadOnly` (a D-owned field is read-only when the definition
 *     is team-managed; I- and L-owned fields stay editable regardless).
 *   - `definitionFieldsDirty` iterates the map to derive the catalog-publish
 *     dirty signal from D-owned field changes — no parallel dirty boolean.
 */
export const FIELD_OWNERS: Record<keyof AgentFormModel, FieldOwner> = {
  // Identity
  displayName: "definition",
  avatarUrl: "definition",
  // Behavior
  systemPrompt: "definition",
  respondTo: "instance", // rows 9–10: I when instance present, D in definition-only
  respondToAllowlist: "instance",
  parallelism: "instance", // rows 9–10: same contract as respondTo
  // Runtime — D preferred-id; per-instance env overlay is I
  runtime: "definition",
  model: "definition",
  provider: "definition",
  envVars: "definition",
  instanceEnvVars: "instance",
  namePool: "definition",
  instanceName: "instance",
  // Instance harness pin — I-owned
  harnessInherit: "instance",
  harnessCommand: "instance",
  harnessArgs: "instance",
  acpCommand: "instance",
  // Device policy
  autoRestartOnConfigChange: "local-policy",
  startOnAppLaunch: "local-policy",
};

/**
 * Resolve the effective FieldOwner for a field given the current edit context.
 *
 * For most fields this is a direct lookup into FIELD_OWNERS. Two sets of
 * context-dependent overrides apply:
 *
 * 1. Rows 9–10 dual-owner fields (respondTo, respondToAllowlist, parallelism):
 *    base entry is "instance", but in definition-only context (no instance) they
 *    are D-owned (definition default).
 *
 * 2. Definition-or-instance fields (systemPrompt, model, provider): base entry
 *    is "definition" (D-field when a definition is present), but in instance-only
 *    context (no definition) they fall back to I-owned.
 *
 * `emitAgentFormDiff` consults this function for every emit routing decision.
 * Dialog editability (enabled/disabled controls) is derived from the same
 * function via `fieldEditable`, and the catalog-publish dirty signal via
 * `definitionFieldsDirty` — the ownership map is the single authority for both.
 */
export function fieldOwner(
  field: keyof AgentFormModel,
  ctx: AgentEditContext,
): FieldOwner {
  const base = FIELD_OWNERS[field];

  // Rows 9–10: respondTo/respondToAllowlist/parallelism are I-owned when an
  // instance is present. In definition-only context (no instance) they are
  // D-owned (definition default).
  if (
    base === "instance" &&
    (field === "respondTo" ||
      field === "respondToAllowlist" ||
      field === "parallelism") &&
    ctx.kind === "definition-only"
  ) {
    return "definition";
  }

  // Definition-fallback fields: D-owned when a definition is present, I-owned
  // in instance-only context (no definition). systemPrompt, model, provider are
  // "definition" in the base map, but must be treated as I-owned for unlinked agents.
  if (
    base === "definition" &&
    (field === "systemPrompt" || field === "model" || field === "provider") &&
    ctx.kind === "instance-only"
  ) {
    return "instance";
  }

  return base;
}

/**
 * Whether a field's control should be editable in the given context.
 *
 * Derived from `fieldOwner`: a D-owned field is read-only when the definition
 * is team-managed (`isDefinitionReadOnly`); I- and L-owned fields are always
 * editable — team management of the definition never disables instance-owned
 * controls (row 9 contract: team-linked live access stays I-editable).
 */
export function fieldEditable(
  field: keyof AgentFormModel,
  ctx: AgentEditContext,
): boolean {
  return !(
    fieldOwner(field, ctx) === "definition" && isDefinitionReadOnly(ctx)
  );
}

/**
 * Whether any D-owned field differs between saved and next.
 *
 * Drives the catalog-publish affordance (shown once the user dirties a D-field
 * on a shared definition). Iterates the ownership map so the dirty signal comes
 * from the same authority that routes emits — no parallel dirty boolean.
 */
export function definitionFieldsDirty(
  saved: AgentFormModel,
  next: AgentFormModel,
  ctx: AgentEditContext,
): boolean {
  return (Object.keys(FIELD_OWNERS) as Array<keyof AgentFormModel>).some(
    (field) =>
      fieldOwner(field, ctx) === "definition" &&
      !fieldValueEqual(field, saved[field], next[field]),
  );
}

/** Canonical per-field equality used by `definitionFieldsDirty`. */
function fieldValueEqual(
  field: keyof AgentFormModel,
  a: AgentFormModel[keyof AgentFormModel],
  b: AgentFormModel[keyof AgentFormModel],
): boolean {
  if (field === "namePool") {
    return namePoolEqual((a as string[]) ?? [], (b as string[]) ?? []);
  }
  if (field === "envVars" || field === "instanceEnvVars") {
    return envVarsMapEqual(
      (a as Record<string, string>) ?? {},
      (b as Record<string, string>) ?? {},
    );
  }
  if (field === "respondToAllowlist" || field === "harnessArgs") {
    return (
      ((a as string[]) ?? []).join(",") === ((b as string[]) ?? []).join(",")
    );
  }
  if (typeof a === "string" && typeof b === "string") {
    return a.trim() === b.trim();
  }
  return (a ?? null) === (b ?? null);
}

/** Which coordinator outputs changed vs. last saved state. */
export type AgentFormEmit = {
  /** D-field update payload, present iff a D-field changed AND the definition is editable. */
  personaInput: UpdatePersonaInput | null;
  /** I-field update payload, present iff an I-field changed. */
  agentInput: UpdateManagedAgentInput | null;
  /** Policy-setter calls, one entry per changed L-field. */
  policySets: Array<
    | { type: "autoRestart"; pubkey: string; value: boolean }
    | { type: "startOnAppLaunch"; pubkey: string; value: boolean }
  >;
};

// ── Canonical comparison helpers ─────────────────────────────────────────────

/** Map equality per Artifact 3 settlement contract. */
export function envVarsMapEqual(
  a: Record<string, string>,
  b: Record<string, string>,
): boolean {
  const aKeys = Object.keys(a);
  const bKeys = Object.keys(b);
  if (aKeys.length !== bKeys.length) return false;
  return aKeys.every((k) => a[k] === b[k]);
}

/** Canonical name-pool equality: order-sensitive, trimmed. */
export function namePoolEqual(a: string[], b: string[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((v, i) => v.trim() === b[i]?.trim());
}

// ── Edit adapters ────────────────────────────────────────────────────────────

/**
 * Seed an AgentFormModel from (definition?, agent?) — at least one present.
 *
 * For instance-with-definition: D-fields seed from the definition (authoritative
 * source); I-fields seed from the instance. For definition-only: instance fields
 * are undefined. For instance-only: definition fields are undefined.
 */
export function seedAgentFormModel(ctx: AgentEditContext): AgentFormModel {
  const def = editContextDefinition(ctx);
  const inst = editContextInstance(ctx);

  // D-fields: from definition when present, else from instance
  const displayName = def?.displayName ?? inst?.name ?? "";
  const avatarUrl = def?.avatarUrl ?? inst?.avatarUrl ?? "";
  const systemPrompt = def?.systemPrompt ?? inst?.systemPrompt ?? "";
  // Rows 9–10: respondTo/allowlist/parallelism seed from INSTANCE when present;
  // only fall back to definition default in definition-only (zero-instance) context.
  // Normalize null → "anyone" so the diff doesn't produce a false phantom write
  // when the DB stores null and the UI shows "anyone" as the default.
  const respondTo =
    inst != null ? (inst.respondTo ?? "anyone") : (def?.respondTo ?? null);
  const respondToAllowlist =
    inst != null
      ? (inst.respondToAllowlist ?? [])
      : (def?.respondToAllowlist ?? []);
  const runtime = def?.runtime ?? undefined;
  const model = def !== null ? (def.model ?? null) : (inst?.model ?? null);
  const provider =
    def !== null ? (def.provider ?? null) : (inst?.provider ?? null);
  // Env: D-field base. Instance overlay is applied separately at spawn;
  // the form edits each store independently (row 8 contract).
  const envVars = def?.envVars ?? inst?.envVars ?? {};
  const namePool = def?.namePool;

  // I-fields
  const instanceName = inst?.name;
  // Row 8 contract: when a definition is present, instance env is a per-agent
  // overlay that does NOT wholesale-replace the definition env. Seed from
  // inst.envVars directly (not the merged/inherited snapshot).
  const instanceEnvVars = inst != null ? (inst.envVars ?? {}) : undefined;
  // Rows 9–10: parallelism seeds from instance when present; definition default for definition-only.
  const parallelism =
    inst != null ? (inst.parallelism ?? null) : (def?.parallelism ?? null);

  // L-fields
  const autoRestartOnConfigChange = inst?.autoRestartOnConfigChange;
  const startOnAppLaunch = inst?.startOnAppLaunch;

  // Harness pin (I-fields). Inherit when the instance is linked and carries no
  // command override; otherwise the stored command is a pin.
  const harnessInherit =
    inst != null
      ? inst.personaId != null && inst.agentCommandOverride == null
      : undefined;
  const harnessCommand = inst?.agentCommand;
  const harnessArgs = inst?.agentArgs;
  const acpCommand = inst?.acpCommand;

  return {
    displayName,
    avatarUrl,
    systemPrompt,
    respondTo,
    respondToAllowlist,
    runtime,
    model,
    provider,
    envVars,
    namePool,
    instanceName,
    instanceEnvVars,
    parallelism,
    harnessInherit,
    harnessCommand,
    harnessArgs,
    acpCommand,
    autoRestartOnConfigChange,
    startOnAppLaunch,
  };
}

/**
 * Emit coordinator inputs from a (previous model / saved state) pair.
 *
 * Returns personaInput when a D-field changed AND the definition is editable.
 * Returns agentInput when an I-field changed.
 * Returns policySets for any L-field changed.
 *
 * Routing decisions — which layer each field's change belongs to — are made
 * via `fieldOwner(field, ctx)`, which reads from `FIELD_OWNERS` and handles
 * the rows-9–10 context dependence. This makes `FIELD_OWNERS` the single
 * authoritative source for ownership routing in this function.
 *
 * The caller (save coordinator) calls this after re-fetching observed state,
 * so "previous" is the re-fetched stored state and "next" is what the user
 * submitted — the diff is exactly what hasn't been persisted yet.
 */
export function emitAgentFormDiff(
  saved: AgentFormModel,
  next: AgentFormModel,
  ctx: AgentEditContext,
  _options?: { publishCatalogUpdates?: boolean },
): AgentFormEmit {
  const def = editContextDefinition(ctx);
  const inst = editContextInstance(ctx);
  const defReadOnly = isDefinitionReadOnly(ctx);

  // Helper: is a field D-owned in this context?
  const isD = (field: keyof AgentFormModel) =>
    fieldOwner(field, ctx) === "definition";
  // Helper: is a field I-owned in this context?
  const isI = (field: keyof AgentFormModel) =>
    fieldOwner(field, ctx) === "instance";

  // D-field diff — only when definition is present and NOT team-managed
  let personaInput: UpdatePersonaInput | null = null;
  if (def !== null && !defReadOnly) {
    const dChanged =
      (isD("displayName") &&
        next.displayName.trim() !== saved.displayName.trim()) ||
      (isD("avatarUrl") &&
        (next.avatarUrl ?? "") !== (saved.avatarUrl ?? "")) ||
      (isD("systemPrompt") &&
        next.systemPrompt.trim() !== saved.systemPrompt.trim()) ||
      (isD("runtime") && next.runtime !== saved.runtime) ||
      (isD("model") && (next.model ?? null) !== (saved.model ?? null)) ||
      (isD("provider") &&
        (next.provider ?? null) !== (saved.provider ?? null)) ||
      (isD("envVars") &&
        !envVarsMapEqual(next.envVars ?? {}, saved.envVars ?? {})) ||
      (isD("namePool") &&
        !namePoolEqual(next.namePool ?? [], saved.namePool ?? [])) ||
      // respondTo/parallelism: D-owned only in definition-only context (rows 9–10)
      (isD("respondTo") &&
        (next.respondTo ?? null) !== (saved.respondTo ?? null)) ||
      (isD("respondToAllowlist") &&
        next.respondTo === "allowlist" &&
        next.respondToAllowlist.join(",") !==
          (saved.respondToAllowlist ?? []).join(",")) ||
      // D-parallelism: included in dChanged so the owner-only build-lock gate
      // can suppress it, and so that a parallelism-only change in definition-only
      // context actually emits a personaInput (Thufir pass-3 CRITICAL-1).
      (isD("parallelism") &&
        (next.parallelism ?? null) !== (saved.parallelism ?? null));

    if (dChanged) {
      personaInput = {
        id: def.id,
        displayName: next.displayName.trim(),
        avatarUrl: next.avatarUrl ?? "",
        systemPrompt: next.systemPrompt.trim(),
        runtime: next.runtime,
        model: next.model ?? undefined,
        provider: next.provider ?? undefined,
        namePool: next.namePool ?? [],
        envVars: next.envVars ?? {},
        // Include behavior block only when respondTo/parallelism are D-owned
        // (definition-only context per rows 9–10).
        behavior:
          isD("respondTo") || isD("parallelism")
            ? {
                respondTo:
                  isD("respondTo") && next.respondTo != null
                    ? next.respondTo
                    : undefined,
                respondToAllowlist:
                  isD("respondTo") && next.respondTo === "allowlist"
                    ? next.respondToAllowlist
                    : undefined,
                parallelism: isD("parallelism")
                  ? (next.parallelism ?? undefined)
                  : undefined,
              }
            : undefined,
      };
    }
  }

  // I-field diff — only when instance is present
  let agentInput: UpdateManagedAgentInput | null = null;
  if (inst !== null) {
    // Row 1 contract: stop materializing displayName→name when a name pool exists
    const hasNamePool = (def?.namePool?.length ?? 0) > 0;

    // Row 8 contract: instance env edit goes to instance, NOT wholesale-replace from definition.
    // When a definition is present, instanceEnvVars holds the per-instance overlay.
    const instanceEnvChanged =
      isI("instanceEnvVars") && next.instanceEnvVars !== undefined
        ? !envVarsMapEqual(next.instanceEnvVars ?? {}, inst.envVars ?? {})
        : false;

    const nameChanged =
      isI("instanceName") &&
      !hasNamePool &&
      (next.instanceName ?? next.displayName.trim()) !== inst.name;
    const systemPromptChanged =
      isI("systemPrompt") &&
      (next.systemPrompt.trim() || null) !== (inst.systemPrompt ?? null);
    const modelChanged =
      isI("model") && (next.model ?? null) !== (inst.model ?? null);
    const providerChanged =
      isI("provider") && (next.provider ?? null) !== (inst.provider ?? null);
    // Rows 9–10: respondTo/parallelism are I-owned when an instance is present.
    // fieldOwner("respondTo", ctx) returns "instance" for instance-with-definition
    // and instance-only contexts, so these always emit to agentInput here.
    // Normalize null → "anyone" in the comparison to avoid phantom writes
    // when the DB stores null but the UI defaults to "anyone".
    const respondToChanged =
      isI("respondTo") &&
      (next.respondTo ?? "anyone") !== (inst.respondTo ?? "anyone");
    const allowlistChanged =
      isI("respondToAllowlist") &&
      next.respondTo === "allowlist" &&
      next.respondToAllowlist.join(",") !== inst.respondToAllowlist.join(",");
    const parallelismChanged =
      isI("parallelism") &&
      (next.parallelism ?? null) !== (inst.parallelism ?? null);

    // Harness pin (I-fields). The dialog resolves next.harnessCommand to the
    // effective command (selected-runtime command or manual entry) before it
    // reaches the model; here we settle inherit-vs-pin against the instance.
    let harnessCommandUpdate: string | undefined;
    if (isI("harnessCommand")) {
      if (next.harnessInherit) {
        // Inherit: clear an existing pin ("" sentinel), else no-op. An unlinked
        // agent has no override, so this stays undefined for it.
        harnessCommandUpdate =
          inst.agentCommandOverride != null ? "" : undefined;
      } else {
        const pinned = (next.harnessCommand ?? "").trim();
        // Emit when the command changed, OR when a linked agent is minting a
        // first pin over the inherited command (override still null). Unlinked
        // agents always have a null override, so the second clause must not fire
        // for them — otherwise every unmodified save writes a phantom pin.
        if (
          pinned !== (inst.agentCommand ?? "") ||
          (inst.personaId != null &&
            inst.agentCommandOverride == null &&
            pinned.length > 0)
        ) {
          harnessCommandUpdate = pinned;
        }
      }
    }
    const argsChanged =
      isI("harnessArgs") &&
      !next.harnessInherit &&
      (next.harnessArgs ?? []).join(",") !== (inst.agentArgs ?? []).join(",");
    const acpChanged =
      isI("acpCommand") &&
      next.acpCommand !== undefined &&
      next.acpCommand.trim() !== (inst.acpCommand ?? "");

    const iChanged =
      nameChanged ||
      systemPromptChanged ||
      modelChanged ||
      providerChanged ||
      instanceEnvChanged ||
      respondToChanged ||
      allowlistChanged ||
      parallelismChanged ||
      harnessCommandUpdate !== undefined ||
      argsChanged ||
      acpChanged;

    if (iChanged) {
      agentInput = { pubkey: inst.pubkey };
      if (nameChanged)
        agentInput.name = next.instanceName ?? next.displayName.trim();
      if (systemPromptChanged)
        agentInput.systemPrompt = next.systemPrompt.trim() || null;
      if (modelChanged) agentInput.model = next.model ?? null;
      if (providerChanged) agentInput.provider = next.provider ?? null;
      if (instanceEnvChanged) agentInput.envVars = next.instanceEnvVars ?? {};
      if (respondToChanged) agentInput.respondTo = next.respondTo ?? undefined;
      if (allowlistChanged)
        agentInput.respondToAllowlist = next.respondToAllowlist;
      if (parallelismChanged && next.parallelism != null)
        agentInput.parallelism = next.parallelism;
      if (harnessCommandUpdate !== undefined) {
        agentInput.agentCommand = harnessCommandUpdate;
        // harnessOverride marks a real pin; the "" inherit-clear leaves it unset.
        agentInput.harnessOverride =
          harnessCommandUpdate.length > 0 ? !next.harnessInherit : undefined;
      }
      if (argsChanged) agentInput.agentArgs = next.harnessArgs ?? [];
      if (acpChanged) agentInput.acpCommand = next.acpCommand?.trim();
    }
  }

  // L-field diff
  const policySets: AgentFormEmit["policySets"] = [];
  if (inst !== null) {
    if (
      next.autoRestartOnConfigChange !== undefined &&
      next.autoRestartOnConfigChange !== saved.autoRestartOnConfigChange
    ) {
      policySets.push({
        type: "autoRestart",
        pubkey: inst.pubkey,
        value: next.autoRestartOnConfigChange,
      });
    }
    if (
      next.startOnAppLaunch !== undefined &&
      next.startOnAppLaunch !== saved.startOnAppLaunch
    ) {
      policySets.push({
        type: "startOnAppLaunch",
        pubkey: inst.pubkey,
        value: next.startOnAppLaunch,
      });
    }
  }

  return { personaInput, agentInput, policySets };
}
