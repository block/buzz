# ADR 015: Deterministic profit allocation

Accepted. Allocation uses versioned business bases, decimal arithmetic, cent
rounding, largest remainder and stable UUID tie-breaking. Preview binds the
inputs by hash and watermark. An LLM cannot calculate or override allocation:
its output is nondeterministic and is not an auditable monetary control.
