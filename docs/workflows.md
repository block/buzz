# Calendar workflow schedules

Use `calendar` for an anchored every-other-week schedule. Existing `cron` schedules stay in UTC and `interval` schedules measure elapsed time. Calendar recurrence does not change either.

```yaml
name: Fortnightly agenda
enabled: false
trigger:
  on: schedule
  calendar:
    every_weeks: 2
    start_date: "2027-01-06"
    time: "13:35"
    timezone: America/New_York
steps:
  - id: remind
    action: send_message
    text: Review the agenda.
```

Choose exactly one of `calendar`, `cron`, or `interval`. All four calendar fields are required. Only `every_weeks: 2` is supported. The first local date sets both the weekday and phase. Later dates are 14 calendar days apart, including across month, year, and daylight-saving boundaries. Past anchors do not replay history or shift phase after a save, re-enable, or restart.

Dates must be real `YYYY-MM-DD` values and times must use `HH:MM`. The relay pins `chrono-tz` 0.10.4 and accepts its case-sensitive IANA names, including `UTC`, aliases such as `US/Eastern`, and fixed-offset names such as `Etc/GMT+5`. Names are preserved. Bare offsets such as `+05:00` are not names. Calendar schedules reject unknown fields, including an outer `trigger.timezone`. Legacy cron/interval unknown-key handling is unchanged.

If a wall time does not exist during a clock change, that occurrence is skipped. This can leave 28 calendar days between runs. Repeated wall times run once at the earlier UTC instant. A skipped date never changes the next eligible date. UTC calendar schedules do not move with daylight saving.

## Execution limits

The scheduler admits occurrences in `(tick time − 60 seconds, tick time]`, then uses the existing owner-authority check and durable `(community_id, workflow_id, scheduled_for)` claim. Competing pods use the same UTC occurrence. Claims prevent repeated admission, not exactly-once side effects or guaranteed delivery. A failed run insert consumes the claim. `trigger_timestamp` is still the observed tick time, not the scheduled occurrence.

There is no backlog replay. An outage, restart, or delayed scan can miss a run. The loop sleeps after each scan, so scanning time can create admission gaps. The scan selects only the oldest 1,000 active, database-enabled schedule rows in non-archived communities, without pagination. Definition-disabled drafts still consume scan slots when their database row is enabled. Calendar recurrence does not repair these existing scheduler limits.

## Rollout and compatibility

Deploy compatible relay binaries and timezone data to every serving and scheduler pod before enabling calendar workflows or releasing editor controls that users can rely on. Do not enable calendar workflows during a mixed-version rollout. This document does not establish live fleet versions or scan capacity.

Old relays reject a calendar-only signed save because neither known schedule arm is present. They do not reinterpret it as weekly cron. Never add a fallback cron to bypass that rejection. Old schedulers skip persisted calendar definitions. Rollback therefore leaves them dormant. A rejected name or enable edit must retain the saved definition. Editors without calendar controls should retain the definition in YAML. The CLI's existing YAML save path needs no new subcommand.

Before rollout, measure the scan population across all communities using the exact database-enabled/status/community filters above, including definition-disabled drafts. Separately count saved calendar definitions, configured-enabled calendar definitions, and their positions in that population. Counting only definition-enabled workflows understates the scan load. Deployment, rollback, historical-client acceptance, and real workflow activation need separate validation and authorization.

## Focused offline checks

Run `cargo test -p buzz-workflow --lib` for schema, recurrence, DST, and legacy schedule checks. With a disposable Postgres cluster, `scripts/postgres-test-run.sh -p buzz-workflow --lib` exercises actual claims, runs, authority, restarts, and a fake action sink. Run the relay's `command_executor::postgres_tests::calendar_definition` test for signed create/name/enable edits and invalid-update preservation. Never point these tests at an existing community database.

The shared fixtures live in `crates/buzz-workflow/tests/fixtures/`. `calendar-timezones.json` is the sorted `chrono_tz::TZ_VARIANTS` name table, checked by the relay test and mirrored by the modern editor. Update both copies together when changing timezone data. The editor does not compute authoritative next-run previews from browser timezone data.
