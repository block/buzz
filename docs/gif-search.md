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

GIPHY's published API best practices
(https://developers.giphy.com/docs/api/best-practices/) list these prohibited
practices:

- "Do not proxy requests to GIPHY, either API calls or media URL loads. All
  requests to GIPHY should be made directly from the client side."
- "Do not cache API responses or media URLs or cache or store copies of GIPHY's
  media assets."

This route proxies the search and trending API calls through the relay on
purpose, so the API key never ships inside a desktop build. That is in direct
tension with the first rule. Media URL loads are not proxied: clients load
GIPHY CDN URLs directly, and the relay stores no API responses, media URLs or
media bytes.

Enabling `BUZZ_GIPHY_API_KEY` does not settle this tension. Before an operator
enables the route, they must obtain written confirmation from GIPHY that
server-side API calls are permitted for their key, or accept the risk that
GIPHY revokes the key. The picker shows "Powered by GIPHY" text; GIPHY's
attribution guidelines ask for its official marks, which Buzz does not ship.

## Upgrading from the KLIPY integration

Buzz relays before this change proxied KLIPY. The cutover is not transparent
to every client, so operators should read these notes before deploying.

- The environment variable is now `BUZZ_GIPHY_API_KEY`. `BUZZ_KLIPY_API_KEY`
  is ignored. A relay with only the old variable set stops advertising
  `buzz-gif` and returns 404 from `/gifs/search`. The Helm chart reads the new
  key name from the `buzz-secrets` secret.
- The NIP-11 `gif.provider` value changes from `klipy` to `giphy`, and the
  `gif.share` path is gone. The `POST /gifs/share` route and the `buzz gifs
  share` CLI command are removed.
- Clients gate the picker on the provider name. The deprecated Buzz 0.X desktop
  client in this repository and the `buzz` CLI accept `giphy`. The active Buzz
  1.0 client in block/buzz-app still requires `klipy`
  (`src/features/relay/gif-capability.ts`) and parses the KLIPY response shape
  (`src/features/relay/gifs.ts`), so its GIF picker disappears on a relay that
  has cut over. Deploy this relay change only after block/buzz-app ships GIPHY
  support, or accept that 1.0 users lose GIF search until it does.
- The search response is `{ "data": [GifObject, ...] }` with GIPHY's string
  dimensions. Any client that parsed the KLIPY `result` and `data.data` shape
  must be updated.
