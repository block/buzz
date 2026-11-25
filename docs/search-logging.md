# Search logging

The relay writes one JSON line per message search to stdout, on the
`buzz_relay::search` tracing target. The line shows what was searched for and
what came back, but never the search text. Lines are the same whichever
backend answers, Postgres or OpenSearch, so you can compare the two.

A search is one NIP-50 filter that the relay sends to the backend, over a
WebSocket `REQ` or `POST /query`. A request with two search filters writes two
lines. A filter the relay answers without asking the backend writes none: for
example, one whose channels are all out of the searcher's reach. Profile name
search (`POST /profiles/search`) is not logged here.

## Fields

Every line has `event="search"` and these fields:

| Field | Meaning |
| --- | --- |
| `search_id` | A random UUID for this search. |
| `transport` | `ws` or `http`. |
| `community`, `searcher` | The community id and the searcher's hex pubkey. |
| `backend` | `postgres` or `opensearch`. |
| `mode`, `sort` | `full_text` or `prefix`; `relevance` or `newest`. |
| `page`, `limit` | The requested page (HTTP only; WebSocket is always 1) and result limit. |
| `kinds`, `kind_count` | Up to 10 kinds, comma-separated, and how many the filter named. |
| `author_count` | How many authors the filter named. |
| `since`, `until` | The filter's time bounds in Unix seconds, when set. |
| `channel_scope`, `channel_count` | `any`, `channel_less`, `channels` or `channels_or_channel_less`, and how many channels were searched. |
| `word_count` | Whitespace-separated words in the search text. |
| `quoted` | Whether the text contains `"`. |
| `has_url` | Whether a word contains `://` or starts with `www.`. |
| `outcome`, `reason` | `served`, `error` or `aborted`; see below. |
| `backend_hits` | Hits the backend returned, before access checks. |
| `result_count` | Events the searcher received, after access checks. |
| `hit_ids` | The first 100 of those event ids, comma-separated, in the order sent. |
| `duration_ms` | Time from the first backend query to the end of this search's own work. |

`outcome` says whether the searcher got the results:

- `served`: they were sent.
- `error`: the relay failed, and `reason` says where: `search_failed` (the
  backend), `query_timed_out` or `database_error` (loading the hits), or, on
  `POST /query`, `other_filter_failed` (another search filter in the same
  request failed, so the response was an error).
- `aborted`: the search ended before its results were sent. The client went
  away, or a newer search replaced it: a superseded `POST /query` typeahead
  search, or a WebSocket subscription the client replaced or closed.

The line is built only while the target is on. It is written after the
search ends, so logging never fails a search or delays its results.

## What you can measure

Use lines with `outcome="served"` and `page=1`, grouped by `backend`. Later
pages are the same search read further, not new searches.

- **Zero-result rate:** the share of lines with `result_count=0`.
- **Repeated searches:** the share of lines followed by another line from the
  same `searcher` in the same `community` within 30 seconds. Compare like with
  like by `mode`: typeahead (`prefix`) searches again on every typing pause.

A request with several search filters writes one line per filter, all within
milliseconds, with the same `searcher` and `transport`. Count them as one
search, or they read as repeats. Such a request also sends each event once:
an event that two of its filters match counts toward the first, so the
second can show `result_count=0` though it matched.

## Turning it off

Add `buzz_relay::search=off` to `RUST_LOG`, for example
`RUST_LOG=buzz_relay=info,buzz_relay::search=off`. That also turns off the
search text lines below.

Search lines are left out of the OpenTelemetry export by default. If you set
`BUZZ_OTEL_FILTER`, keep `buzz_relay::search=off` in it unless you mean to
export them. Search text lines are never exported, whatever it says.

## Search text

Search text can be as sensitive as the messages it searches, DMs included,
so the relay logs none by default. To log it, set:

| Variable | Meaning |
| --- | --- |
| `BUZZ_SEARCH_LOG_QUERY_TEXT_RETENTION_DAYS` | Whole days to keep search text, at least 1. Unset or empty logs no text. |

With it set, each search also writes a line on `buzz_relay::search::text`
with `event="search_text"`, the same `search_id`, `retention_days` (the
setting) and `query`, which holds the first 1024 characters of the text. Join
it to its search line by `search_id`.

The relay can't delete what it has written to stdout, so your log pipeline
enforces retention. Before you turn this on:

1. Route the `buzz_relay::search::text` target to a store that deletes lines
   after `retention_days`, and keep it out of your general log store.
2. Limit who can read that store, as you would the messages themselves.
3. Keep the target out of any other sink that keeps lines longer. The relay
   already keeps it out of OpenTelemetry.
