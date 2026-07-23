import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../client/client_headers.dart';
import 'relay_provider.dart';

const _mediaGetAuthKind = 24242;
const _mediaGetAuthLifetimeSeconds = 600;

/// Re-sign this long before the cached auth event expires, so an in-flight
/// request signed just before the boundary still lands well within validity.
const _mediaGetAuthRefreshMarginSeconds = 60;

/// Builds BUD-01 Blossom auth for relay-hosted media requests, alongside
/// outbound client identification headers.
///
/// The coarse `User-Agent` is returned for any valid remote HTTP(S) URL.
/// `Buzz-Client` and `Authorization` remain restricted to same-origin relay
/// media paths, so arbitrary profile and custom-emoji hosts receive neither
/// structured metadata nor credentials.
///
/// The signed header is memoized until [_mediaGetAuthRefreshMarginSeconds]
/// before expiry: repeated calls return the byte-identical map instead of
/// producing a fresh Schnorr signature per widget build. The service itself is
/// rebuilt (dropping the memo) whenever the relay config — base URL or signing
/// identity — changes, via [mediaGetAuthServiceProvider].
class MediaGetAuthService {
  final String _baseUrl;
  final String? _nsec;
  final ClientHeaders? _clientHeaders;
  final DateTime Function() _now;

  Map<String, String>? _cachedHeaders;
  DateTime? _refreshAt;

  MediaGetAuthService({
    required String baseUrl,
    required String? nsec,
    ClientHeaders? clientHeaders,
    DateTime Function()? now,
  }) : _baseUrl = baseUrl,
       _nsec = nsec,
       _clientHeaders = clientHeaders,
       _now = now ?? DateTime.now;

  Map<String, String> headersFor(String url) {
    final clientHeaders = _clientHeaders;
    final identificationHeaders = clientHeaders == null
        ? const <String, String>{}
        : clientHeadersForUrl(
            headers: clientHeaders,
            targetUrl: url,
            relayUrl: _baseUrl,
          );
    final uri = Uri.tryParse(url);
    final relayUri = Uri.tryParse(_baseUrl);
    if (uri == null) return const {};
    if (relayUri == null) return identificationHeaders;
    if (!_isRelayMediaUrl(uri, relayUri)) return identificationHeaders;

    final nsec = _nsec;
    if (nsec == null || nsec.isEmpty) return identificationHeaders;

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
        ...identificationHeaders,
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
      // Read auth is best-effort: preserve first-party identification headers
      // even when local signing material is invalid.
      return identificationHeaders;
    }
  }

  bool _isRelayMediaUrl(Uri uri, Uri relayUri) {
    if (uri.scheme != 'http' && uri.scheme != 'https') return false;
    if (uri.host.isEmpty || relayUri.host.isEmpty) return false;
    if (!_sameHttpOrigin(uri, relayUri)) return false;
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
  return MediaGetAuthService(
    baseUrl: config.baseUrl,
    nsec: config.nsec,
    clientHeaders: ref.watch(clientHeadersProvider),
  );
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

bool _sameHttpOrigin(Uri a, Uri b) =>
    a.scheme.toLowerCase() == b.scheme.toLowerCase() &&
    a.host.toLowerCase() == b.host.toLowerCase() &&
    _effectiveHttpPort(a) == _effectiveHttpPort(b);

int? _effectiveHttpPort(Uri uri) {
  if (uri.hasPort) return uri.port;
  return switch (uri.scheme.toLowerCase()) {
    'https' => 443,
    'http' => 80,
    _ => null,
  };
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
