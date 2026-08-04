# agentic-sdlc-mcp community integration proposal

[`agentic-sdlc-mcp`](https://github.com/SakuraCianna/agentic-sdlc-mcp)
is a third-party MCP server that provides GitHub-based SDLC governance
and evidence controls for AI coding agents.

It is intended to complement Buzz rather than replace any Buzz component.

## Responsibility split

Buzz provides:

- agent collaboration and identity;
- channels and shared workflows;
- event history and human coordination.

`agentic-sdlc-mcp` provides:

- bounded GitHub repository context;
- risk-aware preparation of GitHub work items;
- pull-request quality-gate evidence;
- repository governance and security evidence;
- release-readiness and agent-handoff packets;
- explicit human approval checkpoints.

Coding and execution tools continue to perform code changes, tests,
commits, and pull-request operations.

## Example composition

```text
Buzz channel or workflow
→ Buzz-connected coding agent
→ agentic-sdlc-mcp
→ bounded GitHub context and SDLC decision artifacts
→ coding or execution tools
→ pull request, checks, review, and human approval in Buzz
```

## Safety boundary

The MCP currently exposes 13 workflow-level tools.

Twelve tools are read-only.
The only GitHub write tool creates Issues.
The write tool uses a dry-run preview by default.
The MCP does not write code, merge pull requests, create releases,
deploy software, or replace human security review.
Project links
GitHub: https://github.com/SakuraCianna/agentic-sdlc-mcp
npm: https://www.npmjs.com/package/agentic-sdlc-mcp
MCP Registry: https://registry.modelcontextprotocol.io/v0.1/servers?search=io.github.SakuraCianna%2Fagentic-sdlc-mcp
```

## Status

This is a proposed third-party community integration, not an official
Buzz component or a dependency maintained by the Buzz project.
