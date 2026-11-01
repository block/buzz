#!/usr/bin/env bash
# OUT-OF-BAND MAINTENANCE: renames listed channels that share a name in one
# community, so the contract migration's unique index can build.
#
# It reads the renames from plan_duplicate_listed_channel_renames.sql and sends
# each one to the relay as a kind:9002 rename, signed by a relay operator. The
# relay applies a rename the way it applies any other: it refuses a name that
# another listed channel has, and publishes the new kind:39000, so clients
# show the new name. Relays keep serving while it runs.
#
# Run it only after every serving relay enforces the name rule and accepts an
# operator rename. An older relay refuses the rename and can create new
# duplicates.
#
# A rename that fails is reported and skipped: the relay refused it (for
# example, because a channel took the new name meanwhile), or the CLI could
# not reach the relay. At the end the script reads the plan again and fails
# if any rename remains; run it again. If one rename fails on every run
# because another channel holds its new name, rename that channel by hand with
# `buzz channels update --channel=<id> --name=<new name>`. An auth refusal
# (CLI exit code 3) stops the script at once, because every other rename would
# fail the same way.
#
# Usage:
#   DATABASE_URL=postgres://... BUZZ_PRIVATE_KEY=<operator key> \
#     scripts/maintenance/rename_duplicate_listed_channel_names.sh [--dry-run] <community host>
#
# BUZZ_PRIVATE_KEY must belong to a relay operator (RELAY_OPERATOR_PUBKEYS, or
# an `operator` row in relay_operators), not a moderator. The key must also be
# a member of the community: POST /events checks community membership before
# it checks the operator role, so a key that is not a member gets 403
# relay_membership_required. Add it first with
# `RELAY_URL=wss://<community host> buzz-admin add-member --pubkey <npub>`.
# The relay serves only an active, unarchived community, so the script checks
# the community first. For an archived one, it stops (or, with --dry-run, warns):
# unarchive it with `buzz-admin communities unarchive`, run this script, then
# archive it again. For one being deleted, it stops: no rename can land there,
# and its duplicates go when its deletion completes.
# BUZZ_RELAY_URL defaults to wss://<community host>. BUZZ_CLI defaults to
# `buzz`.
set -euo pipefail

usage() {
    echo "usage: $0 [--dry-run] <community host>" >&2
    exit 2
}

dry_run=false
if [[ ${1:-} == --dry-run ]]; then
    dry_run=true
    shift
fi
[[ $# -eq 1 ]] || usage
host=$1
: "${DATABASE_URL:?set DATABASE_URL}"
if ! $dry_run; then
    : "${BUZZ_PRIVATE_KEY:?set BUZZ_PRIVATE_KEY to a relay operator key}"
fi
relay_url=${BUZZ_RELAY_URL:-wss://$host}
buzz=${BUZZ_CLI:-buzz}
plan_sql="$(cd "$(dirname "$0")" && pwd)/plan_duplicate_listed_channel_renames.sql"

plan=$(mktemp)
trap 'rm -f "$plan"' EXIT

# Every field ends with a NUL byte. Postgres text cannot hold one, so any
# name, with any control character in it, stays one field.
read_plan() {
    psql "$DATABASE_URL" -X -q -v ON_ERROR_STOP=1 -v host="$host" \
        -A -t -z -0 -f "$plan_sql" >"$plan"
}

community_state=$(psql "$DATABASE_URL" -X -q -v ON_ERROR_STOP=1 -v host="$host" \
    -A -t <<'SQL'
SELECT CASE
    WHEN deleted_at IS NOT NULL OR deletion_state <> 'active' THEN 'deleting'
    WHEN archived_at IS NOT NULL THEN 'archived'
    ELSE 'active'
END
FROM communities WHERE host = :'host';
SQL
)
case $community_state in
active) ;;
archived)
    echo "$host is archived, and the relay applies no rename there. Unarchive it" \
        "with \`buzz-admin communities unarchive\`, run this script, then archive" \
        "it again." >&2
    $dry_run || exit 1
    ;;
deleting)
    echo "$host is being deleted, and the relay applies no rename there. Its" \
        "duplicates go when its deletion completes." >&2
    exit 1
    ;;
*)
    echo "no community has host $host" >&2
    exit 1
    ;;
esac

read_plan
# The plan is the loop's stdin, so the CLI reads /dev/null instead.
renamed=0
failed=0
while IFS= read -r -d '' channel && IFS= read -r -d '' name &&
    IFS= read -r -d '' new_name; do
    # --name=value, so a name that starts with "-" is not read as a flag.
    if $dry_run; then
        printf 'would rename %s from %q to %q\n' "$channel" "$name" "$new_name"
        continue
    fi
    status=0
    BUZZ_RELAY_URL=$relay_url "$buzz" channels update \
        --channel="$channel" --name="$new_name" </dev/null >/dev/null || status=$?
    if [[ $status -eq 0 ]]; then
        printf 'renamed %s from %q to %q\n' "$channel" "$name" "$new_name"
        renamed=$((renamed + 1))
    elif [[ $status -eq 3 ]]; then
        echo "the relay refused the key for $host: it must be a relay operator" \
            "and a member of $host (renamed $renamed before this)" >&2
        exit 1
    else
        printf 'could not rename %s from %q to %q\n' \
            "$channel" "$name" "$new_name" >&2
        failed=$((failed + 1))
    fi
done <"$plan"

$dry_run && exit 0

read_plan
# Three fields, so three NUL bytes, for each rename.
remaining=$(($(tr -cd '\000' <"$plan" | wc -c) / 3))
echo "renamed $renamed, failed $failed, remaining $remaining"
if [[ $remaining -ne 0 ]]; then
    echo "duplicate listed channel names remain in $host; run the script again" >&2
    exit 1
fi
