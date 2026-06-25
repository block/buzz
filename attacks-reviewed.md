# Buzz Attack Review

Reviewed against the current checkout on 2026-06-25.

This version only treats an attack as a finding when it crosses an intended Buzz boundary. I am not counting "an admitted sender drives a dev-MCP agent to use its local machine authority" as a separate finding. That behavior is explicit in the current model: managed agents plus `buzz-dev-mcp` are local code execution, not a repo sandbox. See `report.md:118-132`, `report.md:629-660`, and `report.md:998-1002`.

## High

### A-06. Prompt injection through attacker-controlled repository content

**Verdict:** Yes, conditional.

This is a real boundary bypass because the attacker does not need to pass the inbound author gate. `buzz-acp` only filters Buzz event authors before a turn starts; it does not distinguish trusted instructions from text later read from a repo, PR, issue, or tool output. If an owner asks the agent to inspect attacker-controlled content, the content can steer the same high-authority local tool surface. See `crates/buzz-acp/src/lib.rs:1778-1810` and `crates/buzz-dev-mcp/src/shell.rs:104-163`.

### A-09. Long-lived delegation reused after the agent appears retired

**Verdict:** Yes.

Deleting a managed agent stops the local process and removes the local record, but it does not revoke the key or delegation at the relay. Archiving is explicitly only a UI visibility hint, and empty NIP-OA conditions are valid, so a copied agent key and auth tag remain usable after the owner believes the agent is gone. See `desktop/src-tauri/src/commands/agents.rs:1045-1097`, `crates/buzz-db/src/archived_identities.rs:1-6`, and `crates/buzz-sdk/src/nip_oa.rs:33-40,139-169,172-239`.

### R-05. Channel member creates a workflow that exfiltrates future messages

**Verdict:** Yes.

Workflow definition creation checks only channel membership, not an admin or owner role. A normal channel member can create a `message_posted` workflow that templates message content into a request to a public webhook. The SSRF controls block internal destinations and redirects, but they do not block deliberate exfiltration to an attacker-controlled public endpoint. See `crates/buzz-relay/src/handlers/command_executor.rs:558-652` and `crates/buzz-workflow/src/executor.rs:416-475,641-670,757-870`.

## Medium

### A-07. Lateral movement from a low-trust sibling agent to a high-trust agent

**Verdict:** Yes.

`owner-only` is not literally owner-only. The harness deliberately accepts same-owner sibling agents after verifying their NIP-OA profile tag. That creates one trust domain across all agents owned by the same person, even if those agents have very different expected duties or local authority. If that is intended, it should be explicit in the product model and UI; if not, it is an authorization gap. See `crates/buzz-acp/src/lib.rs:159-209,214-284,1778-1810`.

### R-02. Dev-mode `X-Pubkey` impersonation on read APIs

**Verdict:** Yes, conditional.

When `BUZZ_REQUIRE_AUTH_TOKEN=false`, `/query` and `/count` accept `X-Pubkey` as the caller identity and compute access from that supplied pubkey. If that mode is exposed outside a trusted dev machine, an attacker can read as another user. Event writes as the victim are still blocked by event signature verification and pubkey matching. See `crates/buzz-relay/src/api/bridge.rs:23-70,167-233,241-296,501-548` and `crates/buzz-relay/src/handlers/ingest.rs:1106-1151`.

### R-03. NIP-98 body substitution when the payload hash is omitted

**Verdict:** Yes, conditional.

The generic NIP-98 verifier only checks the body hash when a `payload` tag exists. A captured auth header without that tag can be raced with a different `/query` or `/count` body before the replay cache records the event ID. Current first-party desktop, ACP, and CLI clients include payload hashes, so they do not normally create this request shape themselves. See `crates/buzz-auth/src/nip98.rs:34-130,267-273`, `crates/buzz-relay/src/api/bridge.rs:72-95`, `desktop/src-tauri/src/relay.rs:104-130`, `crates/buzz-acp/src/relay.rs:206-247`, and `crates/buzz-cli/src/client.rs:85-115`.

