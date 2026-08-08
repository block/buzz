import 'dart:async';
import 'dart:convert';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:nostr/nostr.dart' as nostr;

import '../relay/relay.dart';

final _hexPubkey = RegExp(r'^[0-9a-fA-F]{64}$');

final archivedIdentitiesHttpClientProvider = Provider<http.Client>((ref) {
  final client = http.Client();
  ref.onDispose(client.close);
  return client;
});

/// Relay-scoped archive state from the latest valid NIP-IA snapshot.
///
/// This fails open while disconnected or when NIP-11/snapshot verification
/// fails, matching desktop's discovery predicate.
final archivedIdentityPubkeysProvider = FutureProvider<Set<String>>((
  ref,
) async {
  final sessionState = ref.watch(relaySessionProvider);
  if (sessionState.status != SessionStatus.connected) {
    final connected = Completer<Set<String>>();
    ref.onDispose(() {
      if (!connected.isCompleted) connected.complete(const {});
    });
    return connected.future;
  }

  final config = ref.watch(relayConfigProvider);
  try {
    final response = await ref
        .read(archivedIdentitiesHttpClientProvider)
        .get(
          Uri.parse(config.baseUrl),
          headers: const {'Accept': 'application/nostr+json'},
        )
        .timeout(const Duration(seconds: 5));
    if (response.statusCode < 200 || response.statusCode >= 300) {
      return const {};
    }

    final document = jsonDecode(response.body);
    if (document is! Map<String, dynamic>) return const {};
    final relaySelf = document['self'];
    if (relaySelf is! String || !_hexPubkey.hasMatch(relaySelf)) {
      return const {};
    }

    final events = await ref.read(relaySessionProvider.notifier).queryRelay([
      NostrFilter(
        kinds: const [EventKind.archivedIdentities],
        authors: [relaySelf.toLowerCase()],
        limit: 1,
      ),
    ]);
    if (events.isEmpty) return const {};
    events.sort((left, right) => right.createdAt.compareTo(left.createdAt));
    return archivedPubkeysFromSnapshot(events.first, relaySelf);
  } catch (_) {
    return const {};
  }
});

/// Returns the archived pubkeys from a valid snapshot signed by [relayPubkey].
/// Invalid or foreign snapshots fail open so unauthenticated relay state never
/// hides an identity.
Set<String> archivedPubkeysFromSnapshot(
  NostrEvent snapshot,
  String relayPubkey,
) {
  final relay = relayPubkey.toLowerCase();
  if (!_hexPubkey.hasMatch(relay) ||
      snapshot.kind != EventKind.archivedIdentities ||
      snapshot.pubkey.toLowerCase() != relay) {
    return const {};
  }

  final nip70Tags = snapshot.tags.where(
    (tag) => tag.isNotEmpty && tag.first == '-',
  );
  if (nip70Tags.length != 1 || nip70Tags.single.length != 1) {
    return const {};
  }

  try {
    nostr.Event.fromJson(jsonEncode(snapshot.toJson()));
  } catch (_) {
    return const {};
  }

  return Set.unmodifiable({
    for (final tag in snapshot.tags)
      if (tag.length >= 2 && tag.first == 'p' && _hexPubkey.hasMatch(tag[1]))
        tag[1].toLowerCase(),
  });
}
