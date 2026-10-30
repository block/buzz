import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:hooks_riverpod/misc.dart' show ProviderListenable;

import '../../shared/relay/relay.dart';

/// A single typing indicator entry.
@immutable
class TypingEntry {
  final String pubkey;
  final String? threadHeadId;
  final int expiresAtMs;

  const TypingEntry({
    required this.pubkey,
    this.threadHeadId,
    required this.expiresAtMs,
  });

  @override
  bool operator ==(Object other) =>
      other is TypingEntry &&
      other.pubkey == pubkey &&
      other.threadHeadId == threadHeadId &&
      other.expiresAtMs == expiresAtMs;

  @override
  int get hashCode => Object.hash(pubkey, threadHeadId, expiresAtMs);
}

/// The [TypingEntry]s other than [selfPubkey]'s that [include] keeps,
/// compared by value, so a widget that selects them rebuilds only when
/// they change rather than on every update to the channel's list.
@immutable
class VisibleTyping {
  final List<TypingEntry> entries;

  VisibleTyping(
    List<TypingEntry> all, {
    required String? selfPubkey,
    required bool Function(TypingEntry entry) include,
  }) : entries = List.unmodifiable([
         for (final entry in all)
           if ((selfPubkey == null ||
                   entry.pubkey.toLowerCase() != selfPubkey.toLowerCase()) &&
               include(entry))
             entry,
       ]);

  @override
  bool operator ==(Object other) =>
      other is VisibleTyping && listEquals(other.entries, entries);

  @override
  int get hashCode => Object.hashAll(entries);
}

/// What a page watches to show typing in [channelId]: everyone but
/// [selfPubkey], filtered by [include]. The user's own echoed indicator
/// refreshes its expiry every few seconds, and this skips those updates.
ProviderListenable<VisibleTyping> visibleTypingProvider(
  String channelId, {
  required String? selfPubkey,
  required bool Function(TypingEntry entry) include,
}) => channelTypingProvider(
  channelId,
).select((all) => VisibleTyping(all, selfPubkey: selfPubkey, include: include));

/// Tracks who is currently typing in a specific channel.
///
/// Subscribes to typing and message events so a reply ends its typing signal.
/// Entries expire after 8 seconds (matching the desktop TTL).
class ChannelTypingNotifier extends Notifier<List<TypingEntry>> {
  static const _ttlMs = 8000;
  static const _pruneIntervalMs = 1000;

  /// How long a channel nobody shows keeps its typing subscription.
  static const _detachDelay = Duration(seconds: 30);

  final String channelId;
  void Function()? _unsubscribe;
  Timer? _pruneTimer;
  Timer? _retryTimer;
  Timer? _healthyTimer;
  Timer? _detachTimer;
  bool _detached = false;
  int _generation = 0;
  final _activity = <(String, String?), _TypingActivity>{};

  ChannelTypingNotifier(this.channelId);

  @override
  List<TypingEntry> build() {
    final sessionState = ref.watch(relaySessionProvider);
    final generation = ++_generation;
    _activity.clear();
    // Riverpod defers rebuilding a provider nobody is listening to until it
    // is listened to or read again, and then skips onResume. So a build
    // always means the channel is wanted.
    _detached = false;

    ref.onCancel(() {
      _detachTimer?.cancel();
      _detachTimer = Timer(_detachDelay, () {
        _detachTimer = null;
        _detached = true;
        _stop();
        state = [];
      });
    });
    ref.onResume(() {
      _detachTimer?.cancel();
      _detachTimer = null;
      if (_detached) {
        _detached = false;
        Future.microtask(() {
          if (ref.mounted) ref.invalidateSelf();
        });
      }
    });
    ref.onDispose(() {
      _detachTimer?.cancel();
      _detachTimer = null;
      _stop();
    });

    if (sessionState.status == SessionStatus.connected) {
      unawaited(_subscribeLive(generation));
    }

    return [];
  }

  void _stop() {
    _generation++;
    _activity.clear();
    _unsubscribe?.call();
    _unsubscribe = null;
    _pruneTimer?.cancel();
    _pruneTimer = null;
    _retryTimer?.cancel();
    _retryTimer = null;
    _healthyTimer?.cancel();
    _healthyTimer = null;
  }