### R-04. Git NIP-98 credential replay inside the freshness window

**Verdict:** Yes, conditional.

The Git transport intentionally verifies only the repo-root URL, skips method binding, does not hash the pack body, and does not deduplicate event IDs. A captured token is replayable against the same repo within the verifier's +/-60-second timestamp tolerance, subject to repository authorization and pre-receive rules. See `crates/buzz-relay/src/api/git/transport.rs:111-175` and `crates/buzz-auth/src/nip98.rs:77-85`.

### R-06. Workflow secret leakage enables unauthorized triggering

**Verdict:** Yes, conditional.

Webhook workflows are authenticated only by a generated bearer secret stored in the workflow definition, and the endpoint accepts that secret from either `X-Webhook-Secret` or `?secret=`. If the secret leaks, an attacker can submit arbitrary webhook fields and cause workflow runs without user auth. The query-param fallback increases the chance of leakage through logs and copied URLs. See `crates/buzz-relay/src/webhook_secret.rs:22-89` and `crates/buzz-relay/src/api/bridge.rs:831-973`.

## Low

### S-03. Shared relay used as if it provided hard tenant isolation

**Verdict:** Yes, conditional.

This is an architecture warning rather than a direct exploit. The event/query model is channel-scoped and relay-global; I did not find a first-class tenant identifier in the core event query shape. A missed scoping check in a shared deployment can therefore become cross-organization. Operators should not treat workspaces or channels as hard tenant isolation. See `crates/buzz-db/src/event.rs:19-69`, `crates/buzz-db/src/channel.rs:1-5`, and `crates/buzz-relay/src/api/mod.rs:31-158`.

## Not Standalone Findings

The following attacks are real consequences of the current trust model, but I would not count them as findings unless Buzz intends a stronger property than the code currently claims.

| IDs | Why this is not a standalone finding |
| --- | --- |
| A-01, A-02, A-03, A-04, A-05, A-08, A-10 | Once an agent is intentionally reachable and has dev MCP, local shell, filesystem, ambient Git, process environment, and local logs are all inside its authority. `buzz-dev-mcp` is explicitly not a sandbox. |
| A-11 | Imported prompts can be malicious, but I did not find a current persona/team import path that silently changes `respond_to`; the stronger variant is blocked by structured `respond_to` fields and reserved env-key filtering. |
| C-01, C-02, C-04 | These require frontend or host compromise. Once the desktop renderer or local host is compromised, exporting or swapping keys is expected under the current client trust boundary. |
| C-03 | Workspace identity correlation and phishing are social/privacy risks, but workspace reuse of the active identity is intentional and does not itself expose the private key. |
| C-05 | Pairing interception is blocked when the SAS is actually verified. The remaining case depends on the user skipping or misreading the security check. |
| R-01 | Open relay plus open channel is the intended behavior of that deployment mode. It is only a problem if operators mistake it for a closed relay. |
| R-07 | `any` approvals are explicitly supported configuration, not a hidden bypass. The risk is review/UI clarity, not a code-path violation. |
| R-08 | Current relay storage, query, and fanout paths keep global events from becoming channel-scoped solely because they carry an `h` tag. This remains a future consumer footgun, not a current exploit. |
| R-09 | This is a future regression class. Current unknown kinds are rejected and current sensitive kinds have explicit read/fanout/search handling. |
| R-10 | Media reads are intentionally bearer-by-hash. The security property is URL secrecy, not channel re-authorization on every download. |
| R-11 | The relay signing key is an explicit trust anchor. Key theft is serious, but it is not a separate boundary failure unless the system claims resilience after relay-key compromise. |
| S-01, S-02, S-04, S-05, S-06, S-07, S-08 | These are supporting-service, host, relay-operator, or operational-secret assumptions. The current architecture explicitly treats those systems and artifacts as trusted boundaries. |
