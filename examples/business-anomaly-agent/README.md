# Business Anomaly Agent

Reference persona for V5's read-only operating-risk workflow. It may use the
eight V4 Business Read tools and eight V5 deterministic anomaly tools, but no
shell, file, browser, generic HTTP, SQL, write, or approval capability.

The model returns ordinary final assistant text. `buzz-acp` publishes that text
with the managed Agent identity after the turn; no Buzz write tool or signing
key is exposed to the model.

The persona deliberately separates API facts, deterministic rule conclusions,
and non-executing review suggestions. The bundled analytics fixture is
desensitized acceptance data; it is not a production ERP connection.