  Future<void> _subscribeLive(int generation, [int attempt = 0]) async {
    final session = ref.read(relaySessionProvider.notifier);
    var closed = false;
    var failures = attempt;
    void failed() {
      if (closed) return;
      closed = true;
      // subscribe may invoke onClosed synchronously during provider build.
      scheduleMicrotask(() {
        if (generation != _generation) return;
        _unsubscribe?.call();
        _unsubscribe = null;
        _healthyTimer?.cancel();
        _healthyTimer = null;
        _activity.clear();
        state = [];
        _pruneTimer?.cancel();
        _pruneTimer = null;
        // Bound retries for policy rejections; a transport reconnect resets it.
        if (failures < 3) {
          _retryTimer = Timer(Duration(seconds: 1 << failures), () {
            _retryTimer = null;
            if (generation == _generation) {
              unawaited(_subscribeLive(generation, failures + 1));
            }
          });
        }
      });
    }

    try {
      final unsubscribe = await session.subscribe(
        NostrFilter(
          kinds: [
            EventKind.typingIndicator,
            ...EventKind.channelMessageEventKinds,
            EventKind.streamMessageDiff,
          ],
          tags: {
            '#h': [channelId],
          },
          limit: 10,
        ),
        (event) {
          if (!closed && generation == _generation) _handleTypingEvent(event);
        },
        onClosed: (_) => failed(),
      );
      if (closed || generation != _generation) {
        unsubscribe();
      } else {
        // Admission alone is not health: repeated terminal rejections may
        // arrive after subscribe's readiness timeout. Reset only after a full
        // typing TTL of uninterrupted subscription health.
        _healthyTimer = Timer(const Duration(milliseconds: _ttlMs), () {
          if (!closed && generation == _generation) failures = 0;
        });
        _unsubscribe = unsubscribe;
      }
    } catch (_) {
      failed();
    }
  }

  void _handleTypingEvent(NostrEvent event) {
    final now = DateTime.now().millisecondsSinceEpoch;
    final at = event.createdAt * 1000;
    if (event.channelId != channelId || at > now || at + _ttlMs <= now) return;
    final scope = (event.pubkey.toLowerCase(), event.threadReference.rootId);
    _activity.removeWhere((_, value) => value.retainUntil <= now);
    if (!_activity.containsKey(scope) && _activity.length >= 1024) return;
    final record = _activity.putIfAbsent(scope, _TypingActivity.new);
    if (event.kind == EventKind.typingIndicator) {
      if (at <= record.lastActivity || at <= record.lastMessage) return;
      record.lastActivity = at;
      if (now >= record.quietUntil) record.visibleUntil = at + _ttlMs;
    } else {
      if (at <= record.lastMessage) return;
      record.lastMessage = at;
      if (at >= record.lastActivity) {
        record.visibleUntil = 0;
        record.quietUntil = now + 2000;
      }
    }
    _prune();
    _ensurePruneTimer();
  }

  void _ensurePruneTimer() {
    _pruneTimer ??= Timer.periodic(
      const Duration(milliseconds: _pruneIntervalMs),
      (_) => _prune(),
    );
  }

  void _prune() {
    final now = DateTime.now().millisecondsSinceEpoch;
    _activity.removeWhere((_, value) => value.retainUntil <= now);
    final next = [
      for (final entry in _activity.entries)
        if (entry.value.visibleUntil > now)
          TypingEntry(
            pubkey: entry.key.$1,
            threadHeadId: entry.key.$2,
            expiresAtMs: entry.value.visibleUntil,
          ),
    ];
    // The timer ticks every second while anyone is active. Publishing an
    // unchanged list would rebuild every watcher, including channel pages.
    if (!listEquals(next, state)) state = next;
    if (_activity.isEmpty) {
      _pruneTimer?.cancel();
      _pruneTimer = null;
    }
  }
}

final channelTypingProvider =
    NotifierProvider.family<ChannelTypingNotifier, List<TypingEntry>, String>(
      ChannelTypingNotifier.new,
    );

class _TypingActivity {
  int lastActivity = -1;
  int lastMessage = -1;
  int visibleUntil = 0;
  int quietUntil = 0;
  int get retainUntil {
    final expiry =
        (lastActivity > lastMessage ? lastActivity : lastMessage) + 8000;
    return expiry > quietUntil ? expiry : quietUntil;
  }
}
