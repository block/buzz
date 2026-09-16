# Relay-proxied GIF search

Buzz relays can optionally provide GIF search without distributing a provider
credential to desktop clients. An operator configures `BUZZ_KLIPY_API_KEY` in
the relay's secret store. When present, the relay advertises this NIP-11 shape:

```json
{
  "supported_extensions": ["buzz-gif"],
  "gif": {
    "provider": "klipy",
    "search": "/gifs/search",
    "share": "/gifs/share"
  }
}
```

The descriptor is provider-agnostic so a relay can advertise another provider
or paths later. Clients must require `buzz-gif`, recognize the provider, and use
only safe relay-relative search and share paths.

`POST /gifs/search` requires NIP-98 authentication and relay membership. Its
per-pubkey, per-community Redis admission limit defaults to 30 requests per
minute and can be tuned with `BUZZ_RATE_LIMIT_GIF_SEARCHES_PER_MIN`. The relay
sends the provider credential upstream and returns only allowlisted successful
result data. Provider error bodies are never returned to clients or written to
logs.

`POST /gifs/share` uses the same authentication and membership boundary. It
accepts a bounded GIF slug plus anonymous customer ID and forwards KLIPY's share
signal so selected media can enter the user's provider-backed Recents. The
endpoint returns no provider body.

## Message and rendering boundary

Selecting a GIF sends a normal Buzz message containing a KLIPY CDN image URL.
Because Buzz's imeta validator permits only hash-verified local `/media/` paths,
external GIFs are deliberately content-only and carry no imeta tag. Buzz does
not download, cache, or store the GIF bytes. Existing image URL rendering
handles the message, including pasted GIF URLs on relays that do not advertise
`buzz-gif`. Only the picker is capability-gated.
