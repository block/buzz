import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import 'relay_provider.dart';

const _mediaGetAuthKind = 24242;
const _mediaGetAuthLifetimeSeconds = 600;

/// Re-sign this long before the cached auth event expires, so an in-flight
/// request signed just before the boundary still lands well within validity.
const _mediaGetAuthRefreshMarginSeconds = 60;

/// Builds BUD-01 Blossom `t=get` auth headers for relay-host media URLs.
///
/// Returns an empty map for non-relay URLs or when no signing key is available,
/// so callers can safely use this on arbitrary profile/custom-emoji URLs without
/// leaking Buzz credentials to third-party hosts.
///
/// The signed header is memoized until [_mediaGetAuthRefreshMarginSeconds]
/// before expiry: repeated calls return the byte-identical map instead of
/// producing a fresh Schnorr signature per widget build. The service itself is
/// rebuilt (dropping the memo) whenever the relay config — base URL or signing
/// identity — changes, via [mediaGetAuthServiceProvider].
class MediaGetAuthService {
  final String _baseUrl;
  final String? _nsec;
  final DateTime Function() _now;

  Map<String, String>? _cachedHeaders;
  DateTime? _refreshAt;

  MediaGetAuthService({
    required String baseUrl,
    required String? nsec,
    DateTime Function()? now,
  }) : _baseUrl = baseUrl,
       _nsec = nsec,
       _now = now ?? DateTime.now;

  Map<String, String> headersFor(String url) {
    final nsec = _nsec;
    if (nsec == null || nsec.isEmpty) return const {};
    final uri = Uri.tryParse(url);
    final relayUri = Uri.tryParse(_baseUrl);
    if (uri == null || relayUri == null) return const {};
    if (!_isRelayMediaUrl(uri, relayUri)) return const {};

    final cached = _cachedHeaders;
    final refreshAt = _refreshAt;
    if (cached != null && refreshAt != null && _now().isBefore(refreshAt)) {
      return cached;
    }

    try {
      final signedAt = _now();
      final authEvent = _buildGetAuthEvent(nsec);
      final encoded = base64Url
          .encode(utf8.encode(authEvent.toJson()))
          .replaceAll('=', '');
      final headers = Map<String, String>.unmodifiable({
        'Authorization': 'Nostr $encoded',
      });
      _cachedHeaders = headers;
      _refreshAt = signedAt.add(
        const Duration(
          seconds:
              _mediaGetAuthLifetimeSeconds - _mediaGetAuthRefreshMarginSeconds,
        ),
      );
      return headers;
    } catch (_) {
      // Read auth is best-effort: while the relay rollout flag is off, an
      // unsigned fetch still works. Once the flag is on, this request will 403
      // instead of crashing the widget tree because local key material is bad.
      return const {};
    }
  }

  bool _isRelayMediaUrl(Uri uri, Uri relayUri) {
    if (uri.scheme != 'http' && uri.scheme != 'https') return false;
    if (uri.host.isEmpty || relayUri.host.isEmpty) return false;
    // Extract the URL's origin and path. Query strings are ignored for media
    // host/path detection, matching the fetch target shape used by descriptors.
    final base = '${uri.scheme}://${uri.authority}';
    final mediaAuthority = extractServerAuthority(base);
    final relayAuthority = extractServerAuthority(_baseUrl);
    if (mediaAuthority == null || relayAuthority == null) return false;
    if (mediaAuthority.toLowerCase() != relayAuthority.toLowerCase()) {
      return false;
    }
    return uri.path.startsWith('/media/');
  }

  nostr.Event _buildGetAuthEvent(String nsec) {
    final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
    if (privkeyHex.isEmpty) {
      throw Exception('Invalid nsec');
    }

    final expiration =
        (_now().millisecondsSinceEpoch ~/ 1000) + _mediaGetAuthLifetimeSeconds;
    final tags = <List<String>>[
      ['t', 'get'],
      ['expiration', '$expiration'],
      if (extractServerAuthority(_baseUrl) case final authority?)
        ['server', authority],
    ];

    return nostr.Event.from(
      kind: _mediaGetAuthKind,
      content: 'Get buzz-media',
      tags: tags,
      secretKey: privkeyHex,
      verify: false,
    );
  }
}

final mediaGetAuthServiceProvider = Provider<MediaGetAuthService>((ref) {
  final config = ref.watch(relayConfigProvider);
  return MediaGetAuthService(baseUrl: config.baseUrl, nsec: config.nsec);
});

Map<String, String> mediaGetHeadersFor(WidgetRef ref, String url) {
  return ref.read(mediaGetAuthServiceProvider).headersFor(url);
}

Map<String, String> mediaGetHeadersForContext(
  BuildContext context,
  String url,
) {
  final container = ProviderScope.containerOf(context, listen: false);
  return container.read(mediaGetAuthServiceProvider).headersFor(url);
}

String? extractServerAuthority(String baseUrl) {
  final uri = Uri.parse(baseUrl);
  if (uri.host.isEmpty) return null;
  final host = uri.host.contains(':') ? '[${uri.host}]' : uri.host;
  final port = uri.hasPort ? uri.port : null;
  final authority = port == null ? host : '$host:$port';
  return _normalizeAuthority(authority);
}

String _normalizeAuthority(String authority) {
  var normalized = authority.trim().toLowerCase();
  if (normalized.endsWith('.')) {
    normalized = normalized.substring(0, normalized.length - 1);
  }
  if (normalized.endsWith(':443')) {
    return normalized.substring(0, normalized.length - ':443'.length);
  }
  if (normalized.endsWith(':80')) {
    return normalized.substring(0, normalized.length - ':80'.length);
  }
  return normalized;
}
