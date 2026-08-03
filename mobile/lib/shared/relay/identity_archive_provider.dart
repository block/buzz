import 'dart:convert';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:nostr/nostr.dart' as nostr;

import 'nostr_models.dart';
import 'relay_provider.dart';
import 'relay_session.dart';

const _archivedIdentitiesKind = 13535;
final _hexPubkeyPattern = RegExp(r'^[0-9a-f]{64}$');

/// HTTP client used to read the active relay's NIP-11 information document.
///
/// Exposed as a provider so tests can supply deterministic relay metadata.
final identityArchiveHttpClientProvider = Provider<http.Client>((ref) {
  final client = http.Client();
  ref.onDispose(client.close);
  return client;
});

/// Returns the relay identity advertised by a NIP-11 document.
///
/// NIP-IA snapshots are authoritative only when signed by this exact key.
String? relaySelfPubkeyFromDocument(String body) {
  try {
    final document = jsonDecode(body);
    if (document is! Map<String, dynamic>) return null;
    final relaySelf = document['self'];
    if (relaySelf is! String) return null;
    final normalized = relaySelf.trim().toLowerCase();
    return _hexPubkeyPattern.hasMatch(normalized) ? normalized : null;
  } catch (_) {
    return null;
  }
}

/// Parses a trusted NIP-IA archived-identities snapshot.
///
/// Returns `null` when the event is not a structurally and cryptographically
/// valid kind:13535 snapshot signed by [relaySelfPubkey]. Invalid snapshots
/// must fail open at the UI boundary rather than hiding active identities.
Set<String>? archivedIdentityPubkeysFromSnapshot({
  required NostrEvent event,
  required String relaySelfPubkey,
}) {
  final normalizedRelaySelf = relaySelfPubkey.trim().toLowerCase();
  if (!_hexPubkeyPattern.hasMatch(normalizedRelaySelf) ||
      event.kind != _archivedIdentitiesKind ||
      event.pubkey.toLowerCase() != normalizedRelaySelf) {
    return null;
  }

  final nip70Tags = event.tags.where((tag) => tag.isNotEmpty && tag[0] == '-');
  if (nip70Tags.length != 1 || nip70Tags.single.length != 1) {
    return null;
  }

  try {
    final verified = nostr.Event.fromJson(jsonEncode(event.toJson()));
    if (verified.id != event.id ||
        verified.pubkey.toLowerCase() != normalizedRelaySelf) {
      return null;
    }
  } catch (_) {
    return null;
  }

  return {
    for (final tag in event.tags)
      if (tag.length >= 2 &&
          tag[0] == 'p' &&
          _hexPubkeyPattern.hasMatch(tag[1].toLowerCase()))
        tag[1].toLowerCase(),
  };
}

/// Relay-scoped archived identity set used by forward-looking discovery UI.
///
/// The provider deliberately fails open on NIP-11, relay, or verification
/// errors. Historical events remain untouched; consumers only exclude these
/// pubkeys from people and agent pickers.
final relayArchivedIdentityPubkeysProvider =
    FutureProvider.autoDispose<Set<String>>((ref) async {
      final config = ref.watch(relayConfigProvider);
      final client = ref.watch(identityArchiveHttpClientProvider);
      final session = ref.watch(relaySessionProvider.notifier);

      try {
        final response = await client
            .get(
              Uri.parse(config.baseUrl),
              headers: const {'Accept': 'application/nostr+json'},
            )
            .timeout(const Duration(seconds: 5));
        if (response.statusCode < 200 || response.statusCode >= 300) {
          return const {};
        }

        final relaySelf = relaySelfPubkeyFromDocument(response.body);
        if (relaySelf == null) return const {};

        final events = await session.queryRelay([
          NostrFilter(
            kinds: const [_archivedIdentitiesKind],
            authors: [relaySelf],
            limit: 5,
          ),
        ]);
        events.sort((left, right) {
          final timeOrder = right.createdAt.compareTo(left.createdAt);
          return timeOrder != 0 ? timeOrder : left.id.compareTo(right.id);
        });

        for (final event in events) {
          final archived = archivedIdentityPubkeysFromSnapshot(
            event: event,
            relaySelfPubkey: relaySelf,
          );
          if (archived != null) return archived;
        }
      } catch (_) {
        // Archive state is a discovery hint, not an access-control boundary.
        // A failed trust check must not turn into a client-side shadowban.
      }

      return const {};
    });
