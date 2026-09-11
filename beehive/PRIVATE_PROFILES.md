# Private instruction profiles

This slice is instruction editing/publication/selection, **not agent-signed public
kind0 metadata**. It does not change public names, pictures or about fields, create
agent identities/grants, or start an agent as a side effect of publication.

## Owner workflow

Use the same owner state directory for both commands:

```sh
beehive drafts <state-directory>                     # offline, no signer/network
beehive tui discover <relay> <state-directory>       # existing owner signer
# With a retained catalog: its parent directory is the owner state directory.
beehive tui <catalog-file>
```

The offline editor supports `drafts`, `profile-new`, `profile-resume <draft-id>`,
`profile-discard <draft-id>` and `quit`. Draft IDs are printed after saving.
**Save boundary: Enter completing a valid instructions line**, after name input;
not every keystroke. Instructions are currently one input line, at most 2048 UTF-8
bytes (the codec also supports newlines). An interrupted first input is unsaved;
an interrupted edit leaves the previously saved text intact. Do not put passwords
or keys in instructions. This command never loads the owner signer or a host/model.

In the connected TUI:

1. `profile-new`, or `profiles` then `profile-edit <version-number>`, saves a local
   draft before asking whether to publish. Answering `no` or closing retains it.
2. `drafts` lists retained text and any matching submitted operation status.
   `profile-resume <draft-id>` offers `edit/publish/cancel`; publish still needs
   explicit `yes` confirmation. Editing a saved draft edits that draft's candidate;
   to branch from a **published** revision use `profile-edit`, preserving its parent.
3. Publication is owner-signed, encrypted to the owner, independently of host
   placement/reachability. `operations` distinguishes pending/unknown from observed
   immutable publication. `reconcile` reloads history without lifting policy blocks;
   explicitly retry only after resolving the real transport refusal. Drafts remain
   saved even after publication. `profile-discard` deletes only local editable text;
   it does not retract submitted operations, delete revisions or change agents.
4. `profiles` lists complete immutable revisions; missing/conflicting parents stay
   INCOMPLETE and cannot be selected. Concurrent children are explicit branches,
   never a timestamp-selected latest revision. Fresh sessions reload relay history.
5. `hosts`, `select <row>`, `apply <profile-number>` saves **selected-next** using
   the current agent/host revision precondition. No restart. `apply default` clears
   the selected override. `show` separates selected-next from the actual run.
6. Explicit `start` (stopped) or `restart` applies the exact selected snapshot after
   the existing authorization, binding and prerequisite checks. A running agent
   keeps its old snapshot until Restart; existing run history is not rewritten.

## Persistence and authority

- `<state-directory>/profile-drafts/drafts.json`: local editable text, 0600, inside
  a 0700 directory; no secret keys. At most 100 drafts. Atomic file/directory-fsynced
  saves and a short exclusive `edit.lock` protect against concurrent overwrites.
  Stale editors fail rather than erase newer text. A process killed inside a save
  can leave the lock; after confirming no editor owns it, local administration may
  remove that empty lock directory. There is no automated lock-owner guessing.
- `<state-directory>/management-intents/<relay-and-owner-digest>/`: existing
  encrypted durable operation journal, prepared before network effects. Discarding
  draft text cannot cancel an already submitted publication.
- Relay: standard NIP42 connection plus NIP44/NIP59 encrypted kind1059 envelopes.
  Library publications are owner-to-self with a signed `{relay, profile}` body in
  the existing Beehive message namespace. Owner signer, recipient, relay, routing,
  schema and content address are checked before rebuilding the catalog. Private
  instructions never enter kind0 or plaintext public events. Reload depends on
  the relay retaining and returning history; no new pagination/retention service.
- Host: only the authorized selected snapshot travels in the existing private
  owner-signed Save. The host validates strict `selection/profile` codecs (including
  recomputing the content hash), pinned owner/host/agent assignment and CAS, then
  persists selected-next in its per-agent journal. Parent links organize the owner's
  library; they are not grants. The host does **not** independently demand a library
  publication receipt or acquire unrelated ancestors. An authorized owner-signed
  exact snapshot is the instruction authority at this consumer; an arbitrary caller
  label or unverified revision map is not. Legacy dev transport keeps its catalog
  checks. Start/Restart consumes the durable selection and records actual applied
  revision/hash, exact instructions and run history through existing launch paths.

No membership discovery/precheck/invite/proof, host approval ceremony, host OA,
owner-secret sharing with hosts, private-key exchange, relay/native API extension,
Move enablement or production signed operation is added. Public metadata publication
remains the next separate slice and requires the **agent** signer, not the owner
instruction-library signer.
