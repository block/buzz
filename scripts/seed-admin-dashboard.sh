#!/usr/bin/env bash
# Seed deterministic moderation reports and product feedback for local dashboard review.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

if [[ -f ".env" ]]; then
  set -o allexport
  # shellcheck disable=SC1091
  source .env
  set +o allexport
fi

export PGHOST="${PGHOST:-localhost}"
export PGPORT="${PGPORT:-5432}"
export PGUSER="${PGUSER:-buzz}"
export PGPASSWORD="${PGPASSWORD:-buzz_dev}"
export PGDATABASE="${PGDATABASE:-buzz}"

if command -v psql >/dev/null 2>&1; then
  run_psql() {
    PGPASSWORD="${PGPASSWORD}" psql -h "${PGHOST}" -p "${PGPORT}" \
      -U "${PGUSER}" -d "${PGDATABASE}" "$@"
  }
elif docker exec buzz-postgres psql --version >/dev/null 2>&1; then
  run_psql() {
    docker exec -i -e PGPASSWORD="${PGPASSWORD}" buzz-postgres \
      psql -U "${PGUSER}" -d "${PGDATABASE}" "$@"
  }
else
  echo "error: neither psql nor buzz-postgres docker psql is available" >&2
  exit 1
fi

community_id="$(run_psql -At -v ON_ERROR_STOP=1 -c "
  SELECT id
  FROM communities
  WHERE lower(host) IN ('localhost:3000', 'localhost', '127.0.0.1:3000', '127.0.0.1')
  ORDER BY CASE lower(host)
    WHEN 'localhost:3000' THEN 1
    WHEN 'localhost' THEN 2
    WHEN '127.0.0.1:3000' THEN 3
    ELSE 4
  END
  LIMIT 1
")"
if [[ -z "${community_id}" ]]; then
  echo "error: local community is missing; run just setup first" >&2
  exit 1
fi

fixture_hash() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

fixture_size() {
  wc -c < "$1" | awk '{print $1}'
}

upload_fixture() {
  local path="$1" hash="$2" extension="$3" mime="$4" dimensions="$5"
  local size sidecar
  size="$(fixture_size "${path}")"
  sidecar="$(printf '{"dim":"%s","blurhash":"","thumb_url":"","ext":"%s","mime_type":"%s","size":%s,"uploaded_at":0}' \
    "${dimensions}" "${extension}" "${mime}" "${size}")"
  docker exec -i buzz-minio mc pipe --quiet --attr "Content-Type=${mime}" \
    "local/${BUZZ_S3_BUCKET:-buzz-media}/${hash}.${extension}" < "${path}"
  printf '%s' "${sidecar}" | docker exec -i buzz-minio mc pipe --quiet \
    --attr "Content-Type=application/json" \
    "local/${BUZZ_S3_BUCKET:-buzz-media}/_meta/${community_id}/${hash}.json"
}

fixture_dir="$(mktemp -d "${TMPDIR:-/tmp}/buzz-admin-feedback.XXXXXX")"
search_image="${REPO_ROOT}/docs/assets/screenshots/media-comments.png"
workspace_image="${REPO_ROOT}/docs/assets/screenshots/channel-thread.png"
quality_image="${REPO_ROOT}/docs/assets/screenshots/channel-agents.png"
composer_diagnostics="${fixture_dir}/composer-diagnostics.txt"
workspace_diagnostics="${fixture_dir}/workspace-diagnostics.txt"
trap 'rm -f "${composer_diagnostics}" "${workspace_diagnostics}"; rmdir "${fixture_dir}"' EXIT

printf '%s\n' "buzz feedback diagnostics" "area: composer" \
  "event: resumed_from_sleep" "result: composer_unresponsive" > "${composer_diagnostics}"
printf '%s\n' "buzz feedback diagnostics" "area: workspace-switching" \
  "from: design" "to: engineering" \
  "result: previous_sidebar_visible_for_one_frame" > "${workspace_diagnostics}"

