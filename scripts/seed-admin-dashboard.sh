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
    (local_community_id, 'a11d0000-0000-4000-8000-000000000002', decode(repeat('02', 32), 'hex'), decode(repeat('12', 32), 'hex'), 'pubkey', NULL, decode(repeat('22', 32), 'hex'), NULL, 'impersonation', 'Profile appears to impersonate a community organizer.', 'escalated', decode(repeat('32', 32), 'hex'), now() - interval '1 hour', now() - interval '2 hours'),
    (local_community_id, 'a11d0000-0000-4000-8000-000000000003', decode(repeat('03', 32), 'hex'), decode(repeat('13', 32), 'hex'), 'blob', NULL, NULL, decode(repeat('23', 32), 'hex'), 'malware', 'Attachment was flagged after download.', 'resolved', decode(repeat('33', 32), 'hex'), now() - interval '1 day', now() - interval '2 days'),
    (local_community_id, 'a11d0000-0000-4000-8000-000000000004', decode(repeat('04', 32), 'hex'), decode(repeat('14', 32), 'hex'), 'event', decode(repeat('24', 32), 'hex'), NULL, NULL, 'other', 'Context sample for a dismissed report.', 'dismissed', decode(repeat('34', 32), 'hex'), now() - interval '3 days', now() - interval '4 days')
  ON CONFLICT (community_id, report_event_id) DO UPDATE SET
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
    ('feed0000-0000-4000-8000-000000000001', local_community_id, decode(repeat('41', 32), 'hex'), decode(repeat('51', 32), 'hex'), 'bug', 'Unread counts return after reopening the desktop app.', '["desktop", "notifications"]', now() - interval '20 minutes', now() - interval '19 minutes'),
    ('feed0000-0000-4000-8000-000000000002', local_community_id, decode(repeat('42', 32), 'hex'), decode(repeat('52', 32), 'hex'), 'needs-work', 'Search needs clearer empty-state guidance.', '["search", "ux"]', now() - interval '5 hours', now() - interval '5 hours'),
    ('feed0000-0000-4000-8000-000000000003', local_community_id, decode(repeat('43', 32), 'hex'), decode(repeat('53', 32), 'hex'), 'praise', 'The new channel switcher feels immediate.', '["desktop", "navigation"]', now() - interval '1 day', now() - interval '1 day')
  ON CONFLICT (event_id) DO UPDATE SET
    category = EXCLUDED.category,
    body = EXCLUDED.body,
    tags = EXCLUDED.tags,
    event_created_at = EXCLUDED.event_created_at,
    received_at = EXCLUDED.received_at;
END $$;
SQL

if command -v psql >/dev/null 2>&1; then
  PGPASSWORD="${PGPASSWORD}" psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d "${PGDATABASE}" -v ON_ERROR_STOP=1 -c "${sql}"
elif docker exec buzz-postgres psql --version >/dev/null 2>&1; then
  docker exec -i -e PGPASSWORD="${PGPASSWORD}" buzz-postgres \
    psql -U "${PGUSER}" -d "${PGDATABASE}" -v ON_ERROR_STOP=1 -c "${sql}"
else
  echo "error: neither psql nor buzz-postgres docker psql is available" >&2
  exit 1
fi

echo "Seeded 4 moderation reports and 3 feedback entries for the local admin dashboard."
