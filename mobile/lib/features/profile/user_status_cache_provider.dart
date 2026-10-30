import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import 'user_status.dart';

/// In-memory cache of other users' NIP-38 statuses (kind:30315, d=general).
///
/// Subscribes to kind:30315 events over WebSocket for real-time updates.
/// Falls back to a 120-second backstop refresh via fetchHistory.
class UserStatusCacheNotifier extends Notifier<Map<String, UserStatus?>> {
  static const _refreshInterval = Duration(seconds: 120);

  final Set<String> _tracked = {};
  final Set<String> _pending = {};
  Timer? _batchTimer;
  Timer? _refreshTimer;
  Timer? _expirationTimer;
  void Function()? _statusUnsub;
  int _subscriptionVersion = 0;

  @override
  Map<String, UserStatus?> build() {
    ref.watch(relayClientProvider);
    final sessionState = ref.watch(relaySessionProvider);

    ref.onDispose(() {
      _batchTimer?.cancel();
      _batchTimer = null;
      _refreshTimer?.cancel();
      _refreshTimer = null;
      _expirationTimer?.cancel();
      _expirationTimer = null;
      _statusUnsub?.call();
      _statusUnsub = null;
    });

    if (sessionState.status == SessionStatus.connected) {
      _subscribeStatusUpdates();
    }

    return {};
  }

  /// Track status for [pubkeys]. Fetches immediately if not cached,
  /// and includes them in periodic refreshes.
  void track(List<String> pubkeys) {
    final normalized = pubkeys.map((pk) => pk.toLowerCase()).toList();
    final uncached = normalized
        .where((pk) => !state.containsKey(pk) && !_pending.contains(pk))
        .toList();

    _tracked.addAll(normalized);
    _ensureRefreshTimer();

    if (uncached.isEmpty) return;
    _pending.addAll(uncached);
    _batchTimer ??= Timer(const Duration(milliseconds: 50), _flushPending);
  }

  void _ensureRefreshTimer() {
    _refreshTimer ??= Timer.periodic(_refreshInterval, (_) => _refreshAll());
  }

  Future<void> _subscribeStatusUpdates() async {
    _statusUnsub?.call();
    _statusUnsub = null;
    _subscriptionVersion++;
    final version = _subscriptionVersion;

    final session = ref.read(relaySessionProvider.notifier);
    try {
      final unsub = await session.subscribe(
        const NostrFilter(
          kinds: [EventKind.userStatus],
          tags: {
            '#d': ['general'],
          },
          limit: 0,
        ),
        _handleStatusEvent,
      );
      if (version != _subscriptionVersion) {
        unsub();
        return;
      }
      _statusUnsub = unsub;
    } catch (error) {
      debugPrint(
        '[UserStatusCacheNotifier] status subscription failed: $error',
      );
    }
  }

  void _handleStatusEvent(NostrEvent event) {
    // Defense-in-depth: guard d-tag even though filter includes it.
    if (event.getTagValue('d') != 'general') return;

    final pubkey = event.pubkey.toLowerCase();
    if (!_tracked.contains(pubkey)) return;

    final parsed = UserStatus.fromEvent(event);
    final existing = state[pubkey];

    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    // An equal event still needs its expiration re-evaluated. Otherwise the
    // periodic refresh keeps an expired status alive forever.
    if (existing != null && existing.updatedAt > parsed.updatedAt) return;
    if (existing != null &&
        existing.updatedAt == parsed.updatedAt &&
        !parsed.isExpiredAt(now)) {
      return;
    }

    final status = parsed.isEmpty || parsed.isExpiredAt(now) ? null : parsed;
    final updated = Map<String, UserStatus?>.from(state);
    updated[pubkey] = status;
    _replaceState(updated);
  }

  /// Directly update a pubkey's cached status. Used by [UserStatusNotifier]
  /// for optimistic updates after publishing.
  void updateStatus(String pubkey, UserStatus? status) {
    final pk = pubkey.toLowerCase();
    final updated = Map<String, UserStatus?>.from(state);
    updated[pk] = status;
    _replaceState(updated);
  }

  Future<void> _refreshAll() async {
    if (_tracked.isEmpty) return;
    await _fetchStatuses(_tracked.toList());
  }

  Future<void> _flushPending() async {
    _batchTimer = null;
    if (_pending.isEmpty) return;

    final pubkeys = _pending.toList();
    _pending.clear();
    await _fetchStatuses(pubkeys);
  }

  Future<void> _fetchStatuses(List<String> pubkeys) async {
    try {
      final session = ref.read(relaySessionProvider.notifier);
      final events = await session.fetchHistory(
        NostrFilter(
          kinds: const [EventKind.userStatus],
          authors: pubkeys,
          tags: const {
            '#d': ['general'],
          },
          limit: pubkeys.length,
        ),
      );

      final updated = Map<String, UserStatus?>.from(state);

      // Initialise requested pubkeys that had no events to null (cleared).
      for (final pk in pubkeys) {
        if (!updated.containsKey(pk)) {
          updated[pk] = null;
        }
      }

      for (final event in events) {
        if (event.getTagValue('d') != 'general') continue;
        final pk = event.pubkey.toLowerCase();
        final parsed = UserStatus.fromEvent(event);
        final existing = updated[pk];
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        if (existing != null && existing.updatedAt > parsed.updatedAt) {
          continue;
        }
        if (existing != null &&
            existing.updatedAt == parsed.updatedAt &&
            !parsed.isExpiredAt(now)) {
          continue;
        }
        updated[pk] = parsed.isEmpty || parsed.isExpiredAt(now) ? null : parsed;
      }

      _replaceState(updated);
    } catch (_) {
      // Silently fail — backstop will retry.
    }
  }

  void _replaceState(Map<String, UserStatus?> updated) {
    state = updated;
    _scheduleNextExpiration();
  }

  void _scheduleNextExpiration() {
    _expirationTimer?.cancel();
    _expirationTimer = null;

    DateTime? nextDeadline;
    for (final status in state.values) {
      final deadline = status?.expirationDateTime;
      if (deadline == null) continue;
      if (nextDeadline == null || deadline.isBefore(nextDeadline)) {
        nextDeadline = deadline;
      }
    }
    if (nextDeadline == null) return;

    final remaining = nextDeadline.difference(DateTime.now());
    _expirationTimer = Timer(
      remaining.isNegative ? Duration.zero : remaining,
      _expireDueStatuses,
    );
  }

  void _expireDueStatuses() {
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    var changed = false;
    final updated = Map<String, UserStatus?>.from(state);
    for (final entry in state.entries) {
      final status = entry.value;
      if (status != null && status.isExpiredAt(now)) {
        updated[entry.key] = null;
        changed = true;
      }
    }

    if (changed) {
      _replaceState(updated);
    } else {
      // Timer callbacks can land just before the Unix-second boundary.
      _expirationTimer = Timer(
        const Duration(milliseconds: 50),
        _expireDueStatuses,
      );
    }
  }
}

final userStatusCacheProvider =
    NotifierProvider<UserStatusCacheNotifier, Map<String, UserStatus?>>(
      UserStatusCacheNotifier.new,
    );
