import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

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
}

/// Tracks who is currently typing in a specific channel.
///
/// Subscribes to typing and message events so a reply ends its typing signal.
/// Entries expire after 8 seconds (matching the desktop TTL).
class ChannelTypingNotifier extends Notifier<List<TypingEntry>> {
  static const _ttlMs = 8000;
  static const _pruneIntervalMs = 1000;

  final String channelId;
  void Function()? _unsubscribe;
  Timer? _pruneTimer;
  Timer? _retryTimer;
  Timer? _healthyTimer;
  int _generation = 0;
  final _activity = <(String, String?), _TypingActivity>{};

  ChannelTypingNotifier(this.channelId);

  @override
  List<TypingEntry> build() {
    final sessionState = ref.watch(relaySessionProvider);
    final generation = ++_generation;
    _activity.clear();

    ref.onDispose(() {
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
    });

    if (sessionState.status == SessionStatus.connected) {
      unawaited(_subscribeLive(generation));
    }

    return [];
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
    state = [
      for (final entry in _activity.entries)
        if (entry.value.visibleUntil > now)
          TypingEntry(
            pubkey: entry.key.$1,
            threadHeadId: entry.key.$2,
            expiresAtMs: entry.value.visibleUntil,
          ),
    ];
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
