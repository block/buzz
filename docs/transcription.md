# Relay-proxied transcription

Buzz relays can optionally transcribe short audio clips for dictation without
distributing a provider credential to clients. An operator sets
`BUZZ_TRANSCRIPTION_API_KEY` in the relay's secret store. When present, the
relay advertises this NIP-11 shape:

```json
{
  "supported_extensions": ["buzz-transcription"],
  "transcription": { "path": "/audio/transcriptions" }
}
```

The descriptor names no provider, URL or model. Clients must require
`buzz-transcription` and use only the relay-relative `path`. Without the key the
extension is absent and the route is not mounted (404).

| Variable | Default |
|----------|---------|
| `BUZZ_TRANSCRIPTION_API_KEY` | unset (feature off) |
| `BUZZ_TRANSCRIPTION_BASE_URL` | `https://api.openai.com/v1`; any OpenAI-compatible `http(s)` server |
| `BUZZ_TRANSCRIPTION_MODEL` | `gpt-4o-transcribe` |
| `BUZZ_RATE_LIMIT_TRANSCRIPTIONS_PER_MIN` | `10` per pubkey and community |

`POST /audio/transcriptions` takes raw M4A audio as the body with
`Content-Type: audio/mp4`. It requires NIP-98 authentication with a `payload`
tag over the body, plus the same NIP-FI admission and relay membership as the
other member HTTP endpoints. A request with no `Authorization: Nostr` header
is refused with 401 before its body is read. The relay accepts at most 5 MiB
(413 above that; the app checks the same limit before uploading),
about 7 minutes of the mobile app's 96 kbps AAC-LC; the app stops dictation at
5 minutes, about 3.5 MiB. At most 8 upstream calls run at once per relay
process (503 beyond that, without spending quota). The slots are relay-wide,
so while the provider is slow one busy member can briefly cause 503s for
others (the per-minute quota caps how many one member starts). A success
answers `{"text": "<transcript>"}` and nothing else from the provider.
Provider failures are a generic 502, or 504 after 60 seconds.

Audio and transcripts are never logged or stored. Upstream redirects are not
followed, because every request carries the provider key, and provider bodies
never reach clients or logs.
