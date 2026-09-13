import 'dart:convert';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;

import '../relay/nostr_models.dart';
import '../relay/relay.dart';

/// NIP-IA archived identity snapshot (`kind:13535`). Mirrors desktop's
/// `list_archived_identities` — only relay-signed snapshots from NIP-11 `self`
/// affect discovery filtering.
final archivedIdentitiesProvider = FutureProvider<Set<String>>((ref) async {
  final config = ref.watch(relayConfigProvider);
  final relaySelf = await _fetchRelaySelf(config.baseUrl);
  if (relaySelf == null) return const {};

  final events = await ref.read(relaySessionProvider.notifier).queryRelay([
    NostrFilters.archivedIdentities(relaySelf),
  ]);

  if (events.isEmpty) return const {};
  final snapshot = events.first;
  if (snapshot.pubkey.toLowerCase() != relaySelf) return const {};

  return _archivedPubkeysFromSnapshot(snapshot);
});

/// Fail-open archived predicate for mention autocomplete. Returns `false` while
/// the snapshot loads so a cold start cannot briefly hide everyone. The current
/// user is never filtered, matching desktop's `useIsArchivedPredicate`.
bool isArchivedForDiscovery({
  required String pubkey,
  required Set<String> archivedPubkeys,
  String? currentPubkey,
}) {
  final lower = pubkey.toLowerCase();
  final self = currentPubkey?.toLowerCase();
  if (self != null && lower == self) return false;
  return archivedPubkeys.contains(lower);
}

Future<String?> _fetchRelaySelf(String relayUrl) async {
  final uri = _relayInfoUri(relayUrl);
  if (uri == null) return null;

  try {
    final response = await http
        .get(uri, headers: const {'Accept': 'application/nostr+json'})
        .timeout(const Duration(seconds: 5));
    if (response.statusCode < 200 || response.statusCode >= 300) {
      return null;
    }

    final document = jsonDecode(response.body);
    if (document is! Map<String, dynamic>) return null;

    final relaySelf = document['self'];
    if (relaySelf is! String) return null;
    final normalized = relaySelf.trim().toLowerCase();
    if (normalized.length != 64 ||
        !RegExp(r'^[0-9a-f]+$').hasMatch(normalized)) {
      return null;
    }
    return normalized;
  } catch (_) {
    return null;
  }
}

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

Set<String> _archivedPubkeysFromSnapshot(NostrEvent snapshot) {
  final archived = <String>{};
  for (final tag in snapshot.tags) {
    if (tag.isEmpty || tag.first != 'p' || tag.length < 2) continue;
    final pk = tag[1].toLowerCase();
    if (pk.length == 64 && RegExp(r'^[0-9a-f]+$').hasMatch(pk)) {
      archived.add(pk);
    }
  }
  return archived;
}
