/// Joining relay HTTP endpoint URLs so they survive a base-path deployment.
///
/// `Uri.resolve('/events')` discards any path the base URL carries, which is
/// correct for an origin-rooted relay but silently wrong for one served under
/// `BUZZ_BASE_PATH` (`https://host/relay`): the request lands on `/events`
/// instead of `/relay/events`. Since the relay's NIP-98 check reconstructs the
/// prefixed URL, a dropped prefix surfaces as a 401 or a 404 rather than
/// anything that names the real cause.
library;

/// Join `path` onto `baseUrl`, preserving any path prefix the base carries.
///
/// `path` is treated as relative to the base regardless of whether it has a
/// leading slash, so callers can keep passing the route literals they declare
/// (`/events`, `/media/upload`).
///
/// ```dart
/// relayEndpoint('https://host', '/query');        // https://host/query
/// relayEndpoint('https://host/relay', '/query');  // https://host/relay/query
/// relayEndpoint('https://host/relay/', 'query');  // https://host/relay/query
/// ```
String relayEndpoint(String baseUrl, String path) {
  final base = baseUrl.replaceAll(RegExp(r'/+$'), '');
  final suffix = path.replaceAll(RegExp(r'^/+'), '');
  return suffix.isEmpty ? base : '$base/$suffix';
}

/// Whether `urlPath` addresses the relay's media route.
///
/// Matches `/media/` as a full path segment anywhere in the path rather than
/// only at the root, because a base-path deployment serves media at
/// `<prefix>/media/<hash>`. Requiring the whole segment still rejects
/// near-misses such as `/media-evil/<hash>`.
bool isRelayMediaPath(String urlPath) => urlPath.contains('/media/');
