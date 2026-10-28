import 'dart:async';
import 'dart:convert';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../crypto/nip44.dart';
import '../relay/relay.dart';
import 'me_sync.dart';

const defaultAgentTag = 'buzz-default-agent-v1';
String defaultAgentCoordinate(String community) =>
    '$defaultAgentTag:${Uri.encodeComponent(community)}:default-agent:me';

/// Reads the same signed, self-encrypted preference as desktop.
String? decodeDefaultAgent(NostrEvent event, MeSync sync) {
  if (event.kind != 30078 ||
      event.pubkey != sync.pubkey ||
      event.content.length > 4096 ||
      event.tags.where((t) => t.isNotEmpty && t[0] == 'd').length != 1 ||
      event.getTagValue('d') != defaultAgentCoordinate(sync.community) ||
      event.getTagValue('t') != defaultAgentTag) {
    throw const FormatException('Invalid default agent preference');
  }
  nostr.Event.fromJson(jsonEncode(event.toJson()));
  final raw = jsonDecode(nip44Decrypt(sync.key, event.content));
  if (raw is! Map<String, dynamic> ||
      raw.length != 4 ||
      raw['version'] != 1 ||
      raw['community'] != sync.community ||
      raw['deleted'] != false) {
    throw const FormatException('Invalid default agent preference');
  }
  final value = raw['value'];
  if (value is! Map<String, dynamic> ||
      value.length != 3 ||
      !value.containsKey('pubkey') ||
      value['type'] != 'default-agent' ||
      value['id'] != 'me' ||
      (value['pubkey'] != null &&
          (value['pubkey'] is! String ||
              !RegExp(
                r'^[0-9a-f]{64}$',
              ).hasMatch(value['pubkey'] as String)))) {
    throw const FormatException('Invalid default agent preference');
  }
  return value['pubkey'] as String?;
}

/// A cached choice remains available offline; account/community changes reset it.
final defaultAgentProvider = StreamProvider<String?>((ref) {
  final sync = ref.watch(meSyncProvider);
  ref.listen(relaySessionProvider.select((s) => s.status), (before, after) {
    if (before != after && after == SessionStatus.connected) {
      ref.invalidateSelf();
    }
  });
  final output = StreamController<String?>();
  final cacheKey = 'default-agent:${sync.community}:${sync.pubkey}';
  final filter = NostrFilter(
    kinds: const [30078],
    authors: [sync.pubkey],
    tags: {
      '#d': [defaultAgentCoordinate(sync.community)],
    },
    limit: 1,
  );
  var disposed = false;
  NostrEvent? latest;
  void Function()? unsubscribe;
  Future<void> writes = Future.value();
  void accept(NostrEvent event) {
    if (disposed || (latest != null && !meEventIsNewer(event, latest!))) return;
    try {
      final choice = decodeDefaultAgent(event, sync);
      latest = event;
      output.add(choice);
      writes = writes
          .then((_) async {
            if (disposed || latest?.id != event.id) return;
            final prefs = sync.prefs;
            if (prefs != null &&
                !await prefs.setString(cacheKey, jsonEncode(event.toJson()))) {
              throw StateError(
                'Could not save the default agent for offline use.',
              );
            }
          })
          .catchError((Object error, StackTrace stack) {
            if (!disposed) output.addError(error, stack);
          });
    } catch (error, stack) {
      if (!disposed) output.addError(error, stack);
    }
  }

  Future<void> start() async {
    try {
      final cached = sync.prefs?.getString(cacheKey);
      if (cached != null) {
        final event = NostrEvent.fromJson(
          jsonDecode(cached) as Map<String, dynamic>,
        );
        final choice = decodeDefaultAgent(event, sync);
        latest = event;
        output.add(choice);
      }
    } catch (_) {
      // A damaged local cache cannot block a verified relay read.
    }
    try {
      sync.check();
      unsubscribe = await sync.session.subscribe(
        filter,
        accept,
        onClosed: (_) {
          if (!disposed) {
            output.addError(
              StateError('Default agent sync stopped. Retry to reconnect.'),
            );
          }
        },
      );
      if (disposed) {
        unsubscribe?.call();
        return;
      }
      final events = await sync.session.fetchHistory(filter);
      if (disposed) return;
      for (final event in events) {
        accept(event);
      }
      if (latest == null) output.add(null);
    } catch (error, stack) {
      if (!disposed && latest == null) output.addError(error, stack);
    }
  }

  ref.onDispose(() {
    disposed = true;
    unsubscribe?.call();
    unawaited(output.close());
  });
  unawaited(start());
  return output.stream;
}, retry: (_, _) => null);