search_image_hash="$(fixture_hash "${search_image}")"
workspace_image_hash="$(fixture_hash "${workspace_image}")"
quality_image_hash="$(fixture_hash "${quality_image}")"
composer_diagnostics_hash="$(fixture_hash "${composer_diagnostics}")"
workspace_diagnostics_hash="$(fixture_hash "${workspace_diagnostics}")"

if ! docker exec buzz-minio mc alias set local http://localhost:9000 \
  "${BUZZ_S3_ACCESS_KEY:-buzz_dev}" "${BUZZ_S3_SECRET_KEY:-buzz_dev_secret}" >/dev/null; then
  echo "error: local MinIO is unavailable; run just setup first" >&2
  exit 1
fi

read -r -d '' sql <<'SQL' || true
DO $$
DECLARE
  local_community_id UUID;
BEGIN
  SELECT id INTO local_community_id
  FROM communities
  WHERE lower(host) IN ('localhost:3000', 'localhost', '127.0.0.1:3000', '127.0.0.1')
  ORDER BY CASE lower(host)
    WHEN 'localhost:3000' THEN 1
    WHEN 'localhost' THEN 2
    WHEN '127.0.0.1:3000' THEN 3
    ELSE 4
  END
  LIMIT 1;

  IF local_community_id IS NULL THEN
    RAISE EXCEPTION 'local community is missing; run just setup first';
  END IF;

  -- A real channel for the failed-enforcement report below. Kick is only valid
  -- on `event` reports, and the relay rejects it pre-mutation unless the report
  -- carries a channel_id (FK into channels). Seeding this channel makes the
  -- Kick action reachable in the UI and lets the enforcement genuinely run and
  -- fail, so the Cancel & reopen recovery path is exercisable locally.
  INSERT INTO channels (community_id, id, name, created_by)
  VALUES (
    local_community_id,
    'c4a11e10-0000-4000-8000-000000000001',
    'seed-enforcement-channel',
    decode(repeat('3b', 32), 'hex')
  )
  ON CONFLICT (community_id, id) DO UPDATE SET name = EXCLUDED.name;

  INSERT INTO moderation_reports (
    community_id, id, report_event_id, reporter_pubkey, target_kind,
    target_event_id, target_pubkey, target_blob_sha256, channel_id, report_type, note,
    status, resolved_by, resolved_at, created_at
  ) VALUES
    (local_community_id, 'a11d0000-0000-4000-8000-000000000001', decode(repeat('01', 32), 'hex'), decode(repeat('11', 32), 'hex'), 'event', decode(repeat('21', 32), 'hex'), NULL, NULL, NULL, 'spam', 'Repeated unsolicited promotion across several channels.', 'open', NULL, NULL, now() - interval '8 minutes'),
    (local_community_id, 'a11d0000-0000-4000-8000-000000000002', decode(repeat('02', 32), 'hex'), decode(repeat('12', 32), 'hex'), 'pubkey', NULL, decode(repeat('22', 32), 'hex'), NULL, NULL, 'impersonation', 'Profile appears to impersonate a community organizer.', 'open', NULL, NULL, now() - interval '25 minutes'),
    (local_community_id, 'a11d0000-0000-4000-8000-000000000003', decode(repeat('03', 32), 'hex'), decode(repeat('13', 32), 'hex'), 'blob', NULL, NULL, decode(repeat('23', 32), 'hex'), NULL, 'malware', 'Attachment was flagged after download.', 'open', NULL, NULL, now() - interval '50 minutes'),
    (local_community_id, 'a11d0000-0000-4000-8000-000000000004', decode(repeat('04', 32), 'hex'), decode(repeat('14', 32), 'hex'), 'event', decode(repeat('24', 32), 'hex'), NULL, NULL, NULL, 'illegal', 'Contains material that may require legal review.', 'open', NULL, NULL, now() - interval '2 hours'),
    (local_community_id, 'a11d0000-0000-4000-8000-000000000005', decode(repeat('05', 32), 'hex'), decode(repeat('15', 32), 'hex'), 'blob', NULL, NULL, decode(repeat('25', 32), 'hex'), NULL, 'nudity', NULL, 'open', NULL, NULL, now() - interval '5 hours'),
    (local_community_id, 'a11d0000-0000-4000-8000-000000000006', decode(repeat('06', 32), 'hex'), decode(repeat('16', 32), 'hex'), 'pubkey', NULL, decode(repeat('26', 32), 'hex'), NULL, NULL, 'profanity', 'Repeated abusive replies from this account.', 'open', NULL, NULL, now() - interval '12 hours'),
    (local_community_id, 'a11d0000-0000-4000-8000-000000000007', decode(repeat('07', 32), 'hex'), decode(repeat('17', 32), 'hex'), 'event', decode(repeat('27', 32), 'hex'), NULL, NULL, NULL, 'other', 'Does not fit a standard report category.', 'open', NULL, NULL, now() - interval '1 day'),
    (local_community_id, 'a11d0000-0000-4000-8000-000000000008', decode(repeat('08', 32), 'hex'), decode(repeat('18', 32), 'hex'), 'pubkey', NULL, decode(repeat('28', 32), 'hex'), NULL, NULL, 'impersonation', 'Escalated while ownership is verified.', 'escalated', decode(repeat('38', 32), 'hex'), now() - interval '1 hour', now() - interval '2 days'),
    (local_community_id, 'a11d0000-0000-4000-8000-000000000009', decode(repeat('09', 32), 'hex'), decode(repeat('19', 32), 'hex'), 'blob', NULL, NULL, decode(repeat('29', 32), 'hex'), NULL, 'malware', 'Resolved after the attachment was removed.', 'resolved', decode(repeat('39', 32), 'hex'), now() - interval '1 day', now() - interval '3 days'),
    (local_community_id, 'a11d0000-0000-4000-8000-000000000010', decode(repeat('0a', 32), 'hex'), decode(repeat('1a', 32), 'hex'), 'event', decode(repeat('2a', 32), 'hex'), NULL, NULL, NULL, 'other', 'Dismissed after reviewing the surrounding thread.', 'dismissed', decode(repeat('3a', 32), 'hex'), now() - interval '3 days', now() - interval '4 days'),
    (local_community_id, 'a11d0000-0000-4000-8000-000000000011', decode(repeat('0b', 32), 'hex'), decode(repeat('1b', 32), 'hex'), 'event', decode(repeat('2b', 32), 'hex'), NULL, NULL, 'c4a11e10-0000-4000-8000-000000000001', 'spam', 'Event report in a real channel — Kick is offered and the enforcement genuinely runs and fails, exercising the Cancel & reopen recovery path.', 'open', NULL, NULL, now() - interval '3 minutes')
  ON CONFLICT (community_id, report_event_id) DO UPDATE SET
    reporter_pubkey = EXCLUDED.reporter_pubkey,
    target_kind = EXCLUDED.target_kind,
    target_event_id = EXCLUDED.target_event_id,
    target_pubkey = EXCLUDED.target_pubkey,
    target_blob_sha256 = EXCLUDED.target_blob_sha256,
    channel_id = EXCLUDED.channel_id,
    report_type = EXCLUDED.report_type,
    note = EXCLUDED.note,
    status = EXCLUDED.status,
    resolved_by = EXCLUDED.resolved_by,
    resolved_at = EXCLUDED.resolved_at,
    created_at = EXCLUDED.created_at;

  INSERT INTO product_feedback (
    id, community_id, event_id, submitter_pubkey, category, body, tags,
    event_created_at, received_at
  ) VALUES
    ('feed0000-0000-4000-8000-000000000001', local_community_id, decode(repeat('41', 32), 'hex'), decode(repeat('51', 32), 'hex'), 'bug', 'Unread counts return after reopening the desktop app.', '[["category", "bug"]]', now() - interval '20 minutes', now() - interval '19 minutes'),
    ('feed0000-0000-4000-8000-000000000002', local_community_id, decode(repeat('42', 32), 'hex'), decode(repeat('52', 32), 'hex'), 'needs-work', E'Search needs clearer empty-state guidance.\n![image](http://localhost:3000/media/__SEARCH_IMAGE_HASH__.png)', '[["category", "needs-work"], ["imeta", "url http://localhost:3000/media/__SEARCH_IMAGE_HASH__.png", "m image/png", "x __SEARCH_IMAGE_HASH__", "size __SEARCH_IMAGE_SIZE__", "dim 2000x1172", "filename search-empty-state.png"]]', now() - interval '5 hours', now() - interval '5 hours'),
    ('feed0000-0000-4000-8000-000000000003', local_community_id, decode(repeat('43', 32), 'hex'), decode(repeat('53', 32), 'hex'), 'praise', 'The new channel switcher feels immediate.', '[["category", "praise"]]', now() - interval '1 day', now() - interval '1 day'),
    ('feed0000-0000-4000-8000-000000000004', local_community_id, decode(repeat('44', 32), 'hex'), decode(repeat('54', 32), 'hex'), 'bug', E'The composer froze after waking my laptop. Diagnostics attached.\n[feedback-diagnostics.txt](http://localhost:3000/media/__COMPOSER_DIAGNOSTICS_HASH__.txt)', '[["category", "bug"], ["imeta", "url http://localhost:3000/media/__COMPOSER_DIAGNOSTICS_HASH__.txt", "m text/plain", "x __COMPOSER_DIAGNOSTICS_HASH__", "size __COMPOSER_DIAGNOSTICS_SIZE__", "filename feedback-diagnostics.txt"]]', now() - interval '2 days', now() - interval '2 days'),
    ('feed0000-0000-4000-8000-000000000005', local_community_id, decode(repeat('45', 32), 'hex'), decode(repeat('55', 32), 'hex'), NULL, 'General feedback without a selected category or any attachments.', '[]', now() - interval '3 days', now() - interval '3 days'),
    ('feed0000-0000-4000-8000-000000000006', local_community_id, decode(repeat('46', 32), 'hex'), decode(repeat('56', 32), 'hex'), 'needs-work', E'The sidebar briefly renders the previous workspace after switching. Screenshot and diagnostics attached.\n![image](http://localhost:3000/media/__WORKSPACE_IMAGE_HASH__.png)\n[feedback-diagnostics.txt](http://localhost:3000/media/__WORKSPACE_DIAGNOSTICS_HASH__.txt)', '[["category", "needs-work"], ["imeta", "url http://localhost:3000/media/__WORKSPACE_IMAGE_HASH__.png", "m image/png", "x __WORKSPACE_IMAGE_HASH__", "size __WORKSPACE_IMAGE_SIZE__", "dim 2000x1172", "filename workspace-flash.png"], ["imeta", "url http://localhost:3000/media/__WORKSPACE_DIAGNOSTICS_HASH__.txt", "m text/plain", "x __WORKSPACE_DIAGNOSTICS_HASH__", "size __WORKSPACE_DIAGNOSTICS_SIZE__", "filename feedback-diagnostics.txt"]]', now() - interval '5 days', now() - interval '5 days'),
    ('feed0000-0000-4000-8000-000000000007', local_community_id, decode(repeat('47', 32), 'hex'), decode(repeat('57', 32), 'hex'), 'praise', E'Calls have been much more reliable this week. Attaching the quality graph that made the improvement obvious.\n![image](http://localhost:3000/media/__QUALITY_IMAGE_HASH__.png)', '[["category", "praise"], ["imeta", "url http://localhost:3000/media/__QUALITY_IMAGE_HASH__.png", "m image/png", "x __QUALITY_IMAGE_HASH__", "size __QUALITY_IMAGE_SIZE__", "dim 2000x1172", "filename huddle-quality.png"]]', now() - interval '8 days', now() - interval '8 days')
  ON CONFLICT (event_id) DO UPDATE SET
    community_id = EXCLUDED.community_id,
    submitter_pubkey = EXCLUDED.submitter_pubkey,
    category = EXCLUDED.category,
    body = EXCLUDED.body,
    tags = EXCLUDED.tags,
    event_created_at = EXCLUDED.event_created_at,
    received_at = EXCLUDED.received_at,
    status = 'new';
