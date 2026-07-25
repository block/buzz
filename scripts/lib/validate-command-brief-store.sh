#!/usr/bin/env bash

COMMAND_BRIEF_SCHEMA_V4_SHA256="6f111e78cf9da58d0041e8c2909955ba7cacd7e343487efb6e253e7cf5314088"

command_brief_store_schema_digest() {
  local store_file="$1"
  sqlite3 "$store_file" \
    "SELECT type || ':' || name || ':' ||
            lower(replace(replace(replace(replace(replace(replace(
              sql,char(10),''),char(13),''),char(9),''),' ',''),'\"',''),';',''))
     FROM sqlite_master
     WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'
     ORDER BY type,name;" |
    shasum -a 256 | awk '{print $1}'
}

command_brief_table_signature() {
  local store_file="$1"
  local table="$2"
  sqlite3 "$store_file" \
    "SELECT group_concat(
       name || ':' || type || ':' || \"notnull\" || ':' ||
       ifnull(dflt_value,'') || ':' || pk || ':' || hidden, ',')
     FROM pragma_table_xinfo('${table}');"
}

validate_command_brief_store() {
  local store_file="$1"
  local timezone_catalog="${2:-}"
  local expected_spool
  local expected_heads
  local expected_schedule
  local expected_claims
  local timezone

  expected_spool="owner_pubkey:TEXT:1::1:0,run_id:TEXT:1::2:0,event_id:TEXT:1::3:0,status:TEXT:1::0:0,previous_event_id:TEXT:0::0:0,encrypted_payload:TEXT:1::0:0,raw_event:TEXT:1::0:0,publish_state:TEXT:1::0:0,retry_count:INTEGER:1:0:0:0,next_retry_at:INTEGER:1:0:0:0,last_error_code:TEXT:0::0:0,created_at:INTEGER:1::0:0,append_sequence:INTEGER:1::0:0,published_at:INTEGER:0::0:0"
  expected_heads="owner_pubkey:TEXT:1::1:0,run_id:TEXT:1::2:0,head_event_id:TEXT:1::0:0,head_sequence:INTEGER:1::0:0"
  expected_schedule="schedule_id:TEXT:0::1:0,classification:TEXT:1::0:0,enabled:INTEGER:1::0:0,local_time:TEXT:1::0:0,timezone:TEXT:1::0:0,catch_up_same_day:INTEGER:1::0:0,concurrency:INTEGER:1::0:0,updated_at:INTEGER:1::0:0"
  expected_claims="idempotency_key:TEXT:0::1:0,schedule_id:TEXT:1::0:0,local_date:TEXT:1::0:0,timezone:TEXT:1::0:0,state:TEXT:1::0:0,deferred_reason:TEXT:0::0:0,retry_count:INTEGER:1:0:0:0,transition_token:TEXT:0::0:0,claimed_at:INTEGER:1::0:0,updated_at:INTEGER:1::0:0,run_id:TEXT:1::0:0"

  [[ -f "$store_file" && ! -L "$store_file" ]] ||
    local_workspace_die "command brief store must be a regular file" || return
  [[ "$(sqlite3 "$store_file" 'PRAGMA integrity_check;')" == "ok" ]] ||
    local_workspace_die "command brief store failed integrity validation" || return
  [[ "$(sqlite3 "$store_file" 'PRAGMA user_version;')" == "4" ]] ||
    local_workspace_die "command brief store schema version is not current" || return
  [[ "$(command_brief_store_schema_digest "$store_file")" == \
    "$COMMAND_BRIEF_SCHEMA_V4_SHA256" ]] ||
    local_workspace_die "command brief store schema is not exact v4" || return
  [[ "$(command_brief_table_signature "$store_file" command_brief_spool)" == \
    "$expected_spool" ]] ||
    local_workspace_die "command brief spool columns are not exact v4" || return
  [[ "$(command_brief_table_signature "$store_file" command_brief_heads)" == \
    "$expected_heads" ]] ||
    local_workspace_die "command brief heads columns are not exact v4" || return
  [[ "$(command_brief_table_signature "$store_file" command_brief_schedule)" == \
    "$expected_schedule" ]] ||
    local_workspace_die "command brief schedule columns are not exact v4" || return
  [[ "$(command_brief_table_signature \
    "$store_file" command_brief_schedule_claims)" == "$expected_claims" ]] ||
    local_workspace_die "command brief claim columns are not exact v4" || return

  [[ "$(sqlite3 "$store_file" \
    "SELECT COUNT(*) FROM command_brief_schedule
     WHERE classification <> 'OFFICIAL'
        OR schedule_id <> 'daily-command-brief'
        OR enabled NOT IN (0,1)
        OR local_time NOT GLOB '[0-2][0-9]:[0-5][0-9]'
        OR CAST(substr(local_time,1,2) AS INTEGER) > 23
        OR catch_up_same_day NOT IN (0,1)
        OR concurrency NOT IN (1,2);")" == "0" ]] ||
    local_workspace_die "command brief schedule validation failed" || return
  [[ "$(sqlite3 "$store_file" \
    "SELECT COUNT(*) FROM command_brief_schedule_claims
     WHERE idempotency_key <> schedule_id || ':' || local_date
        OR retry_count NOT BETWEEN 0 AND 8
        OR length(local_date) <> 10
        OR date(local_date) <> local_date
        OR claimed_at > updated_at
        OR state NOT IN ('claimed','deferred','started','completed')
        OR (state = 'deferred' AND
            (deferred_reason NOT IN
               ('identity_locked','model_unavailable','local_state_unavailable')
             OR transition_token IS NULL
             OR length(CAST(transition_token AS BLOB)) NOT BETWEEN 1 AND 256
             OR instr(transition_token,char(0)) <> 0
             OR transition_token GLOB (
               '*[' ||
               char(1) || char(2) || char(3) || char(4) || char(5) ||
               char(6) || char(7) || char(8) || char(9) || char(10) ||
               char(11) || char(12) || char(13) || char(14) || char(15) ||
               char(16) || char(17) || char(18) || char(19) || char(20) ||
               char(21) || char(22) || char(23) || char(24) || char(25) ||
               char(26) || char(27) || char(28) || char(29) || char(30) ||
               char(31) || char(127) || ']*'
             )))
        OR (state <> 'deferred' AND
            (deferred_reason IS NOT NULL
             OR (transition_token IS NOT NULL AND
                 (length(CAST(transition_token AS BLOB)) NOT BETWEEN 1 AND 256
                  OR instr(transition_token,char(0)) <> 0
                  OR transition_token GLOB (
                    '*[' ||
                    char(1) || char(2) || char(3) || char(4) || char(5) ||
                    char(6) || char(7) || char(8) || char(9) || char(10) ||
                    char(11) || char(12) || char(13) || char(14) || char(15) ||
                    char(16) || char(17) || char(18) || char(19) || char(20) ||
                    char(21) || char(22) || char(23) || char(24) || char(25) ||
                    char(26) || char(27) || char(28) || char(29) || char(30) ||
                    char(31) || char(127) || ']*'
                  )))));")" == "0" ]] ||
    local_workspace_die "command brief claim validation failed" || return

  while IFS='|' read -r idempotency_key run_id; do
    local expected_run_id
    expected_run_id="scheduled-$(
      printf '%s' "$idempotency_key" | shasum -a 256 | awk '{print $1}'
    )"
    [[ "$run_id" == "$expected_run_id" ]] ||
      local_workspace_die "command brief deterministic run identity is invalid" ||
      return
  done < <(
    sqlite3 "$store_file" \
      "SELECT idempotency_key,run_id FROM command_brief_schedule_claims;"
  )

  [[ -f "$timezone_catalog" && ! -L "$timezone_catalog" ]] ||
    local_workspace_die "command brief timezone catalog is unavailable" || return
  while IFS= read -r timezone; do
    grep -Fqx -- "$timezone" "$timezone_catalog" ||
      local_workspace_die "command brief timezone is not a chrono_tz identifier" ||
      return
  done < <(
    sqlite3 "$store_file" \
      "SELECT DISTINCT timezone FROM command_brief_schedule_claims
       UNION SELECT DISTINCT timezone FROM command_brief_schedule;"
  )
}
