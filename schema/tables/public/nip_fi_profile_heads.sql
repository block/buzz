CREATE TABLE nip_fi_profile_heads (
    community_id UUID NOT NULL,
    pubkey TEXT NOT NULL,
    issuer TEXT NOT NULL,
    issued_at NUMERIC(30, 0) NOT NULL,
    event JSONB NOT NULL,
    PRIMARY KEY (community_id, pubkey)
);

-- Profile name search over verified names
-- (crates/buzz-search/src/query.rs, verified_names_tsv).
CREATE INDEX idx_nip_fi_profile_heads_names ON nip_fi_profile_heads USING GIN (
    to_tsvector('simple',
        COALESCE(jsonb_path_query_first(event, 'strict $.tags[*] ? (@[0] == "display_name")[1]', '{}', true) #>> '{}', '')
        || ' ' || COALESCE(jsonb_path_query_first(event, 'strict $.tags[*] ? (@[0] == "username")[1]', '{}', true) #>> '{}', '')
        || ' ' || COALESCE(jsonb_path_query_first(event, 'strict $.tags[*] ? (@[0] == "full_name")[1]', '{}', true) #>> '{}', ''))
);