END $$;
SQL

sql="${sql//__SEARCH_IMAGE_HASH__/${search_image_hash}}"
sql="${sql//__WORKSPACE_IMAGE_HASH__/${workspace_image_hash}}"
sql="${sql//__QUALITY_IMAGE_HASH__/${quality_image_hash}}"
sql="${sql//__COMPOSER_DIAGNOSTICS_HASH__/${composer_diagnostics_hash}}"
sql="${sql//__WORKSPACE_DIAGNOSTICS_HASH__/${workspace_diagnostics_hash}}"
sql="${sql//__SEARCH_IMAGE_SIZE__/$(fixture_size "${search_image}")}"
sql="${sql//__WORKSPACE_IMAGE_SIZE__/$(fixture_size "${workspace_image}")}"
sql="${sql//__QUALITY_IMAGE_SIZE__/$(fixture_size "${quality_image}")}"
sql="${sql//__COMPOSER_DIAGNOSTICS_SIZE__/$(fixture_size "${composer_diagnostics}")}"
sql="${sql//__WORKSPACE_DIAGNOSTICS_SIZE__/$(fixture_size "${workspace_diagnostics}")}"

# Multi-community data: two named communities plus filler so the directory
# pages past 50, named members, one ban and one timeout per named community,
# extra reports and feedback, and one pre-signed kind-9 message each. The
# messages were signed once with fixed test keys (secret = 5eed + the member
# number, zero-padded to 32 bytes), so their signatures stay valid forever.
read -r -d '' multi_sql <<'SQL' || true
CREATE TEMP TABLE seed_community (host TEXT, channel_id UUID, author INT, event_id TEXT, sig TEXT, members INT[]);
INSERT INTO seed_community VALUES
  ('localhost:3000', 'c4a11e10-0000-4000-8000-000000000001', 1,
   '0fe4de81609e02e263b268f7069882f2db68dd74c3f62074be31eaab24b0b5fc',
   '63d4a64362bf1f91e56ea1f0001a196d4bc9a0e26712d8083516ffe054665a04fc7c3a13a554d327d0a37e0439bc67d9c0bf2252f595d9b8ce5a09587192e06d',
   '{1,2,5,6}'),
  ('beta.localhost:3000', 'c4a11e10-0000-4000-8000-000000000002', 3,
   '50b89df773d8009a88d43ad707ebe0054c7e619a6c13727e0984aca0f930d03c',
   'b7d6c122ff4589498c05b3cabd26c7759d0acc497c747f9331130fd5f65e645203313b7c32ddfd97ac2fde0d2341aa4f189135e2b188d8bd7689a54e7c5787b4',
   '{1,3,5,6}'),
  ('gamma.localhost:3000', 'c4a11e10-0000-4000-8000-000000000003', 4,
   '108f54732c522a97e2481b41b8b5a588a99c536ae25d5c0c2e9a1a0c28189f00',
   '839d536f36f998020596c080e2356f881a43af7d046b19b6e8cd214b8c097720cdeea5cc3173a60da8b671846580c4612c5e5fa1e34002ce6c3754bba683fb71',
   '{2,4,5,6}');

