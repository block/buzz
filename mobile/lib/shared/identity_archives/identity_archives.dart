import 'dart:async';
import 'dart:convert';
import 'dart:isolate';

import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:buzz/shared/http/app_http_client.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../relay/relay.dart';
import '../me/me_sync.dart';

final relayInformationClientProvider = Provider<http.Client>((ref) {
  final client = newAppHttpClient();
  ref.onDispose(client.close);
  return client;
});

/// The last verified archive list per community and authority, so a refresh
/// asks the relay only for a newer list instead of downloading it again.
class _ArchiveCache {
  final entries = <(String, String), ({int createdAt, Set<String> archived})>{};
}

final _archiveCacheProvider = Provider<_ArchiveCache>((_) => _ArchiveCache());

/// The identities this relay's NIP-IA archive list (kind 13535) names, never
/// the viewer: an archive stays visible to its subject. Empty when the list
/// cannot be read. This is presentation, never ownership or admission.
final identityArchivesProvider = FutureProvider<Set<String>>((ref) async {
  final sync = ref.watch(meSyncProvider);
  final client = ref.watch(relayInformationClientProvider);
  // Completed on every exit, so a dropped, refused or timed-out response
  // stops downloading on a shared session instead of staying in flight.
  final abort = Completer<void>();
  try {
    final uri = Uri.parse(sync.community);
    final response = await client
        .send(
          http.AbortableRequest('GET', uri, abortTrigger: abort.future)
            ..headers['Accept'] = 'application/nostr+json',
        )
        .timeout(const Duration(seconds: 3));
    if (response.statusCode != 200) return {};
    final chunks = StreamIterator(
      response.stream.timeout(const Duration(seconds: 3)),
    );
    final bytes = <int>[];
    try {
      await (() async {
        while (await chunks.moveNext()) {
          final chunk = chunks.current;
          if (bytes.length + chunk.length > 256 * 1024) {
            throw const FormatException(
              'Relay information exceeds size limit.',
            );
          }
          bytes.addAll(chunk);
        }
      })().timeout(const Duration(seconds: 3));
    } finally {
      await chunks.cancel();
    }
    sync.check();
    final info = jsonDecode(utf8.decode(bytes));
    // NIP-11 self is the signing authority; pubkey is only a contact key.
    final authority = info is Map<String, dynamic> ? info['self'] : null;
    if (authority is! String ||
        !RegExp(r'^[0-9a-f]{64}$').hasMatch(authority)) {
      return {};
    }
    final cache = ref.read(_archiveCacheProvider).entries;
    final key = (sync.community, authority);
    final known = cache[key];
    // Ask only for a list newer than the known one. This assumes each list is
    // stamped strictly later than the previous one, which the relay does
    // except when two lists are published in the same second; then the
    // second can be missed until a later list arrives or the app restarts.
    final events = await sync.session.fetchHistory(
      NostrFilter(
        kinds: const [13535],
        authors: [authority],
        since: known == null ? null : known.createdAt + 1,
        limit: 1,
      ),
      timeout: const Duration(seconds: 3),
    );
    sync.check();
    // Hand back the known instance when it does not name the viewer, so an
    // unchanged list does not rebuild every watcher.
    Set<String> visible(Set<String> archived) => archived.contains(sync.pubkey)
        ? ({...archived}..remove(sync.pubkey))
        : archived;
    if (known != null && events.isEmpty) return visible(known.archived);
    final archived = await _parseArchives(events, authority);
    if (events.length != 1 || archived == null) {
      cache.remove(key);
      return {};
    }
    final shared = Set<String>.unmodifiable(archived);
    cache[key] = (createdAt: events.single.createdAt, archived: shared);
    return visible(shared);
  } catch (_) {
    return {};
  } finally {
    if (!abort.isCompleted) abort.complete();
  }
});

/// [identityArchivesProvider] with the relay configuration (community and
/// account) it was read for.
final _scopedArchivesProvider = FutureProvider<(RelayConfig, Set<String>)>((
  ref,
) async {
  final config = ref.watch(relayConfigProvider);
  return (config, await ref.watch(identityArchivesProvider.future));
});

/// Identities to leave out of forward-looking choices (pickers, search and
/// the Agents page), as desktop's `archiveHides`: the known-archived ones.
/// None while the archive list loads or cannot be read. History (messages,
/// activity, profiles) keeps showing archived identities.
///
/// A refresh keeps the last known set, but after a community or account
/// switch the previous set does not apply: nobody is hidden until the new
/// community's list arrives.
final hiddenArchivedIdentitiesProvider = Provider<Set<String>>((ref) {
  final config = ref.watch(relayConfigProvider);
  final known = ref.watch(_scopedArchivesProvider).value;
  if (known == null || !identical(known.$1, config)) return const {};
  return known.$2;
});

/// The archived identities, or null when [events] is not one valid list.
Future<Set<String>?> _parseArchives(
  List<NostrEvent> events,
  String authority,
) => Isolate.run(() => _archived(events, authority));

Set<String>? _archived(List<NostrEvent> events, String authority) {
  if (events.length != 1) return null;
  final event = events.single;
  final markers = event.tags.where((tag) => tag.isNotEmpty && tag.first == '-');
  if (event.kind != 13535 ||
      event.pubkey != authority ||
      event.content.isNotEmpty ||
      markers.length != 1 ||
      markers.single.length != 1) {
    return null;
  }
  final encoded = jsonEncode(event.toJson());
  if (utf8.encode(encoded).length > 2 * 1024 * 1024) return null;
  try {
    verifiedNostrEventFromJson(encoded);
  } on nostr.EventValidationException {
    return null;
  }
  return {
    for (final tag in event.tags)
      if (tag.length >= 2 &&
          tag[0] == 'p' &&
          RegExp(r'^[0-9a-f]{64}$').hasMatch(tag[1]))
        tag[1],
  };
}
