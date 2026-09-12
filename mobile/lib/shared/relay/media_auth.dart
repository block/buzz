import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:http/http.dart' as http;

import '../auth/event_signer.dart';
import '../auth/enterprise_identity.dart';

import 'relay_provider.dart';

const _mediaGetAuthKind = 24242;
const _mediaGetAuthLifetimeSeconds = 600;
// Managed signer policy caps all bearer media proofs at 300 seconds.
const _managedMediaGetAuthLifetimeSeconds = 120;
const _managedMediaGetAuthRefreshMarginSeconds = 10;

/// Re-sign this long before the cached auth event expires, so an in-flight
/// request signed just before the boundary still lands well within validity.
const _mediaGetAuthRefreshMarginSeconds = 60;

/// Builds BUD-01 Blossom `t=get` auth headers for relay-host media URLs.
///
/// Returns an empty map for non-relay URLs or when no signing key is available,
/// so callers can safely use this on arbitrary profile/custom-emoji URLs without
/// leaking Buzz credentials to third-party hosts.
///
/// The signed header is memoized until the refresh margin before expiry
/// (managed: 120s lifetime/10s margin; OSS: 600s/60s): repeated calls return the byte-identical map instead of
/// producing a fresh Schnorr signature per widget build. The service itself is
/// rebuilt (dropping the memo) whenever the relay config — base URL or signing
/// identity — changes, via [mediaGetAuthServiceProvider].
class MediaGetAuthService {
  final String _baseUrl;
  final String? _nsec;
  final EventSigner? _signer;
  bool _disposed = false;
  final DateTime Function() _now;

  Map<String, String>? _cachedHeaders;
  DateTime? _refreshAt;

  MediaGetAuthService({
    required String baseUrl,
    required String? nsec,
    EventSigner? signer,
    DateTime Function()? now,
  }) : _baseUrl = baseUrl,
       _nsec = nsec,
       _signer = signer,
       _now = now ?? DateTime.now;

  /// Remote credentials must not be handed to redirect-following native players.
  bool get isRemote => _signer is RemoteEventSigner;

  int get _lifetimeSeconds => isRemote
      ? _managedMediaGetAuthLifetimeSeconds
      : _mediaGetAuthLifetimeSeconds;
  int get _refreshMarginSeconds => isRemote
      ? _managedMediaGetAuthRefreshMarginSeconds
      : _mediaGetAuthRefreshMarginSeconds;

  /// Fetch through a transport which cannot forward proof through a redirect.
  Future<http.Response> get(http.Client client, String url) async {
    final headers = await headersFor(url);
    checkCurrent();
    final response = await getMediaWithoutRedirects(
      client,
      Uri.parse(url),
      headers,
    );
    checkCurrent();
    return response;
  }

  /// Invalidate cached proof when its provider/community is retired.
  void dispose() {
    _disposed = true;
    _cachedHeaders = null;
  }

  /// Check immediately before and after a transport await.
  void checkCurrent() {
    if (_disposed) throw StateError('Media identity scope changed');
    if (_signer case final RemoteEventSigner remote) remote.checkCurrent();
  }

  bool isRelayMediaUrl(String url) {
    // Uri normalizes an empty userinfo marker away; reject it before parsing.
    final userinfo = RegExp(r'^[a-zA-Z][a-zA-Z0-9+.-]*://[^/?#]*@');
    if (userinfo.hasMatch(url) || userinfo.hasMatch(_baseUrl)) return false;
    final uri = Uri.tryParse(url);
    final relayUri = Uri.tryParse(_baseUrl);
    if (uri == null || relayUri == null) return false;
    return _isRelayMediaUrl(uri, relayUri);
  }

  Future<Map<String, String>>? _pendingHeaders;

  Future<Map<String, String>> headersFor(String url) async {
    final nsec = _nsec;
    checkCurrent();
    if (_signer == null && (nsec == null || nsec.isEmpty)) return const {};
    if (!isRelayMediaUrl(url)) return const {};

    final cached = _cachedHeaders;
    final refreshAt = _refreshAt;
    if (cached != null && refreshAt != null && _now().isBefore(refreshAt)) {
      return cached;
    }

    final pending = _pendingHeaders;
    if (pending != null) {
      // The synchronous path completed each refresh before the next call.
      // Recheck the cache after waiting: failures must not be shared/cached,
      // and the existing expiry boundary still applies to every caller.
      await pending;
      return headersFor(url);
    }
    return _pendingHeaders = _refreshHeaders();
  }

  Future<Map<String, String>> _refreshHeaders() async {
    try {
      final signedAt = _now();
      final authEvent = await _buildGetAuthEvent(signedAt);
      checkCurrent();
      final encoded = base64Url
          .encode(utf8.encode(authEvent.toJson()))
          .replaceAll('=', '');
      final headers = Map<String, String>.unmodifiable({
        'Authorization': 'Nostr $encoded',
      });
      _cachedHeaders = headers;
      _refreshAt = signedAt.add(
        Duration(seconds: _lifetimeSeconds - _refreshMarginSeconds),
      );
      return headers;
    } catch (_) {
      if (_signer is RemoteEventSigner || _disposed) rethrow;
      // Read auth is best-effort: while the relay rollout flag is off, an
      // unsigned fetch still works. Once the flag is on, this request will 403
      // instead of crashing the widget tree because local key material is bad.
      return const {};
    } finally {
      _pendingHeaders = null;
    }
  }

  bool _isRelayMediaUrl(Uri uri, Uri relayUri) {
    if (uri.userInfo.isNotEmpty || relayUri.userInfo.isNotEmpty) return false;
    if (_signer case final RemoteEventSigner remote) {
      if (relayUri != Uri.parse(remote.relayUrl)) return false;
      return uri.scheme == 'https' &&
          relayUri.scheme == 'https' &&
          uri.host == relayUri.host &&
          uri.port == relayUri.port &&
          uri.path.startsWith('/media/');
    }
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

  Future<nostr.Event> _buildGetAuthEvent(DateTime signedAt) async {
    final signer =
        _signer ?? LocalEventSigner(nostr.Nip19.decode(payload: _nsec!).data);

    final expiration =
        (signedAt.millisecondsSinceEpoch ~/ 1000) + _lifetimeSeconds;
    final tags = <List<String>>[
      ['t', 'get'],
      ['expiration', '$expiration'],
      if (extractServerAuthority(_baseUrl) case final authority?)
        ['server', authority],
    ];

    return signEvent(
      kind: _mediaGetAuthKind,
      createdAt: signedAt.millisecondsSinceEpoch ~/ 1000,
      content: 'Get buzz-media',
      tags: tags,
      signer: signer,
    );
  }
}

final mediaGetAuthServiceProvider = Provider<MediaGetAuthService>((ref) {
  final config = ref.watch(relayConfigProvider);
  final service = MediaGetAuthService(
    baseUrl: config.baseUrl,
    nsec: config.nsec,
    signer: config.signer,
  );
  ref.onDispose(service.dispose);
  return service;
});

Future<Map<String, String>> mediaGetHeadersFor(WidgetRef ref, String url) {
  return ref.read(mediaGetAuthServiceProvider).headersFor(url);
}

Future<Map<String, String>> mediaGetHeadersForContext(
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

/// The actual HTTP seam, shared by all credential-bearing media downloads.
Future<http.Response> getMediaWithoutRedirects(
  http.Client client,
  Uri uri,
  Map<String, String> headers,
) async {
  final request = http.Request('GET', uri)..followRedirects = false;
  request.headers.addAll(headers);
  return http.Response.fromStream(await client.send(request));
}