CREATE TEMP TABLE seed_member (n INT, pubkey BYTEA, display_name TEXT);
INSERT INTO seed_member VALUES
  (1, decode('7a863b2e9a491321daf769357e398269d074730493cd1ca14812fbfd07cece70', 'hex'), 'Ada Lovelace'),
  (2, decode('ef1e7fbda3e8776e2cadd5ee1fc3cd3728014b18814ea21d15748aca6a1c5c27', 'hex'), 'Grace Hopper'),
  (3, decode('5929d0d248167876c88ded48bfd901389f644f5c47b967cab2c112ef9bf14f1a', 'hex'), 'Linus Pauling'),
  (4, decode('b75653278afb3d4522fdf8bd8d418c2085c2995c273d0357540c21374be953d5', 'hex'), 'Rosalind Franklin'),
  (5, decode('2ad6553f16e40b22045a68ffe6ba31ae5f400470d0e6276f377d50fc23f936a3', 'hex'), 'Spam Account'),
  (6, decode('9eb3e44e539549ff1436b6f98b3b583b8f89326d1cbee78a4edaea042cd34057', 'hex'), 'Heated Debater');

CREATE TEMP VIEW seeded AS
SELECT s.*, c.id AS community_id
FROM seed_community s
JOIN communities c ON lower(c.host) = lower(s.host)
  OR (s.host = 'localhost:3000' AND c.id = :'local_community_id');

