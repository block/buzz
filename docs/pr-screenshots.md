# PR screenshots

Use this guide when a pull request needs screenshots. It covers capture,
verification, and GitHub-safe posting; test behavior belongs in
[`desktop/tests/e2e/AGENTS.md`](../desktop/tests/e2e/AGENTS.md).

## Capture

Desktop screenshots need the E2E mock bridge and cannot be rendered in a plain
browser. The helper builds the frontend, starts a preview if necessary, and
captures a PNG:

```bash
just desktop-screenshot --name home
just desktop-screenshot --name channel --route /channels/general
just desktop-screenshot --name search --click open-search
just desktop-screenshot --name settings --click open-settings
```

Useful options are `--active-channel`, `--click`, `--right-click`, `--hover`,
`--clip`, `--wait`, `--viewport`, `--outdir`, and `--messages`. `--messages`
takes a JSON array whose required fields are `channelName` and `content`; its
other fields are passed to the mock event helper. Without `--active-channel`,
all seeded messages must target one channel. With it, messages can target other
channels to demonstrate badges or unread state while the view stays put.

```json
[
  {
    "channelName": "random",
    "content": "Hey @tyler, check this out",
    "pubkey": "953d...",
    "kind": 40002,
    "mentionPubkeys": ["deadbeef..."],
    "extraTags": [["broadcast", "1"]],
    "parentEventId": "abc123"
  }
]
```

Examples for common focused states:

```bash
just desktop-screenshot --name code-blocks --messages /tmp/messages.json
just desktop-screenshot --name unread-dot --active-channel general --messages /tmp/badge-messages.json
just desktop-screenshot --name sidebar-unread --active-channel general --messages /tmp/badge-messages.json --clip 0,0,256,720
just desktop-screenshot --name context-menu --active-channel general --messages /tmp/badge-messages.json --right-click channel-random --clip 0,200,320,300
```

Available mock channels are `general`, `random`, `design`, `sales`,
`engineering`, `agents`, `watercooler`, `announcements`, `alice-tyler`, and
`bob-tyler`. `general` has pre-seeded messages; use `engineering` when a
no-unread state matters.

For seeded state, live messages, or interaction before capture, add a Playwright
spec under `desktop/tests/e2e/` and register it in the appropriate Playwright
project. Follow the E2E instruction file for bridge, subscription, animation,
and scoped-capture requirements.

## Verify

Use a crop or locator capture for focused UI, especially sidebars and menus.
Before posting multiple screenshots intended to represent different states,
confirm their hashes differ:

```bash
shasum -a 256 test-results/<directory>/*.png
```

Identical hashes mean the captures are not distinct; fix the spec or capture
setup before posting.

## Post to GitHub

Do not link Buzz relay media URLs in GitHub PR markdown: GitHub’s image proxy
cannot reliably fetch them. Post PNGs with the repository script, which creates
GitHub-safe, commit-addressed URLs on the screenshot branch:

```bash
./scripts/post-screenshots.sh <pr-number> test-results/screenshots
./scripts/post-screenshots.sh <pr-number> test-results/screenshots body.md
```

The optional body file may contain `{{filename}}` placeholders (without
`.png`); the script replaces them with images and appends unreferenced files.
Use a heading and short description for each state. The script checks a supplied
body file for relay-media URLs; when editing PR markdown without it, run:

```markdown
### Unread indicator

A message arrives in `#random`.

{{unread-dot}}
```

```bash
./scripts/check-pr-image-urls.sh <markdown-file>
```

The posting script appends a new PR comment on every run. If you replace a
screenshot set, remove the superseded comment so reviewers see the current
evidence:

```bash
gh pr view <pr-number> --repo block/buzz --json comments --jq '.comments[] | select(.body | test("pr-<pr-number>--")) | {id, url}'
gh api -X DELETE repos/block/buzz/issues/comments/<stale-comment-id>
```

Screenshot branches can be removed after the PR is complete:

```bash
git push origin --delete agent-screenshots/<github-username>
```
