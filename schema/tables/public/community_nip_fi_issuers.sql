-- ── NIP-FI community issuer bindings ──────────────────────────────────────────
-- The explicit per-community issuer allowlist for NIP-FI assertions. The
-- assertion audience is `https://<communities.host>`, so it needs no column of
-- its own. Zero rows means the community has no binding and NIP-FI admission
-- for it fails closed.

CREATE TABLE community_nip_fi_issuers (
    community_id UUID NOT NULL REFERENCES communities(id),
    issuer       TEXT NOT NULL CHECK (length(issuer) BETWEEN 1 AND 2048),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, issuer)
);