-- Refuse to reset while the admin console has unfinished work on a fixture:
-- forcing a processing report back to open strands its action, and a pending
-- action could re-apply after the reset. Locking admin actions blocks new
-- claims and direct actions until this transaction commits.
-- A claim or reopen locks its report row before writing an action, so the
-- fixture rows are taken with NOWAIT: waiting on one would deadlock.
LOCK TABLE relay_admin_actions IN SHARE ROW EXCLUSIVE MODE;
CREATE TEMP VIEW seed_report AS
SELECT r.* FROM moderation_reports r
JOIN seeded s ON s.community_id = r.community_id
JOIN (SELECT 'localhost:3000', to_hex(n) FROM generate_series(1, 11) n
      UNION ALL VALUES ('beta.localhost:3000', '61'), ('beta.localhost:3000', '62'),
                       ('gamma.localhost:3000', '63')) k(host, byte)
  ON k.host = s.host AND r.report_event_id = decode(repeat(lpad(k.byte, 2, '0'), 32), 'hex');
DO $$
BEGIN
  PERFORM 1 FROM moderation_reports r JOIN seed_report f USING (community_id, id) FOR UPDATE OF r NOWAIT;
EXCEPTION WHEN lock_not_available THEN
  RAISE EXCEPTION 'admin-seed refused, nothing was written: moderation is active on a seeded report. Finish it in the admin console, then rerun just admin-seed.';
