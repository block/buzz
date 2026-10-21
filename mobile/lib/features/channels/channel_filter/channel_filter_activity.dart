import 'dart:async';
import 'dart:convert';
import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import '../../../shared/relay/relay.dart';
import '../channels_provider.dart';

/// Verification stays off the UI isolate. Null after successful history means
/// known empty; a failed/invalid response stays unknown and exposes Retry.
int? verifiedFilterActivity(List<Map<String, dynamic>> records) {
  int? latest;
  final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
  for (final record in records) {
    nostr.Event.fromJson(jsonEncode(record));
    final at = record['created_at'] as int;
    if (at <= now + 300 && (latest == null || at > latest)) latest = at;
  }
  if (records.isNotEmpty && latest == null) {
    throw const FormatException('Activity history has no acceptable timestamp');
  }
  return latest;
}

class _ActivityReadPool {
  int _running = 0;
  final _waiting = <Completer<void>>[];
  Future<T> run<T>(Future<T> Function() read) async {
    if (_running >= 4) {
      final turn = Completer<void>();
      _waiting.add(turn);
      await turn.future;
    } else {
      _running++;
    }
    try {
      return await read();
    } finally {
      if (_waiting.isNotEmpty) {
        _waiting.removeAt(0).complete();
      } else {
        _running--;
      }
    }
  }
}

final _activityReadPoolProvider = Provider((ref) => _ActivityReadPool());
final channelFilterActivityProvider = FutureProvider.autoDispose
    .family<int?, String>((ref, channelId) async {
      // Every live message and foreground refresh updates this owner. Re-read
      // only the affected channel; never trust its unverified presentation stamp.
      final channelsNotifier = ref.watch(channelsProvider.notifier);
      ref.watch(
        channelsProvider.select(
          (value) => (
            value.value
                ?.where((c) => c.id == channelId)
                .firstOrNull
                ?.lastMessageAt,
            channelsNotifier.activityRevisionFor(channelId),
          ),
        ),
      );
      ref.listen(appLifecycleProvider, (_, next) {
        if (next == AppLifecycleState.resumed) ref.invalidateSelf();
      });
      final session = ref.watch(relaySessionProvider.select((s) => s.status));
      if (session != SessionStatus.connected) {
        throw StateError('Activity unavailable while disconnected');
      }
      var disposed = false;
      ref.onDispose(() => disposed = true);
      final relay = ref.read(relaySessionProvider.notifier);
      return ref.read(_activityReadPoolProvider).run(() async {
        if (disposed) throw StateError('Activity request superseded');
        final events = await relay.fetchHistory(
          NostrFilter(
            kinds: EventKind.channelMessageEventKinds,
            tags: {
              '#h': [channelId],
            },
            limit: 1,
          ),
        );
        final matching = events
            .where(
              (e) =>
                  e.channelId == channelId &&
                  EventKind.channelMessageEventKinds.contains(e.kind),
            )
            .map((e) => e.toJson())
            .toList();
        return compute(verifiedFilterActivity, matching);
      });
    }, retry: (_, _) => null);
