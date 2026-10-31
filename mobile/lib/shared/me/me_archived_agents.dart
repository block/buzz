import 'dart:async';
import 'dart:convert';
import 'dart:isolate';

import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;

import '../relay/relay.dart';
import 'me_sync.dart';

final meRelayInformationClientProvider = Provider<http.Client>((ref) {
  final client = http.Client();
  ref.onDispose(client.close);
  return client;
});

/// Matches desktop's known-archived filtering. Unknown archive evidence does
/// not hide identities; this is presentation, never ownership or admission.
final meArchivedAgentsProvider = FutureProvider<Set<String>>((ref) async {
  final sync = ref.watch(meSyncProvider);
  final client = ref.watch(meRelayInformationClientProvider);
  try {
    final uri = Uri.parse(sync.community);
    final response = await client
        .send(
          http.Request('GET', uri)
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
    final events = await sync.session.fetchHistory(
      NostrFilter(kinds: const [13535], authors: [authority], limit: 1),
      timeout: const Duration(seconds: 3),
    );
    sync.check();
    return await _parseArchives(events, authority);
  } catch (_) {
    return {};
  }
});

Future<Set<String>> _parseArchives(List<NostrEvent> events, String authority) =>
    Isolate.run(() => _archived(events, authority));

Set<String> _archived(List<NostrEvent> events, String authority) {
  if (events.length != 1) return {};
  final event = events.single;
  final markers = event.tags.where((tag) => tag.isNotEmpty && tag.first == '-');
  if (event.kind != 13535 ||
      event.pubkey != authority ||
      event.content.isNotEmpty ||
      markers.length != 1 ||
      markers.single.length != 1) {
    return {};
  }
  final encoded = jsonEncode(event.toJson());
  if (utf8.encode(encoded).length > 2 * 1024 * 1024) return {};
  verifiedNostrEventFromJson(encoded);
  return {
    for (final tag in event.tags)
      if (tag.length >= 2 &&
          tag[0] == 'p' &&
          RegExp(r'^[0-9a-f]{64}$').hasMatch(tag[1]))
        tag[1],
  };
}