END $$;
CREATE TEMP TABLE seed_busy AS
SELECT format('report %s in %s is %s', r.id, c.host, r.status) AS what
FROM seed_report r JOIN communities c ON c.id = r.community_id
WHERE r.status = 'processing' OR r.active_action_id IS NOT NULL
UNION ALL
SELECT format('%s action %s in %s is %s', a.action, a.id, c.host, a.state)
FROM relay_admin_actions a JOIN communities c ON c.id = a.report_community_id
WHERE a.state IN ('pending', 'enforcing')
  AND a.report_community_id IN (SELECT community_id FROM seeded)
  AND (a.enforcement_target_pubkey IN (SELECT pubkey FROM seed_member)
    OR a.enforcement_target_event_id IN (SELECT decode(event_id, 'hex') FROM seed_community));
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM seed_busy) THEN
    RAISE EXCEPTION E'admin-seed refused, nothing was written: %\nFinish or cancel these in the admin console (Cancel & reopen for a failed report action), wait for pending actions to complete, then rerun just admin-seed.',
      (SELECT string_agg(what, '; ') FROM seed_busy);
  END IF;
END $$;

INSERT INTO communities (host)
SELECT host FROM seed_community WHERE host <> 'localhost:3000'
UNION ALL
SELECT format('filler-%s.localhost:3000', lpad(n::text, 2, '0')) FROM generate_series(1, 50) n
ON CONFLICT ((lower(host))) DO NOTHING;

INSERT INTO channels (community_id, id, name, created_by)
SELECT community_id, channel_id, 'seed-enforcement-channel', decode(repeat('3b', 32), 'hex') FROM seeded
ON CONFLICT (community_id, id) DO UPDATE SET name = EXCLUDED.name;

