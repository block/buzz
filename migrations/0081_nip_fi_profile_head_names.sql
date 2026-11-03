-- Profile name search matches verified profiles by name. Index the names in
-- each current head; the search query repeats this expression exactly
-- (crates/buzz-search/src/query.rs, verified_names_tsv). One row per
-- verified person, so a plain build is bounded.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

CREATE INDEX idx_nip_fi_profile_heads_names ON nip_fi_profile_heads USING GIN (
    to_tsvector('simple',
        COALESCE(jsonb_path_query_first(event, 'strict $.tags[*] ? (@[0] == "display_name")[1]', '{}', true) #>> '{}', '')
        || ' ' || COALESCE(jsonb_path_query_first(event, 'strict $.tags[*] ? (@[0] == "username")[1]', '{}', true) #>> '{}', '')
        || ' ' || COALESCE(jsonb_path_query_first(event, 'strict $.tags[*] ? (@[0] == "full_name")[1]', '{}', true) #>> '{}', ''))
);
