import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:nostr/nostr.dart' as nostr;

import 'relay_provider.dart';

const _mediaGetAuthKind = 24242;
const _mediaGetAuthLifetimeSeconds = 600;

/// Re-sign this long before the cached auth event expires, so an in-flight
/// request signed just before the boundary still lands well within validity.
const _mediaGetAuthRefreshMarginSeconds = 60;

class MediaRequestTarget {
  final Uri uri;
  final Map<String, String> headers;

  const MediaRequestTarget({required this.uri, required this.headers});
}

Future<http.Response> fetchMediaResponse(
  http.Client client,
  List<MediaRequestTarget> targets,
) async {
  if (targets.isEmpty) {
    throw const FormatException('No valid media request target');
  }

  Object? lastError;
  for (var index = 0; index < targets.length; index++) {
    final target = targets[index];
    final hasFallback = index + 1 < targets.length;
    try {
      final response = await client.get(target.uri, headers: target.headers);
      if (kDebugMode) {
        debugPrint(
          'Buzz media GET ${target.uri} '
          'host=${target.headers[HttpHeaders.hostHeader] ?? target.uri.authority} '
          'auth=${target.headers.keys.any((key) => key.toLowerCase() == HttpHeaders.authorizationHeader)} '
          'status=${response.statusCode}',
        );
      }
      if (response.statusCode >= 200 && response.statusCode < 300) {
        return response;
      }
      if (hasFallback &&
          (response.statusCode == HttpStatus.notFound ||
              response.statusCode >= 500)) {
        lastError = HttpException(
          'Media request failed (${response.statusCode})',
          uri: target.uri,
        );
        continue;
      }
      return response;
    } catch (error) {
      if (kDebugMode) {
        debugPrint('Buzz media GET ${target.uri} failed: $error');
      }
      if (!hasFallback) rethrow;
      lastError = error;
    }
  }

  throw lastError ?? StateError('No relay media transport available');
}

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
  final String? _lanRelayUrl;
  final String? _nsec;
  final DateTime Function() _now;

  Map<String, String>? _cachedHeaders;
  DateTime? _refreshAt;

  MediaGetAuthService({
    required String baseUrl,
    String? lanRelayUrl,
    required String? nsec,
    DateTime Function()? now,
  }) : _baseUrl = baseUrl,
       _lanRelayUrl = lanRelayUrl,
       _nsec = nsec,
       _now = now ?? DateTime.now;

  bool isRelayMediaUrl(String url) {
    final uri = Uri.tryParse(url);
    if (uri == null) return false;
    return _isRelayMediaUrl(uri);
  }

  Map<String, String> headersFor(String url) {
    final nsec = _nsec;
    if (nsec == null || nsec.isEmpty) return const {};
    if (!isRelayMediaUrl(url)) return const {};

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

  List<MediaRequestTarget> requestTargetsFor(String url) {
    final uri = Uri.tryParse(url);
    if (uri == null || !uri.hasScheme) return const [];
    final canonicalHeaders = headersFor(url);
    if (!isRelayMediaUrl(url)) {
      return [MediaRequestTarget(uri: uri, headers: canonicalHeaders)];
    }

    final canonicalUri = _canonicalMediaUri(uri);
    final canonicalTarget = MediaRequestTarget(
      uri: canonicalUri,
      headers: canonicalHeaders,
    );
    final lanUri = _lanMediaUri(uri);
    if (lanUri == null || lanUri == uri) return [canonicalTarget];
    final lanHeaders = Map<String, String>.unmodifiable({
      ...canonicalHeaders,
      HttpHeaders.hostHeader: Uri.parse(_baseUrl).authority,
    });
    return [
      MediaRequestTarget(uri: lanUri, headers: lanHeaders),
      canonicalTarget,
    ];
  }

  Uri? _lanMediaUri(Uri mediaUri) {
    final relay = Uri.tryParse(_lanRelayUrl ?? '');
    if (relay == null || relay.host.isEmpty) return null;
    if (!const {'ws', 'wss', 'http', 'https'}.contains(relay.scheme)) {
      return null;
    }
    return Uri(
      scheme: 'http',
      host: relay.host,
      port: relay.hasPort ? relay.port : null,
      path: mediaUri.path,
      query: mediaUri.hasQuery ? mediaUri.query : null,
    );
  }

  Uri _canonicalMediaUri(Uri mediaUri) {
    final relay = Uri.parse(_baseUrl);
    return Uri(
      scheme: relay.scheme,
      host: relay.host,
      port: relay.hasPort ? relay.port : null,
      path: mediaUri.path,
      query: mediaUri.hasQuery ? mediaUri.query : null,
    );
  }

  bool _isRelayMediaUrl(Uri uri) {
    if (uri.scheme != 'http' && uri.scheme != 'https') return false;
    if (uri.host.isEmpty || !uri.path.startsWith('/media/')) return false;
    // Extract the URL's origin and path. Query strings are ignored for media
    // host/path detection, matching the fetch target shape used by descriptors.
    final base = '${uri.scheme}://${uri.authority}';
    final mediaAuthority = extractServerAuthority(base);
    if (mediaAuthority == null) return false;
    final relayAuthorities = [
      extractServerAuthority(_baseUrl),
      if (_lanRelayUrl != null) extractServerAuthority(_lanRelayUrl),
    ].whereType<String>();
    return relayAuthorities.any(
      (authority) => mediaAuthority.toLowerCase() == authority.toLowerCase(),
    );
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
    lanRelayUrl: config.lanRelayUrl,
    nsec: config.nsec,
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
