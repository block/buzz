---
name: github-research
description: "Search GitHub issues, PRs, and code using the gh CLI."
---

# GitHub Research

Search GitHub for prior art, implementation patterns, and maintainer decisions.

## Commands

```bash
# Search issues
gh search issues "topic" --repo owner/repo --limit 20 \
  --json number,title,state,url

# Search merged PRs (highest signal)
gh search prs "topic" --repo owner/repo --merged --limit 20 \
  --json number,title,url

# Search code (use query syntax for path filtering)
gh search code "pattern path:src/" --repo owner/repo --limit 20 \
  --json path,textMatches

# Get full issue or PR details
gh issue view 123 --repo owner/repo --json number,title,body,comments
gh pr view 456 --repo owner/repo --json number,title,body,reviews,files
```

## Rate Limits

- Search API: 30 requests/minute
- Check with: `gh api rate_limit --jq '.resources.search'`

## Signal Ranking

1. Merged PRs — decisions that shipped
2. Maintainer comments — authoritative
3. Closed issues with solutions — problems solved
4. Open issues — current problems (lower signal)

## Report Format

```markdown
## Research: [Topic]

### Summary
- Key finding 1 [#123]
- Key finding 2 [PR #456]

### Findings
1. **#123: [Title]** — [summary]. URL: https://...
2. **PR #456: [Title]** — [what it changed]. URL: https://...

### Gaps
- [What you looked for but didn't find]
```

Always include URLs. If nothing relevant exists, say so.