INSERT INTO users (community_id, pubkey, display_name)
SELECT s.community_id, m.pubkey, m.display_name
FROM seeded s JOIN seed_member m ON m.n = ANY (s.members)
ON CONFLICT (community_id, pubkey) DO UPDATE SET display_name = EXCLUDED.display_name;

INSERT INTO relay_members (community_id, pubkey, role, added_by)
SELECT s.community_id, encode(m.pubkey, 'hex'), 'member', NULL
FROM seeded s JOIN seed_member m ON m.n = ANY (s.members)
ON CONFLICT (community_id, pubkey) DO NOTHING;

INSERT INTO channel_members (community_id, channel_id, pubkey)
SELECT community_id, channel_id, (SELECT pubkey FROM seed_member WHERE n = author) FROM seeded
ON CONFLICT (community_id, channel_id, pubkey) DO UPDATE SET removed_at = NULL, removed_by = NULL;

INSERT INTO community_bans (community_id, pubkey, banned, ban_reason, muted_until, mute_reason, actor_pubkey)
SELECT s.community_id, m.pubkey, m.n = 5,
  CASE WHEN m.n = 5 THEN 'Seeded ban: repeated spam.' END,
  CASE WHEN m.n = 6 THEN now() + interval '1 day' END,
  CASE WHEN m.n = 6 THEN 'Seeded timeout: heated thread.' END,
  decode(repeat('3b', 32), 'hex')
FROM seeded s JOIN seed_member m ON m.n = ANY (s.members)
ON CONFLICT (community_id, pubkey) DO UPDATE SET
  banned = EXCLUDED.banned, ban_expires_at = NULL, ban_reason = EXCLUDED.ban_reason,
  muted_until = EXCLUDED.muted_until, mute_reason = EXCLUDED.mute_reason, updated_at = now();

INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, channel_id)
SELECT s.community_id, decode(s.event_id, 'hex'), m.pubkey, to_timestamp(1790000000), 9,
  jsonb_build_array(jsonb_build_array('h', s.channel_id::text)),
  format('Seeded message in %s for the admin delete preview.', s.host),
  decode(s.sig, 'hex'), s.channel_id
FROM seeded s JOIN seed_member m ON m.n = s.author
ON CONFLICT (community_id, created_at, id) DO UPDATE SET deleted_at = NULL;

INSERT INTO moderation_reports (
  community_id, id, report_event_id, reporter_pubkey, target_kind,
  target_event_id, target_pubkey, channel_id, report_type, note, status, created_at
)
SELECT s.community_id, ('a11d0000-0000-4000-8000-0000000001' || lpad(r.n::text, 2, '0'))::uuid,
  decode(repeat(to_hex(96 + r.n), 32), 'hex'), decode(repeat('1c', 32), 'hex'),
  r.kind, CASE WHEN r.kind = 'event' THEN decode(s.event_id, 'hex') END,
  CASE WHEN r.kind = 'pubkey' THEN (SELECT pubkey FROM seed_member WHERE n = 5) END,
  CASE WHEN r.kind = 'event' THEN s.channel_id END,
  r.report_type, format('%s report in %s.', initcap(r.report_type), s.host), 'open',
  now() - r.n * interval '7 minutes'
FROM seeded s
JOIN (VALUES (1, 'beta.localhost:3000', 'event', 'spam'),
             (2, 'beta.localhost:3000', 'pubkey', 'impersonation'),
             (3, 'gamma.localhost:3000', 'event', 'profanity')) r(n, host, kind, report_type)
  ON r.host = s.host
ON CONFLICT (community_id, report_event_id) DO UPDATE SET
  channel_id = EXCLUDED.channel_id, status = EXCLUDED.status, resolved_by = NULL,
  resolved_at = NULL, created_at = EXCLUDED.created_at;

