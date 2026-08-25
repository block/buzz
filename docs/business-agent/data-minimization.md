# Data minimization and memory

The Business API should return only fields needed for the current answer. The
MCP result excludes bank accounts, tax ids, identity numbers, phone numbers,
addresses, cookies, tokens, secrets, invoice files, unrelated notes and bulk
detail. Buzz answers list at most ten rows and use Business Dock for more.

Business tool results are transient. Dedicated mode disables ACP core-memory
injection and provides no memory MCP. `buzz-agent` keeps tool results only in
the in-memory conversation needed to finish the Turn; the Host rotates the ACP
Session after each Business Turn. No raw result is written to NIP-AE/Engram,
Persona, files, or audit. A concise, redacted final Buzz answer may remain in
the collaboration record.

Audit contains counts and reason codes, never raw input/result or sensitive
business fields.
