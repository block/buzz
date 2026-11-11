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

/// Resolves reply ancestry to a channel-checked root, including out-of-page
/// parents. A claimed root is only a hint, never a substitute for the path.
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
  ) async {
    await verifyForumEvents(rows);
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
      final targets = missing.take(100).toList();
      attempted.addAll(targets);
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
      await verifyForumEvents(parents);
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
