import 'dart:convert';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;

/// Supplies the HTTP client used for NIP-11 community icon lookups.
///
/// Tests can override this provider to return deterministic relay responses.
final communityIconHttpClientProvider = Provider<http.Client>((ref) {
  final client = http.Client();
  ref.onDispose(client.close);
  return client;
});

/// Reads a community's icon from its public NIP-11 relay information document.
///
/// The lookup does not depend on the active relay session, so icons can render
/// for every paired community in the switcher. Callers explicitly invalidate
/// this family when opening the switcher so transient failures and relay
/// metadata updates can be retried without background polling.
final communityIconProvider = FutureProvider.autoDispose
    .family<String?, String>((ref, relayUrl) async {
      final uri = _relayInfoUri(relayUrl);
      if (uri == null) return null;

      try {
        final response = await ref
            .read(communityIconHttpClientProvider)
            .get(uri, headers: const {'Accept': 'application/nostr+json'})
            .timeout(const Duration(seconds: 5));
        if (response.statusCode < 200 || response.statusCode >= 300) {
          return null;
        }

        final document = jsonDecode(response.body);
        if (document is! Map<String, dynamic>) return null;

        final icon = document['icon'];
        if (icon is! String || icon.trim().isEmpty) return null;
        return icon.trim();
      } catch (_) {
        return null;
      }
    });

Uri? _relayInfoUri(String relayUrl) {
  try {
    final uri = Uri.parse(relayUrl.trim());
    final scheme = switch (uri.scheme) {
      'wss' => 'https',
      'ws' => 'http',
      'https' || 'http' => uri.scheme,
      _ => null,
    };
    return scheme == null ? null : uri.replace(scheme: scheme);
  } on FormatException {
    return null;
  }
}
