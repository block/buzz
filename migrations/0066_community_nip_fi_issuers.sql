-- NIP-FI community issuer bindings: the explicit issuer allowlist for each
-- community, written when a community is created and by the operator
-- backfill. The assertion audience is `https://<communities.host>`. Zero rows
-- means no binding, so NIP-FI admission for that community fails closed.
--
-- Additive. Nothing reads this table at admission time yet.
CREATE TABLE community_nip_fi_issuers (
    community_id UUID NOT NULL REFERENCES communities(id),
    issuer       TEXT NOT NULL CHECK (length(issuer) BETWEEN 1 AND 2048),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, issuer)
);

SELECT attach_community_write_fence('community_nip_fi_issuers');
