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

upload_fixture "${search_image}" "${search_image_hash}" png image/png 2000x1172
upload_fixture "${workspace_image}" "${workspace_image_hash}" png image/png 2000x1172
upload_fixture "${quality_image}" "${quality_image_hash}" png image/png 2000x1172
upload_fixture "${composer_diagnostics}" "${composer_diagnostics_hash}" txt text/plain ""
upload_fixture "${workspace_diagnostics}" "${workspace_diagnostics_hash}" txt text/plain ""

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

  INSERT INTO moderation_reports (
    community_id, id, report_event_id, reporter_pubkey, target_kind,
    target_event_id, target_pubkey, target_blob_sha256, report_type, note,
    status, resolved_by, resolved_at, created_at
  ) VALUES
    (local_community_id, 'a11d0000-0000-4000-8000-000000000001', decode(repeat('01', 32), 'hex'), decode(repeat('11', 32), 'hex'), 'event', decode(repeat('21', 32), 'hex'), NULL, NULL, 'spam', 'Repeated unsolicited promotion across several channels.', 'open', NULL, NULL, now() - interval '8 minutes'),
    (local_community_id, 'a11d0000-0000-4000-8000-000000000002', decode(repeat('02', 32), 'hex'), decode(repeat('12', 32), 'hex'), 'pubkey', NULL, decode(repeat('22', 32), 'hex'), NULL, 'impersonation', 'Profile appears to impersonate a community organizer.', 'open', NULL, NULL, now() - interval '25 minutes'),
    (local_community_id, 'a11d0000-0000-4000-8000-000000000003', decode(repeat('03', 32), 'hex'), decode(repeat('13', 32), 'hex'), 'blob', NULL, NULL, decode(repeat('23', 32), 'hex'), 'malware', 'Attachment was flagged after download.', 'open', NULL, NULL, now() - interval '50 minutes'),
    (local_community_id, 'a11d0000-0000-4000-8000-000000000004', decode(repeat('04', 32), 'hex'), decode(repeat('14', 32), 'hex'), 'event', decode(repeat('24', 32), 'hex'), NULL, NULL, 'illegal', 'Contains material that may require legal review.', 'open', NULL, NULL, now() - interval '2 hours'),
    (local_community_id, 'a11d0000-0000-4000-8000-000000000005', decode(repeat('05', 32), 'hex'), decode(repeat('15', 32), 'hex'), 'blob', NULL, NULL, decode(repeat('25', 32), 'hex'), 'nudity', NULL, 'open', NULL, NULL, now() - interval '5 hours'),
    (local_community_id, 'a11d0000-0000-4000-8000-000000000006', decode(repeat('06', 32), 'hex'), decode(repeat('16', 32), 'hex'), 'pubkey', NULL, decode(repeat('26', 32), 'hex'), NULL, 'profanity', 'Repeated abusive replies from this account.', 'open', NULL, NULL, now() - interval '12 hours'),
    (local_community_id, 'a11d0000-0000-4000-8000-000000000007', decode(repeat('07', 32), 'hex'), decode(repeat('17', 32), 'hex'), 'event', decode(repeat('27', 32), 'hex'), NULL, NULL, 'other', 'Does not fit a standard report category.', 'open', NULL, NULL, now() - interval '1 day'),
    (local_community_id, 'a11d0000-0000-4000-8000-000000000008', decode(repeat('08', 32), 'hex'), decode(repeat('18', 32), 'hex'), 'pubkey', NULL, decode(repeat('28', 32), 'hex'), NULL, 'impersonation', 'Escalated while ownership is verified.', 'escalated', decode(repeat('38', 32), 'hex'), now() - interval '1 hour', now() - interval '2 days'),
    (local_community_id, 'a11d0000-0000-4000-8000-000000000009', decode(repeat('09', 32), 'hex'), decode(repeat('19', 32), 'hex'), 'blob', NULL, NULL, decode(repeat('29', 32), 'hex'), 'malware', 'Resolved after the attachment was removed.', 'resolved', decode(repeat('39', 32), 'hex'), now() - interval '1 day', now() - interval '3 days'),
    (local_community_id, 'a11d0000-0000-4000-8000-000000000010', decode(repeat('0a', 32), 'hex'), decode(repeat('1a', 32), 'hex'), 'event', decode(repeat('2a', 32), 'hex'), NULL, NULL, 'other', 'Dismissed after reviewing the surrounding thread.', 'dismissed', decode(repeat('3a', 32), 'hex'), now() - interval '3 days', now() - interval '4 days')
  ON CONFLICT (community_id, report_event_id) DO UPDATE SET
    reporter_pubkey = EXCLUDED.reporter_pubkey,
    target_kind = EXCLUDED.target_kind,
    target_event_id = EXCLUDED.target_event_id,
    target_pubkey = EXCLUDED.target_pubkey,
    target_blob_sha256 = EXCLUDED.target_blob_sha256,
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
    received_at = EXCLUDED.received_at;
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

run_psql -v ON_ERROR_STOP=1 -c "${sql}"

echo "Seeded 10 moderation reports, 7 feedback entries, and 5 attachments for the local admin dashboard."
