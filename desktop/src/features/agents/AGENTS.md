# Agent Configuration — Contributor Rules

Scope: `desktop/src/features/agents/` (config surfaces, shared config renderer,
and the agent config core). Read this before changing how harness / provider /
model / effort configuration is modeled, rendered, persisted, or applied.

Plan of record: `Buzz/Harness-Provider-Model.md` in Morgan's Obsidian vault
(PR sequence, decisions log). PRs: #2140 (rename), #2148 (flag reduction),
#2156 (honest model states), #2158 (Agent Config Core).

## The one rule

**Harness capability facts have exactly one source: the Rust runtime catalog.**
`KnownAcpRuntime` (`desktop/src-tauri/src/managed_agents/discovery/runtime_metadata.rs`)
declares each harness's model/provider/effort env keys and capabilities. Spawn
applies them; `AcpRuntimeCatalogEntry` exposes them over IPC; and
`lib/agentConfigCore.ts` projects them into field descriptors. The frontend
never maintains a rival copy of this table. Setup guidance follows the same
rule: `requires_external_cli` is derived from `KnownAcpRuntime` and projected
to the UI rather than inferred from a runtime ID in a component.

If you need a new capability fact (a new env key, a native option, a "supports
X" flag): add it to `KnownAcpRuntime` first, expose it on
`AcpRuntimeCatalogEntry`, then project it through the core. Do not shortcut
with a TypeScript lookup table or an id comparison in a component.

## Rules

1. **No hardcoded harness-ID checks in render code.** `runtime.id === "claude"`
   belongs in `deriveAgentConfigFieldModel` (once, with a named reason), never
   in a component. Components ask the field model what exists
   (`hasRenderableAgentConfigField`, `getRenderableEffortField`).
2. **Effort reads/writes go through the descriptor.** Use the effort
   descriptor's `currentPersistence` key — never a raw
   `BUZZ_AGENT_THINKING_EFFORT` literal in UI code. `currentPersistence` is
   where the value lives *today*; `targetApplication` is how the harness
   *should* receive it. They intentionally differ until PR 2.7 migrates
   Goose/Claude — do not "fix" one to match the other without doing the
   migration work.
3. **Field absence has a named reason, not a boolean.** Codex effort is
   `ownedByModelId`; Claude effort is `deferredUntilNativeOptionsAvailable`.
   New absences get new named reasons in `AgentConfigOmission` /
   `render` — never a `showX` prop.
4. **The clearing policy is the named types.** `onContextChange:
   "resetDependentValues"` (user changed harness/provider → dependent values
   reset everywhere) vs `onCatalogMismatch: "explainOnly" | "onboardingCleanup"`
   (an async catalog miss never silently erases saved state outside
   onboarding's named cleanup). Do not add mutation booleans like
   `clearInvalidModel`; extend the policy types.
5. **"Metadata unknown" ≠ "harness lacks the capability".** Passing
   `runtime: undefined` to the core means fields won't render. Surfaces must
   gate on the runtime catalog query settling (loading/error states) rather
   than letting fields silently vanish — see `AgentDefaultsEditor` /
   `DefaultConfigStep` for the pattern.
6. **One canonical behavior, disclosure presets for visibility.** Behavior
   flags were deliberately killed in #2148 (`CANONICAL_CONFIG_BEHAVIORS`).
   Surface differences are expressed via the `disclosure` preset, not new
   boolean props.  **Exception:** `onboarding-essential` hides happy-path
   helper copy (provider/model descriptions) but a non-null model-discovery
   status always bypasses the preset and renders the status line — enforced
   via `shouldShowModelStatusMessage()` (`AgentConfigFields.tsx`).
   Additionally, a successful discovery response that yields no usable options
   (`supportsSwitching:false` or empty model list) synthesizes a warning status
   via `synthesizeEmptyDiscoveryStatus()` and is intentionally **not cached**
   so that closing → reopening the dialog re-runs discovery after the user
   installs or signs into the CLI (`isCacheableDiscoveryResponse()`).
7. **Onboarding setup detects readiness; it does not select defaults.** The
   setup page derives visible and ready harnesses from the runtime catalog and
   only offers install or sign-in actions. The following defaults page is the
   sole onboarding surface that chooses and persists `preferred_runtime`, and
   its Finish gate consumes the shared renderer's `onValidityChange` signal —
   a harness selection alone does not complete onboarding when the harness
   requires provider/model/credential config (e.g. buzz-agent with no
   provider). Baked build env and runtime-file config satisfy the gate.
   `onboarding-agent-defaults.spec.ts` is the acceptance gate for anything
   touching this flow or the shared renderer.
8. **Omit the Model control only after a confirmed successful empty
   discovery on an optional-model harness.** When the field model marks model
   as `acpNative` (Claude Code / Codex), `shouldRenderModelControl` hides the
   picker while discovery is in flight and after IPC resolves with no usable
   options (`modelDiscoverySuccessfulEmpty` / `isSuccessfulEmptyDiscovery`).
   A thrown or unavailable discovery keeps the control so #2246 failure UI can
   render, and must not heal/clear persisted model or effort. Full disclosure
   still shows the control when Custom model is available. Required-model
   harnesses always keep the field. Gate: `defaults hides model when optional
   harness has empty discovery` (and the failed-discovery counterpart) in
   `onboarding-agent-defaults.spec.ts`.
9. **The defaults modal is progressively disclosed.** An unset global config
   starts on the Buzz Agent-first deployment fallback and carries that visible
   harness into the next saved edit. The `progressive-defaults` disclosure
   preset therefore begins at Provider for Buzz Agent, then reveals Model,
   Effort, and Advanced only after a provider is configured. Harnesses whose
   runtime metadata has no provider field skip that gate. Reveals animate their
   height through Motion and become immediate when reduced motion is requested.
   Once the Advanced toggle is visible, its expanded state is exclusively
   user-controlled: provider, harness, and required-env changes must never
   open it automatically in defaults, create, or edit flows. In Create mode,
   the defaults summary follows preferred-harness changes saved while the
   dialog is open, and its configured state includes required credentials as
   well as provider/model values. If no available harness can resolve, Create
   starts in Customize and lets unavailable catalog entries be selected only
   to expose their setup guidance; submission remains blocked.
   Advanced-only required credentials mark the collapsed Advanced toggle
   without opening it in Global Defaults and Edit, and block incomplete saves.
   Runtime-file credentials satisfy Global Defaults just as they do Create and
   Edit. In Edit,
   selecting Custom command keeps its required command field beside the harness
   picker rather than hiding it in Advanced.
10. **Catalog visibility is community-scoped relay state, never a global
    definition field.** `AgentDefinition.shared` is only the active
    relay+owner projection returned to the UI. Durable heads and pending
    publications live in the scoped retention database, and explicit share
    toggles await relay acceptance before the UI claims that an agent was
    published or removed. A queued update must stay visibly queued, and the
    catalog itself must render only relay-confirmed publications — never an
    optimistic local persona.
11. **A remote create's models come from the host, never from this computer.**
    When "Where to run" targets a backend provider and a harness is picked from
    the host's catalog, `WhereToRunSection` calls `probeProviderModels`
    (`probe_provider_models`, guarded by `resolve_discovered_provider`) and
    parks the result in `WhereToRunDraft.remoteModelProbe`.
    `remoteModelDiscoveryView()` projects it into the exact shape
    `usePersonaModelDiscovery` returns (`RemoteModelDiscoveryView` extends
    `ModelDiscoveryView` so the compiler holds them together), and
    `useRemoteAwareModelDiscovery` substitutes it for the local one — the two
    are never merged (different machines; the union would offer models the
    chosen harness cannot run) and local discovery is suppressed entirely while
    the host owns the control. Both decisions live in pure helpers
    (`resolveModelDiscovery`, `shouldSuppressLocalDiscovery`) precisely so they
    are testable without hook infrastructure; keep them that way. Changing the
    picked harness resets the model for the same reason changing the local
    runtime does. Do not add a remote-specific rendering path in
    `PersonaModelField`: keep the substitution at the discovery seam.
12. **Every host round-trip carries a request id.** `WhereToRunSection` opens
    real SSH connections (`discoverProviderHarnesses`, `probeProviderModels`).
    Both claim `hostRequestRef` at their start and re-check it after every
    await; anything that moves the draft off the host they were made for
    (provider switch, config edit, re-pick, re-check) bumps it. Without that
    re-check on the CATALOG read, a config edit made mid-flight lets the old
    host's catalog reinstall itself and then fires a credential-carrying model
    probe at the NEW host under the OLD host's harness command. A new
    host-touching call gets the same treatment — do not add one that only
    guards its own continuation.
13. **"Where does this agent run?" is the create flow's first question, and
    it scopes every field below it.** `createRunSection` renders above name,
    persona, and harness in `AgentDefinitionDialog` because the harness comes
    from the chosen machine's catalog and the models come from that harness —
    asking last would mean answering the dependent questions against the wrong
    computer and silently re-scoping them. Consequences that must move
    together: the local `AgentHarnessField` is hidden for a remote create (its
    "not installed, visit Settings" guidance describes the wrong machine), and
    the defaults summary names the host's pick via `createRemoteHarnessLabel`
    rather than the locally seeded `runtime`. The credential gate is asked of
    the host's pin too (`createGateHarnessId`): the deploy keys the agent's env
    off the REMOTE command, so the local id would demand `BUZZ_AGENT_*` for a
    remote Goose, or nothing at all when this machine defaults to Claude. It is
    NOT relaxed, though — a deploy writes that env to the host verbatim, so a
    missing key is just as fatal there. The one layer that is suppressed is the
    runtime *file* config: it reads this machine's `~/.config`, and letting a
    local `goose/config.yaml` satisfy a remote requirement trades a loud
    create-time block for a silent deploy-time failure.
    Edit mode is untouched — `createRunSection` is create-only.
14. **A provider decorates a config property with `oneOf`; the desktop renders
    it generically.** `providerConfigChoices` reads
    `oneOf: [{ const, title }]` off any config-schema property and
    `ProviderConfigFields` renders a select over it plus an "Other…" escape
    hatch; no `oneOf`, and the field is the plain Input it always was. The
    desktop knows nothing about tailnets — the SSH provider fills the
    decoration from the local Tailscale peer list and supplies the display
    strings. Keep it that way: a Tailscale-shaped branch in the renderer makes
    the next provider's suggestions unrenderable. A value that is not in the
    list (carried over, or a peer that has left the tailnet) stays in free text
    rather than reading as "nothing selected" — that is
    `usesProviderConfigFreeText`, and it is pure so it can be tested.
15. **Remote liveness is relay presence; `"deployed"` is a control-plane fact,
    not a liveness one.** `build_managed_agent_summary` reports `"deployed"`
    whenever `backend_agent_id` is set, and that id is written exactly once —
    on a successful deploy (`commands/agents.rs`) — with no clearer anywhere,
    because the provider protocol has no undeploy. So `isManagedAgentActive`
    answering true for a remote record means "this desktop deployed it", and
    it keeps meaning that after the remote process dies. Every surface this
    stack touches therefore paints liveness through
    `managedAgentPresenceStatus`, which keeps the control plane authoritative
    for a local record (this machine's own process table, and relays need not
    retain ephemeral kind:20001 presence) and defers to relay presence for a
    provider-backed one — the same channel `deleteManagedAgentWithRules`
    already trusts before warning about an orphaned deployment. Two upstream
    surfaces, `MembersSidebarMemberCard` and `AgentStatusBadge`, still paint
    from the stale flag; they are follow-up candidates, not a licence to add a
    third. Do not add SSH polling, a second status channel, or a `local_setup`
    read for a non-local record: `local_setup` asks whether *this* machine
    could run the agent and its UI copy ("Needs setup on this device") names
    the wrong machine. See the doc comment on `status_for_with`
    (`runtime_commands.rs`).
16. **The pinned harness must be an `available` catalog entry.**
    `selectedRemoteHarness` filters on `available`, so an id that a re-check
    turned unavailable stops being the pin rather than deploying a command the
    host says is not installed. Likewise the create-time args of a provider
    record are pinned verbatim (`create_time_agent_args`): normalizing them
    would compare a REMOTE command against LOCAL runtime identity, and a host
    binary sharing a basename with a local runtime would have its explicit
    args silently rewritten.
17. **An `exclusive` catalog entry may back at most one agent.** The provider
    marks entries that name a persistent IDENTITY on the host (its own memory,
    sessions, credentials) rather than an ephemeral runner — today only the
    per-Hermes-profile entries. Deploying `claude` N times to one host is the
    point; two agents on one profile are two puppeteers on one body.
    `isExclusiveRemoteHarnessAdded` decides "already taken" generically: same
    provider, same provider config, same command+args as an existing record's
    `agentCommand`/`agentArgs` (the RESOLVED pin — `agentCommandOverride` is
    null for a pin equal to what the definition inherits). The picker renders a
    taken entry disabled with an "(added)" suffix — the existing annotated-and-
    disabled option vocabulary, not a new badge — auto-pick skips it, and
    `WhereToRunSection` clears a pick the agent list turns stale so a
    background refresh cannot leave an armed submit behind. Nothing in the
    desktop knows what Hermes or a profile is; do not teach it. Config equality
    is exact after trimming/dropping-blanks/sorting, so a host reached by two
    names under-matches (the guard does not fire) rather than falsely blocking
    a create; the real fix is a host-identity answer from the provider.
18. **A location label names the PROVIDER, never the host.** `agentRunsOnLabel`
    is the one owner of "where does this agent run" for every surface that
    lists agents (`AgentIdentityCard`, `MembersSidebarMemberCard`, the user
    profile panel's "Runs on" row). It answers `null` for a local agent — "on
    this computer" is the assumption a reader already holds, so painting it
    costs a metadata line to say nothing — and for a provider-backed one it
    returns `backendProviderLabel(backend.id)`. It does NOT read
    `backend.config`. That is the same refusal rule 17 makes: a blessed
    `ssh_host` key means the desktop grows a host vocabulary per provider, and
    a provider id is constrained to `^[a-z0-9][a-z0-9_-]*$` by
    `provider_id_is_valid` while a host is not, so naming the provider is also
    the only answer with a bounded shape. Ids stand in for probed names on
    these surfaces on purpose (`backendProviderLabel`'s own rule): a card list
    must not spawn one subprocess per provider to decorate a string. A new
    agent-listing surface calls the same helper and extends an existing
    metadata slot rather than adding a badge.
19. **Settings → Remote servers reports what is installed; the create flow owns
    deployment.** `RemoteServersCard` is read-only by design and has no host
    list. A provider is a binary on `PATH`, so "adding" one is an install, not
    a form; and the host is a per-agent decision the create dialog pins onto
    the agent record verbatim at create time. CRUD here would either edit saved
    configs that deployed agents deliberately do not re-read — which reads as a
    bug — or duplicate the create flow's ownership of the host. Three
    consequences that must not drift: this gallery is the ONLY surface that
    pays for an `info` probe per discovered binary
    (`useBackendProviderProbesQuery`; the create dialog and the onboarding
    notice render ids, per rule 18); `"ready"` means "this binary answers the
    provider protocol", never "the server is reachable", because `info` opens
    no connection; and every settled probe must land in the probe map, since an
    absent entry is indistinguishable from one still in flight and a dropped
    result is therefore a row that spins forever (`remoteServerProbes`). The
    no-provider sentence is `NO_BACKEND_PROVIDER_HINT`, stated once and
    rendered by all three surfaces — a user meets it in up to three places, and
    three spellings of one fact read as three different facts.
20. **A provider record answers from itself, never from the local catalog.**
    A provider-backed record's `agentCommand`/`agentArgs` name a binary on the
    HOST, which this computer's runtime catalog has never seen — so every local
    lookup either misses (`hermes …` → generic icon, raw command as a name) or
    hits by pure name collision (`claude-agent-acp` happens to be a local
    builtin's command, the only reason a Claude card ever looked right).
    Neither is knowledge. `providerRecordHarness` (`lib/pinnedHarness.ts`) is
    the single owner of "may this surface read the record instead of the local
    catalog?", and it answers `null` for a local agent — whose catalog entry
    genuinely describes it, and whose rendering must not change. Three
    consequences:
    - **Derivation is generic.** A command basename (both separators, since
      the path is the host's), the base-id fallback `resolveHarnessLogo`
      already uses for variant ids, and a `--profile <name>` flag — a
      widespread CLI convention. Nothing here knows what Hermes or SSH is; do
      not teach it. The profile is part of the identity: two profiles of one
      harness are two agents with their own memory and credentials, and must
      not read as one name (same fact rule 17 guards on).
    - **Normalization has one owner.** `normalizePinnedCommand` trims the
      command and drops blank args, matching `create_time_agent_args`, and
      BOTH the displayed `pin.command` and `exclusiveRemoteHarness`'s equality
      check go through it. A rule that decides "same agent?" and a rule that
      decides what the user reads must be one rule.
    - **Avatar precedence is by authorship** (`lib/agentAvatarUrl.ts`): what a
      human chose, then what the agent published about itself, then the record's
      create-time stamp, then the pin's bundled harness mark. That last step
      exists because a local create stamps this computer's runtime avatar onto
      the record and a remote one has nothing to stamp — the host's catalog
      entry deliberately carries no avatar url, since rendering a host-supplied
      image is a tracking-pixel and spoofing vector (`RuntimeIcon`'s bundled
      maps are the only permitted route). Do not add a host-supplied image
      path, and do not move the derivation to deploy time: the fleet already
      exists, and records minted before this carry an empty avatar forever.
    - **Editing routes by the same question.** A provider record is edited in
      the INSTANCE dialog, never the definition one. `AgentDefinition` has no
      slot for `backend` or `agent_command` (`to_definition_view` drops both by
      design), so the definition dialog can only show a remote record's harness
      as blank — and then fill that blank from this computer's catalog, which
      arms a provider requirement and an API-key demand for a machine the agent
      never runs on. Both doors ask `providerRecordHarness`:
      `profileEditAgentTarget` for the profile panel's Edit action and
      `agentManagementUpdateTarget` for the owner-reviewed `!model` draft.
      `createRuntimeSeedAction`'s `editsProviderRecord` is the backstop; the
      blank runtime is deliberate, not an absence to fill. Do NOT widen
      `AgentDefinition` to carry these fields instead: `into_agent_record`
      silently reverts non-default `backend`/`agent_command`, turning a display
      bug into data loss.

    The pin is editable only where it was made — at create/deploy.
    `personaManagedAgentUpdate` must NOT write a locally-discovered runtime's
    command/args over a provider record (its `runtimeChanged` gate is
    `backend.type === "local"`): the catalog entry it would write is a path on
    THIS machine, so a working `hermes --profile marshall acp` would be
    replaced by a binary that does not exist on the host, from a dialog that
    never said it would touch the harness.
21. **A host failure the user can fix carries a typed recovery, and the URL is
    validated on entry.** A provider may answer a failed op with
    `recovery: {action: "open_url", url}` (see `docs/remote-agents.md`); today
    the only case is a tailnet ACL demanding browser re-auth. It reaches the UI
    as `ProviderFailure {message, recovery}` — there is deliberately no
    `From<ProviderFailure> for String`, which is the type-level guard against a
    caller flattening the recovery away. The frontend reads it off
    `TauriInvokeError.payload` with `providerRecoveryOf`; `hostFailureOf` is the
    one place `WhereToRunSection` converts a rejection, so a new host call picks
    the recovery up for free. Two rules for anything added here:
    - **Validate where the value enters, not where it is used.**
      `ProviderRecovery::from_response` checks the Tailscale prefix AND the
      token charset before constructing the value, so an unvalidated URL never
      exists in desktop memory and no later reader can become a second,
      unguarded way to open it. A provider is a discovered subprocess, not a
      trusted peer — the same footing that makes `invoke_provider` re-redact
      secrets on the way in. Do not relax this to a bare `starts_with`, and do
      not let a provider name the destination.
    - **The message always stands alone.** The recovery only ever ADDS a button;
      the copy names the problem without it, so an older provider (or a dropped
      recovery) degrades to an ordinary failure rather than an empty one.
      Nothing auto-retries: the desktop cannot observe a browser it does not
      own, so "Check the host again" stays the retry.
    - **Dropping it is allowed, silently doing so is not.** `start_managed_agent`
      and `create_managed_agent`'s `spawn_error` render into a toast and a
      reported field, neither of which has room for an action, so each converts
      to the message at one named site. Do not turn that into a blanket `impl
      From<ProviderFailure> for String` — the explicitness is the point. Give
      the surface an action before widening it.

## The tests that enforce this

- `lib/agentConfigCore.test.mjs` — field model per harness × scope, clearing
  policy. Update when the capability model changes.
- `ui/agentConfigFieldsContract.test.mjs` — canonical behaviors + disclosure
  presets + `shouldShowModelStatusMessage` status-bypass +
  `shouldRenderModelControl` (successful-empty omit vs failure keep). If this
  fails, you probably reintroduced a per-surface flag or conflated empty with
  failed discovery.
- `ui/whereToRunIntent.test.mjs` — the remote create's submit gate, the
  available-only harness pin, `runTargetOptions` / `rememberProbedProviderName`
  / `remoteHarnessSummaryLabel` (the first question, the label cache that keeps
  its entries from renaming themselves as the selection moves, and the summary
  that follows it), and `remoteModelDiscoveryView`
  (idle/loading/failed/loaded/empty-catalog). Covers the PROJECTION of the
  host's probe, not the substitution that consumes it. Also `remoteHarnessOptions`
  / `autoPickRemoteHarness`: rule 17's disabled "(added)" row and the auto-pick
  that must never arm a create the picker itself refuses.
- `lib/exclusiveRemoteHarness.test.mjs` — rule 17's matcher. Same host + same
  pinned identity is taken; a different host, user, provider, profile or
  command is not; a local agent never occupies a host identity; a
  non-exclusive entry is never taken however many agents run it; and an absent
  flag is exactly today's behavior.
- `lib/pinnedHarness.test.mjs` — rule 20's derivation, plus the label table's
  agreement with the Rust catalogs in BOTH directions (it reads
  `discovery.rs` as text, the same trick `presetLogos.test.mjs` uses; a TS-only
  key must be listed in `NOT_IN_RUST_CATALOG` with the surface that carries the
  command). The cases are the two fleet shapes that broke: a
  `hermes --profile marshall acp` pin that rendered a generic icon and lost its
  profile, and a `claude-agent-acp` pin that only looked right by collision.
  Also that an unknown host binary shows itself rather than a local guess — if
  a remote card starts naming a harness the host does not run, this is the test
  that should have caught it.
- `../profile/ui/profileEditAgentTarget.test.mjs` and
  `agentManagementUpdateTarget.test.mjs` — rule 20's editing route, once per
  door. Both pin the same pair: a provider-backed record selects the instance
  editor even though it has a personaId (every provider create does), and a
  local persona-backed one still selects the definition editor. If a remote
  agent's Edit starts demanding an API key again, these are the tests that
  should have caught it. `tests/e2e/edit-agent-provider-routing.spec.ts` covers
  the same route end to end.
- `ui/createRuntimeGate.test.mjs` — the harness auto-seed, including the
  edit-mode `editsProviderRecord` guard. Its create-mode sibling asks
  `runsRemotely`, which is false in edit mode, so a remote record's blank
  runtime was being filled with the local default.
- `lib/agentAvatarUrl.test.mjs` — rule 20's precedence chain. A human's choice
  beats the agent's own published avatar beats the record's stamp beats the
  harness mark; a LOCAL record never reaches the harness step, so its rendering
  is unchanged.
- `features/profile/ui/profileRuntimeLabel.test.mjs` — that the profile
  surfaces name a record from its pin and a foreign surface's free-form command
  (a relay agent's declared `agentType`) through the SAME owner. A second label
  table here is what let `codex-acp` be "Codex" in one place and a harness
  learned in Rust be a raw command in the other.
- `lib/agentLocationLabel.test.mjs` — rule 18. A local, undefined or null
  backend is unlabelled; a provider-backed one is named by its id; and a
  config carrying an `ssh_host` still yields the provider's name, so the host
  cannot leak into a card by accident.
- `lib/backendProviderLabel.test.mjs` — the id-vs-probed-name fallback and the
  sorted label list. A blank probed name reads as no name (the id is the better
  label), and sorting is what stops a `PATH`-order change from reshuffling a
  hint line under the user.
- `features/onboarding/ui/remoteRunNotice.test.mjs` — the setup step's three
  states. Pending stays pending over a cached list, because rendering the
  install hint and then contradicting it a frame later is the failure this
  projection exists to prevent.
- `features/settings/ui/remoteServerGalleryLogic.test.mjs` — rule 19.
  `remoteServerProbes` lands every settled query somewhere (a response-less
  success is a failed row, not a permanent spinner), and `remoteServerEntries`
  covers ready/probing/unavailable, blank-vs-absent metadata, and the
  ready-first sort. The card itself is a thin render over these two; if a row
  spins forever or the gallery reshuffles, this is the test that should have
  caught it.
- `shared/api/tauri.test.mjs` — `fromRawRemoteHarness`: the wire boundary for
  `exclusive`. An asserted flag is carried; absent stays absent (the desktop
  must not claim something the provider never said).
- `ui/providerConfigFields.test.mjs` — `providerConfigChoices` (a malformed
  `oneOf` entry costs one row, not the list), `usesProviderConfigFreeText`
  (an unlisted value stays editable), and `providerConfigSelection` (picking
  "Other…" keeps the value; returning to a suggestion drops the override, so a
  round trip leaves no text box stuck open). Rule 13 lives or dies here.
- `lib/managedAgentControlActions.test.mjs` — `managedAgentPresenceStatus`.
  Rule 14: a deployed remote agent with nothing on the relay reads offline, a
  running local one stays online through a silent relay. If a stopped remote
  agent's card goes green again, this is the test that should have caught it.
- `ui/useRemoteAwareModelDiscovery.test.mjs` — `resolveModelDiscovery` and
  `shouldSuppressLocalDiscovery`. If the Model control starts offering this
  computer's models to a remote harness, or runs local discovery IPC
  underneath a live remote catalog, these are the tests that should have
  caught it. The staleness guard in `WhereToRunSection` itself is NOT covered
  (no hook/DOM test infrastructure in this workspace — `pnpm test` is bare
  `node --test`); it is held by rule 12 and review, so read it carefully.
- `ui/usePersonaModelDiscovery.test.mjs` — `synthesizeEmptyDiscoveryStatus`,
  `isCacheableDiscoveryResponse`, `deriveModelDiscoveryPending`,
  `isSuccessfulEmptyDiscovery`. If the "reopen to retry" copy becomes inert
  again, these tests will catch it.
- `desktop/tests/e2e/onboarding-agent-defaults.spec.ts` — onboarding behavior
  acceptance coverage for readiness, failure states, defaults, navigation,
  successful-empty vs failed optional-model discovery, and persistence races.
- Rust: `runtime_metadata_env_vars` tests pin spawn-time key application.
- Rust: persona sharing/retention tests pin relay+owner scoping, durable
  enqueue errors, relay rejection/unavailability, and accepted publication.
- Rust: `discovery/tests/create_time_args.rs` — the create-time args authority.
  Every case asserts the local AND provider backend over the same input,
  because a remote binary sharing a basename with a local runtime is
  normalized without complaint otherwise.

## Keep this file true

**If you change how agent configuration is modeled, rendered, persisted,
applied, or cleared — update this file in the same PR.** A rule that no longer
matches the code is worse than no rule; a new pattern that isn't written down
here will be broken by the next agent that never learns it existed. Reviewers:
treat a config-behavior diff without a matching AGENTS.md diff (or an explicit
"no rules changed" note) as incomplete.
