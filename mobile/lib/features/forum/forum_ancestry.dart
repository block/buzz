import 'package:flutter/foundation.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../shared/crypto/event_signing.dart';
import '../../shared/relay/relay.dart';

/// Checks event hashes and signatures away from the UI isolate.
Future<void> verifyForumEvents(List<NostrEvent> events) =>
    events.isEmpty ? Future.value() : compute(_verifyEvents, events);

void _verifyEvents(List<NostrEvent> events) {
  for (final event in events) {
    if (event.createdAt < 0 ||
        nostr.Event.fromMap(event.toJson(), verify: false).getEventId() !=
            event.id ||
        !verifySchnorrHex(
          publicKey: event.pubkey,
          messageHex: event.id,
          signature: event.sig,
        )) {
      throw const FormatException('Invalid forum event signature');
    }
  }
}

/// Resolves signed reply chains to [root], fetching parents outside the page.
///
/// The caller must first verify the root's signature, channel, and lack of a
/// reply parent. A claimed root tag alone does not establish ancestry.
class ForumAncestry {
  final NostrEvent root;
  final Map<String, NostrEvent> _events;
  final Set<String> verified;

  ForumAncestry(this.root) : _events = {root.id: root}, verified = {root.id};
  ForumAncestry._(this.root, this._events, this.verified);

  /// Isolates in-flight work so a retired read cannot modify newer state.
  ForumAncestry copy() => ForumAncestry._(root, {..._events}, {...verified});

  /// Fetches at most 1,000 ancestors in batches of 100, to a depth of 100.
  /// Missing, conflicting, cyclic, and over-budget paths remain unresolved.
  Future<void> resolve(
    RelaySessionNotifier session,
    List<NostrEvent> rows,
    void Function() checkActive,
  ) async {
    checkActive();
    await verifyForumEvents(rows);
    checkActive();
    _events.addAll({for (final event in rows) event.id: event});
    _checkSize();
    final attempted = <String>{};
    for (var depth = 0; depth < 100; depth++) {
      final missing = <String>{};
      for (final row in rows) {
        _walk(row.id, missing);
      }
      missing.removeAll(attempted);
      if (missing.isEmpty || attempted.length >= 1000) break;
      final remaining = 1000 - attempted.length;
      final targets = missing.take(remaining < 100 ? remaining : 100).toList();
      attempted.addAll(targets);
      checkActive();
      final parents = await session.queryRelay([
        NostrFilter(
          kinds: EventKind.channelMessageEventKinds,
          ids: targets,
          tags: {
            '#h': [root.channelId!],
          },
          limit: targets.length,
        ),
      ]);
      checkActive();
      await verifyForumEvents(parents);
      checkActive();
      for (final event in parents) {
        if (targets.contains(event.id) &&
            event.channelId == root.channelId &&
            EventKind.channelMessageEventKinds.contains(event.kind)) {
          _events[event.id] = event;
        }
      }
      _checkSize();
    }
    for (final id in _events.keys.toList()) {
      _walk(id, <String>{});
    }
  }

  void _checkSize() {
    if (_events.length > 11000) {
      throw StateError('Forum ancestry limit reached');
    }
  }

  void _walk(String id, Set<String> missing) {
    final path = <String>[];
    var current = id;
    for (var depth = 0; depth <= 100; depth++) {
      if (verified.contains(current)) {
        verified.addAll(path);
        return;
      }
      if (path.contains(current)) return;
      final event = _events[current];
      if (event == null) {
        missing.add(current);
        return;
      }
      if (event.channelId != root.channelId) return;
      final thread = event.threadReference;
      if (thread.parentId == null ||
          (thread.rootId != root.id && thread.rootId != thread.parentId)) {
        return;
      }
      path.add(current);
      current = thread.parentId!;
    }
  }
}
