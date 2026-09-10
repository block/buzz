# Local presets and custom ACP: bounded contract

Source: immutable Buzz `051c3a270be9c73da9ab06700bcab7d5552fceaa`, read using
`git show` in `buzz-reference`, not that checkout's current tree. Grounding report:
`WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/HARNESS_PROVIDER_UX_GROUNDING.md`.

- `desktop/src-tauri/src/managed_agents/discovery/presets.rs:69–98,106–228`:
  ten definitions, empty definition env, absent model/provider mappings,
  NotApplicable auth and manual-only installation. Pi requires pi + pi-acp;
  Amp requires amp + amp-acp. Grok's actual argv is
  `["agent","--always-approve","stdio"]`, not a guessed `acp` argument.
  OpenClaw needs Gateway; child BUZZ_* variables do not automatically reach its
  tools. Beehive does not provision Gateway or declare that conversation supported.
- `custom_harnesses.rs:1–10,42–70,176–204`: local id/label/command/array/env/manual
  hints, no executable install scripts. Its comma rejection is specific to old
  transport. Beehive's owned broker launches the configured argv array directly;
  comma/empty/literal shell metacharacters are retained. Only fixed shim arguments
  use the installed buzz-acp comma transport, with their existing guard unchanged.
- `discovery.rs:587–648,937–1025`: executable discovery differs from auth and
  adapter capabilities. This slice intentionally uses bounded explicit local PATH
  resolution only, no login-shell/nvm/managed-directory probe, auth command or
  process launch. Missing path can still be stored as a diagnostic definition.
- `discovery/catalog.rs:12–49`: Goose startup GOOSE_PROVIDER/GOOSE_MODEL,
  GOOSE_MODE=auto, native config and no unstable model switching. Existing Beehive
  Goose model/profile owners are reused for the explicit `goose-native` custom
  contract. `crates/buzz-acp/src/pool.rs:287–320,1465+,1525–1541` gates native
  Goose profile support on a successful extension request; it is not inferred
  from protocol2. Existing ACP code sends the source-native
  `_goose/unstable/session/system-prompt/set` request. Custom normal additionally
  checks Goose initialize identity/protocol, fresh exact configOptions model,
  native profile bytes and successful acknowledgement before admitting prompts.

## Local workflow

`presets`, first-run setup choice 7, or existing `local-setup` action `presets`
shows availability/manual hints. Existing `local-setup` → `add-preset` → source
binding → preset ID → workspace → new binding ID → confirmation stores a new
immutable **diagnostic-only** binding. All ten are inspectable remotely with no
models and no Start eligibility. No preset is claimed authenticated or working.

Create a local owner-only JSON file (0600) with these exact fields:

```json
{
  "id": "my-adapter",
  "label": "My local adapter",
  "executable": "/absolute/canonical/adapter",
  "args": ["acp", "literal,comma"],
  "env": {"LOCAL_SETTING": "local value"},
  "installHint": "Configure locally as the dedicated host service user.",
  "installInstructionsUrl": "https://example.invalid/manual",
  "contract": "diagnostic"
}
```

Existing `local-setup` → `add-custom` reads this file, selects allowed workspace,
and saves only after confirmation. No install hint is executed; no env/argv is
printed or advertised. Missing/malformed input is rejected without mutation;
malformed JSON contents are withheld. Env is bounded; host identity, relay,
HOME/PATH, loader and existing Goose/model/auth context variables cannot be
replaced. Custom env is plaintext in the owner-only immutable setup manifest,
not a credential vault. Configure provider authentication locally under the
same dedicated service HOME; file/executable presence is never authentication.

For an adapter **actually implementing the Goose-native contract**, explicitly
use `"contract":"goose-native"`; first-run choice 6 accepts this through the
normal wizard. Existing installations can add it without changing identity or
common runtime/tool/relay/trust. Provider/model remain Goose-owned. Remote TUI
selects only advertised compatible model/workspace, then explicit Start/Restart.
The name is a local assertion of compatibility that must be proven by actual ACP
responses, not fabricated ACP capability metadata. Arbitrary/custom protocol2,
Codex systemPrompt, other native settings and resume/load are not generalized.

Replacement/retirement reuse existing local atomic startup-lock and immutable
fingerprint mechanisms. Original definitions remain inspectable, retired
references cannot execute, and bindings never automatically select or restart.

## Acceptance scope

Reused installed normal journey adds ONE custom fixture case, not ten vendor
claims: normal wizard → real host CLI → remote TUI → REAL installed buzz-acp
and Buzz CLI → four signed same-identity replies, two channels/later turn and
fresh-session Restart. Exact native model/profile, history/sibling preservation,
literal comma/metacharacter argv, private env, wrong model/unknown initialize/
rejected native profile preserving actual, and missing agent key/no spawn/no
recreation are required. Adapter is an owner-only deterministic TypeScript
Goose-native fixture, not installed Goose or a vendor. Preset/custom schema and
actual local wizard tests cover diagnostic registration, public privacy, rejected
remote injection/unsupported selection, private permissions and retirement.

Remaining: arbitrary adapter model/profile contracts, real provider login/inference,
all ten vendor conversation acceptance, other Buzz Agent provider modes, mesh,
compute deployment, production/current-kind40002 and full product parity.
Historical broker/tool failures remain separately unclassified. Prior scoped
Codex/Claude/NORMAL/CLI reviews are CLOSED, not new review of this delta.
