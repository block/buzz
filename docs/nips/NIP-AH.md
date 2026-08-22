# NIP-AH: Agent Handoff Records

## Summary

Agent handoffs are durable, sender-authored `kind:44201` events containing a
curated task history encrypted with NIP-44 v2 to one receiving Agent.

A handoff transfers a point-in-time work snapshot. It does not grant access to
the sender's future Activity, local Codex JSONL files, hidden reasoning, or
other conversations.

## Event

```json
{
  "kind": 44201,
  "pubkey": "<sending-agent>",
  "content": "<nip44-v2 ciphertext>",
  "tags": [
    ["p", "<receiving-agent>"],
    ["handoff", "1"]
  ]
}
```

The decrypted JSON payload is:

```json
{
  "version": 1,
  "title": "Continue attachment previews",
  "summary": "Core implementation is complete",
  "history": "## Completed\n..."
}
```

`summary` is optional. `history` is Markdown and should contain requests,
outcomes, decisions, relevant files or links, verification, unresolved work,
and next actions. Producers must exclude credentials and hidden chain-of-thought.

## Relay behavior

- Require exactly one `p` tag and one supported `handoff` version tag.
- Store the event globally with no channel scope.
- Return it only when the authenticated reader matches the `p` tag.
- Apply the recipient check to kindless event-id lookups and COUNT requests.
- Exclude ciphertext from full-text search indexing.

## CLI

```bash
buzz agents handoff send --to <pubkey> --title <title> --history-file handoff.md
buzz agents handoff list
buzz agents handoff show <event-id>
```
