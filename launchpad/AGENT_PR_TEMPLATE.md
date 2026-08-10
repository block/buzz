<!--
AGENT INSTRUCTIONS — read fully before writing this PR body.

HOW TO USE THIS FILE
  1. Read this file. Fill every field below. Delete every HTML comment.
  2. Write the result to a temp file and submit it:
       gh pr create -F /tmp/pr-body.md --label by:agent --base launchpad
  3. Do NOT pass --template. This file is a schema you fill, not a body you paste.

HARD RULES
  A. Do not add headings that are not in this file. Do not remove any.
  B. If a field does not apply, write exactly: N/A - <one-line reason>
  C. Never write "tests pass", "verified", or "works as expected" on their own.
     Paste the actual command and its RAW stdout/stderr in the fenced block.
     A reviewer must be able to read the output, not your summary of it.
  D. "Not verified" must never be empty and must never be "nothing".
     There is always something you did not check. Name it.
  E. You may draft everything here. You may not approve anything.
     Anything you were unsure about goes in Escalations, not into a decision.
  F. Every checkbox must be either [x] with evidence above it, or [ ] left unticked.
     Do not tick a box you cannot point at evidence for.
-->

## Summary
<!-- 3 sentences maximum. What changed and why. No preamble, no restating the issue title. -->

### Related issue
<!-- Closing keyword is required so the board updates on merge. -->
Closes #

### Issue type
<!-- One of: PRD | Task | Enhancement | Bug | ADR — must match the linked issue's type: label. -->

---

### Agent provenance
<!-- Exact values. "Claude" is not a model. "claude-sonnet-5" is. -->

| Field | Value |
|---|---|
| Harness / provider | <!-- e.g. Claude Code, GitHub Copilot, Codex --> |
| Model | <!-- exact model id --> |
| Session reference | <!-- run id or URL if the harness exposes one, else N/A --> |
| Initiating human | <!-- @handle of the person who asked for this work --> |

### Objective
<!-- One sentence: the artifact this PR creates or changes. Not a description of the diff. -->

### Impacted components
<!-- Real paths in this repo, one per line. Never "various", never "the system". -->

### Approach and rejected alternatives
<!-- Why this approach. Name at least one alternative you considered and why you rejected it.
     If you considered none, write: none considered - <reason>. -->

### Verification

Command run:
```
# paste the exact command(s)
```

Raw output:
```
# paste RAW stdout/stderr, unedited and untruncated
# if it is very long, paste the first 20 and last 20 lines and say so
```

- [ ] Tests or checks were run and the raw output is pasted above
- [ ] The diff is confined to the scope of the linked issue
- [ ] No secrets, keys, tokens or hostnames were added to tracked files

### Not verified
<!-- REQUIRED. What you could not check, and why. Examples: could not test against a
     live relay; no VPS access; did not run the desktop E2E suite. Be specific. -->

### Security implications
<!-- What this changes about exposure, trust, or blast radius.
     If you believe there are none, state why in one sentence. -->

### Escalations
<!-- Anything you raised rather than decided. Write "none" only if you genuinely
     hit no ambiguity. An empty escalations list on a complex change is a review signal. -->