INSERT INTO product_feedback (id, community_id, event_id, submitter_pubkey, category, body, tags, event_created_at, received_at)
SELECT ('feed0000-0000-4000-8000-0000000001' || lpad(f.n::text, 2, '0'))::uuid, s.community_id,
  decode(repeat(to_hex(112 + f.n), 32), 'hex'), (SELECT pubkey FROM seed_member WHERE n = f.member),
  f.category, format('%s (from %s)', f.body, s.host), jsonb_build_array(jsonb_build_array('category', f.category)),
  now() - f.n * interval '3 hours', now() - f.n * interval '3 hours'
FROM seeded s
JOIN (VALUES (1, 'beta.localhost:3000', 3, 'bug', 'Notifications arrive twice on mobile.'),
             (2, 'gamma.localhost:3000', 4, 'praise', 'Thread replies load instantly now.')) f(n, host, member, category, body)
  ON f.host = s.host
ON CONFLICT (event_id) DO UPDATE SET
  community_id = EXCLUDED.community_id, body = EXCLUDED.body, event_created_at = EXCLUDED.event_created_at,
  received_at = EXCLUDED.received_at, status = 'new';
SQL

# One transaction: the guard in multi_sql locks admin actions and refuses before
# any write, so the reset cannot race a claim. Uploads only follow a commit.
run_psql -q -1 -v ON_ERROR_STOP=1 -v local_community_id="${community_id}" \
  <<< "${multi_sql}"$'\n'"${sql}"

upload_fixture "${search_image}" "${search_image_hash}" png image/png 2000x1172
upload_fixture "${workspace_image}" "${workspace_image_hash}" png image/png 2000x1172
upload_fixture "${quality_image}" "${quality_image_hash}" png image/png 2000x1172
upload_fixture "${composer_diagnostics}" "${composer_diagnostics_hash}" txt text/plain ""
upload_fixture "${workspace_diagnostics}" "${workspace_diagnostics_hash}" txt text/plain ""

cat <<'EOF'
Seeded 14 moderation reports, 9 feedback entries, and 5 attachments, plus
beta.localhost:3000, gamma.localhost:3000 and 50 filler-NN.localhost:3000 communities.

Communities (with localhost:3000): beta.localhost:3000, gamma.localhost:3000
Members (display name, npub, communities):
  Ada Lovelace       npub102rrkt56fyfjrkhhdy6huwvzd8g8gucyj0x3eg2gztal6p7weecqlj45d5  localhost, beta
  Grace Hopper       npub1au08l0drapmkut9d6hhpls7dxu5qzjccs982y8g4wj9v56sutsnshfethq  localhost, gamma
  Linus Pauling      npub1ty5ap5jgzeu8djyda4ytlkgp8z0kgn6ug7uk0j4jcyfwlxl3fudq59tr5r  beta
  Rosalind Franklin  npub1kat9xfu2lv752ghalz7c6svvyzzu9x2uyu7sx465pssnwjlf202s04ej99  gamma
  Spam Account       npub19tt920ckus9jypz6drl7dw334e05qprs6rnzwmeh04g0cglex63s34trhf  all three, banned
  Heated Debater     npub1n6e7gnjnj4yl79pkkmuckw6c8w8cjvndrjlw0zjwmt4qgtxngptse52a73  all three, timed out 1 day
Messages (rerunning the seed restores deleted ones):
  localhost:3000        note1pljdaqtqncpwycajdrmsdxyz7tdk3ht5c0mzqa97x842kf9skh7q9whkgc
                        nevent1qqsqlex7s9sfuqhzvwex3acxnzp09kmgm46v8a3qwjlrr64tyjcttlqrmlq9s
  beta.localhost:3000   note12zufmamnmqqf4zx58tts06lqq4x8ucv6dsfhylsfsjk2p7fs6q7q2svrv4
                        nevent1qqs9pwya7aeasqy63r2r44c8a0sq2nr7vxdxcymj0cycft9qlycdq0q2n8jdv
  gamma.localhost:3000  note1zz84guev2g4f0cjgrdqm3dd93z5ec5m2ufw4crpwngdqc2qcnuqqd7x0uy
                        nevent1qqsppr65wvk9y25hufypksdckkjc32vu2d4wyh2upshf5xsv9qvf7qqft8acc
EOF
