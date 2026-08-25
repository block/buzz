# Operational adjustments

An adjustment batch progresses draft → previewed → posted → reversed. Editing a
draft replaces its lines and increments the version. Posting requires the exact
preview id/hash, batch version and unchanged global fact watermark; stale input
returns `STALE_PREVIEW`. Posted allocations/facts and reversal facts are
immutable. Supported metrics are management costs/rebates, never journal entries.
