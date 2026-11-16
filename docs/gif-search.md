# Relay-proxied GIF search

Buzz relays can optionally provide GIF search without distributing a provider
credential to desktop clients. An operator configures `BUZZ_GIPHY_API_KEY` in
the relay's secret store. When present, the relay advertises this NIP-11 shape:

```json
{
  "supported_extensions": ["buzz-gif"],
  "gif": {
    "provider": "giphy",
    "search": "/gifs/search"
  }
}
```

The descriptor is provider-agnostic so a relay can advertise another provider
or paths later. Clients must require `buzz-gif`, recognize the provider, and use
only a safe relay-relative search path.

`POST /gifs/search` requires NIP-98 authentication and relay membership. Its
per-pubkey, per-community Redis admission limit defaults to 30 requests per
minute and can be tuned with `BUZZ_RATE_LIMIT_GIF_SEARCHES_PER_MIN`. The request
body is `{ "query", "customer_id", "locale" }`. An empty `query` returns GIPHY
trending results. `query` is capped at 50 characters, which is GIPHY's limit.
The relay forwards `customer_id` unchanged and derives the two-letter GIPHY
`lang` value from the first subtag of `locale`.

The relay sends the provider credential upstream as the `api_key` query
parameter and returns only the GIPHY `data` array as `{ "data": [...] }`. The
`meta` and `pagination` objects, and any provider error body, are never
returned to clients or written to logs. The upstream client follows no
redirects, so a provider 3xx cannot replay the key-bearing URL to another host.

GIPHY has no share or Recents endpoint, so there is no share path in the
descriptor. GIPHY's optional analytics pingbacks are client-side GET requests
that carry no credential; Buzz does not send them today.

## Message and rendering boundary

Selecting a GIF sends a normal Buzz message containing a GIPHY CDN image URL.
Because Buzz's imeta validator permits only hash-verified local `/media/` paths,
external GIFs are deliberately content-only and carry no imeta tag. Buzz does
not download, cache, or store the GIF bytes. Existing image URL rendering
handles the message, including pasted GIF URLs on relays that do not advertise
`buzz-gif`. Only the picker is capability-gated.

## Provider terms

GIPHY's API documentation states that search and trending calls should be made
from the client side. Buzz proxies them through the relay on purpose, so the
API key never ships inside a desktop build. The operator who holds the GIPHY
key is responsible for confirming that this server-side use, and the
"Powered by GIPHY" attribution shown in the picker, satisfy GIPHY's current API
terms for their key tier.
