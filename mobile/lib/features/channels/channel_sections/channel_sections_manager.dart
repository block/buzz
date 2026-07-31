import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:uuid/uuid.dart';

import '../../../shared/crypto/nip44.dart';
import '../../../shared/relay/relay.dart';
import '../read_state/read_state_time.dart';
import 'channel_sections_storage.dart';

const _uuid = Uuid();

class ChannelSectionsCrypto {
  final Uint8List _conversationKey;

  ChannelSectionsCrypto(String nsec, String pubkey)
    : _conversationKey = _deriveKey(nsec, pubkey);

  static Uint8List _deriveKey(String nsec, String pubkey) {
    final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
    return getConversationKey(privkeyHex, pubkey);
  }

  String encrypt(String plaintext) => nip44Encrypt(_conversationKey, plaintext);

  String decrypt(String ciphertext) =>
      nip44Decrypt(_conversationKey, ciphertext);
}

class ChannelSectionsManager {
  final String pubkey;
  final ChannelSectionsStorage _storage;
  final ChannelSectionsCrypto _crypto;
  final RelaySessionNotifier? _relaySession;
  final SignedEventRelay? _signedEventRelay;
  final bool _remoteEnabled;
  final VoidCallback _onChanged;

  ChannelSectionStore _store;
  ChannelSectionStore? _lastPublishedStore;
  Timer? _publishDebounce;
  int _lastRemoteCreatedAt = 0;
  String? _lastRemoteEventId;
  void Function()? _unsubscribe;
  bool _disposed = false;

  /// Base delay for the startup-sync retry backoff. Overridable in tests.
  final Duration _startupRetryBaseDelay;
  Timer? _startupRetryTimer;
  int _startupRetryAttempt = 0;
  bool _startupFetchSucceeded = false;

  ChannelSectionsManager({
    required this.pubkey,
    required SharedPreferences prefs,
    required ChannelSectionsCrypto crypto,
    required RelaySessionNotifier? relaySession,
    required SignedEventRelay? signedEventRelay,
    required bool remoteEnabled,
    required VoidCallback onChanged,
    @visibleForTesting
    Duration startupRetryBaseDelay = const Duration(seconds: 2),
  }) : _storage = ChannelSectionsStorage(prefs),
       _crypto = crypto,
       _relaySession = relaySession,
       _signedEventRelay = signedEventRelay,
       _remoteEnabled = remoteEnabled,
       _onChanged = onChanged,
       _startupRetryBaseDelay = startupRetryBaseDelay,
       _store = ChannelSectionsStorage(prefs).read(pubkey);

  ChannelSectionStore get store => _store;

  Future<void> initialize() async {
    if (_disposed) return;

    if (!_remoteEnabled || _relaySession == null) {
      _onChanged();
      return;
    }

    await _syncWithRelay();
    _onChanged();
  }

  /// One startup-sync attempt: fetch the remote blob, then start the live
  /// subscription. Either step can lose a transient race on cold start (the
  /// relay rate-limits the burst of per-channel subscriptions and rejects
  /// with `rate-limited: quota exceeded`) — retry with backoff instead of
  /// silently giving up, which left desktop-created groups invisible until
  /// an unrelated refetch.
  ///
  /// Retries are intentionally unbounded for the manager's lifetime: this
  /// sync must eventually land for groups to appear at all, and at the 30s
  /// delay ceiling a persistent retry is cheap. Do not "fix" this into a
  /// bounded loop — giving up permanently is the exact bug this replaces.
  Future<void> _syncWithRelay() async {
    if (!_startupFetchSucceeded) {
      _startupFetchSucceeded = await _fetchAndMerge();
    }

    final subscribed = _unsubscribe != null || await _startLiveSubscription();

    if (!_startupFetchSucceeded || !subscribed) {
      _scheduleStartupRetry();
    } else {
      // Fully recovered — later transient failures (e.g. a late relay
      // CLOSED) start backing off from the base delay again instead of the
      // ceiling the cold start climbed to.
      _startupRetryAttempt = 0;
    }
  }

  void _scheduleStartupRetry() {
    if (_disposed) return;
    _startupRetryTimer?.cancel();
    // The inner shift cap is overflow protection, not the delay policy: the
    // consecutive-failure counter is unbounded, and an unchecked `<<`
    // past 62 wraps negative, which would make the Timer fire immediately in
    // a hot loop. At the default 2s base the outer 30s clamp is what callers
    // actually observe (2s, 4s, …, 30s); the shift cap only bites for the
    // tiny injected bases used in tests.
    final delayMs = min(
      _startupRetryBaseDelay.inMilliseconds << min(_startupRetryAttempt, 5),
      30000,
    );
    _startupRetryAttempt++;
    debugPrint(
      '[ChannelSectionsManager] startup sync incomplete; '
      'retrying in ${delayMs}ms (attempt $_startupRetryAttempt)',
    );
    _startupRetryTimer = Timer(Duration(milliseconds: delayMs), () {
      _startupRetryTimer = null;
      unawaited(
        _syncWithRelay().then((_) {
          if (!_disposed) _onChanged();
        }),
      );
    });
  }

  void dispose({bool flushPending = true}) {
    if (_disposed) return;
    _disposed = true;

    _startupRetryTimer?.cancel();
    _startupRetryTimer = null;

    final hadPending = _publishDebounce != null;
    _publishDebounce?.cancel();
    _publishDebounce = null;

    if (flushPending && hadPending && _remoteEnabled) {
      unawaited(_publish(allowDisposed: true));
    }

    _unsubscribe?.call();
    _unsubscribe = null;
  }

  void createSection(String name) {
    if (_disposed) return;
    final maxOrder = _store.sections.fold<int>(
      -1,
      (max, s) => s.order > max ? s.order : max,
    );
    final section = ChannelSection(
      id: _uuid.v4(),
      name: name.trim(),
      order: maxOrder + 1,
    );
    _store = ChannelSectionStore(
      sections: [..._store.sections, section],
      assignments: _store.assignments,
    );
    _persist();
    markDirty();
  }

  void renameSection(String sectionId, String newName) {
    if (_disposed) return;
    _store = ChannelSectionStore(
      sections: [
        for (final s in _store.sections)
          if (s.id == sectionId)
            ChannelSection(
              id: s.id,
              name: newName.trim(),
              icon: s.icon,
              order: s.order,
            )
          else
            s,
      ],
      assignments: _store.assignments,
    );
    _persist();
    markDirty();
  }

  void deleteSection(String sectionId) {
    if (_disposed) return;
    final updatedAssignments = Map<String, String>.from(_store.assignments)
      ..removeWhere((_, sid) => sid == sectionId);
    _store = ChannelSectionStore(
      sections: [
        for (final s in _store.sections)
          if (s.id != sectionId) s,
      ],
      assignments: updatedAssignments,
    );
    _persist();
    markDirty();
  }

  void moveSectionUp(String sectionId) {
    if (_disposed) return;
    final sorted = _sortedSections();
    final idx = sorted.indexWhere((s) => s.id == sectionId);
    if (idx <= 0) return;
    _swapOrders(sorted, idx, idx - 1);
    markDirty();
  }

  void moveSectionDown(String sectionId) {
    if (_disposed) return;
    final sorted = _sortedSections();
    final idx = sorted.indexWhere((s) => s.id == sectionId);
    if (idx < 0 || idx >= sorted.length - 1) return;
    _swapOrders(sorted, idx, idx + 1);
    markDirty();
  }

  void assignChannel(String channelId, String sectionId) {
    if (_disposed) return;
    final updated = Map<String, String>.from(_store.assignments)
      ..[channelId] = sectionId;
    _store = ChannelSectionStore(
      sections: _store.sections,
      assignments: updated,
    );
    _persist();
    markDirty();
  }

  void unassignChannel(String channelId) {
    if (_disposed) return;
    final updated = Map<String, String>.from(_store.assignments)
      ..remove(channelId);
    _store = ChannelSectionStore(
      sections: _store.sections,
      assignments: updated,
    );
    _persist();
    markDirty();
  }

  void markDirty() {
    if (!_remoteEnabled || _disposed) return;
    _publishDebounce?.cancel();
    _publishDebounce = Timer(const Duration(seconds: 5), () {
      _publishDebounce = null;
      unawaited(_publish());
    });
  }

  /// Returns whether the fetch reached the relay (regardless of whether a
  /// remote blob exists).
  Future<bool> _fetchAndMerge() async {
    if (_relaySession == null) return false;
    try {
      final events = await _relaySession.fetchHistory(
        NostrFilter(
          kinds: const [EventKind.readState],
          authors: [pubkey],
          tags: const {
            '#d': ['channel-sections'],
          },
          limit: 1,
        ),
      );
      _mergeEvents(events);
      _persist();
      if (!_disposed) _onChanged();
      return true;
    } catch (error) {
      debugPrint('[ChannelSectionsManager] fetch failed: $error');
      // Local state remains usable when relay is unavailable.
      return false;
    }
  }

  /// Returns whether the live subscription was established.
  Future<bool> _startLiveSubscription() async {
    if (_relaySession == null) return false;
    try {
      _unsubscribe = await _relaySession.subscribe(
        NostrFilter(
          kinds: const [EventKind.readState],
          authors: [pubkey],
          tags: const {
            '#d': ['channel-sections'],
          },
          limit: 1,
        ),
        _handleIncomingEvent,
        onClosed: _handleSubscriptionClosed,
      );
      return true;
    } catch (error) {
      debugPrint('[ChannelSectionsManager] live subscription failed: $error');
      // Non-fatal — local state and history still work; retried by the
      // startup-sync backoff.
      return false;
    }
  }

  /// A relay `CLOSED` can arrive after `subscribe()` already reported
  /// success: the 500ms readiness wait times out silently under load, and
  /// the rate-limit rejection lands later. Without this handler the manager
  /// would keep a dead subscription and never retry — the exact
  /// load-correlated cold-start failure this retry exists for.
  void _handleSubscriptionClosed(String message) {
    if (_disposed) return;
    debugPrint(
      '[ChannelSectionsManager] live subscription closed by relay: $message',
    );
    _unsubscribe = null;
    _scheduleStartupRetry();
  }

  void _mergeEvents(List<NostrEvent> events) {
    for (final event in events) {
      if (event.pubkey != pubkey) continue;
      _mergeEvent(event);
    }
  }

  void _mergeEvent(NostrEvent event) {
    // Only process channel-sections d-tag events.
    final dTag = event.getTagValue('d');
    if (dTag != 'channel-sections') return;

    try {
      final plaintext = _crypto.decrypt(event.content);
      final parsed = jsonDecode(plaintext);
      if (parsed is! Map<String, dynamic>) return;

      final incoming = ChannelSectionStore.fromJson(parsed);

      // Last-write-wins: newer createdAt wins; tie-break by event ID.
      final isNewer =
          event.createdAt > _lastRemoteCreatedAt ||
          (event.createdAt == _lastRemoteCreatedAt &&
              event.id.compareTo(_lastRemoteEventId ?? '') > 0);

      if (isNewer) {
        _lastRemoteCreatedAt = event.createdAt;
        _lastRemoteEventId = event.id;
        _store = incoming;
        _persist();
      }
    } catch (_) {
      // Decryption failure or parse error — keep existing state.
    }
  }

  void _handleIncomingEvent(NostrEvent event) {
    if (_disposed) return;
    _mergeEvent(event);
    if (!_disposed) _onChanged();
  }

  bool _isIdenticalToLastPublished() {
    final last = _lastPublishedStore;
    if (last == null) return false;
    if (last.sections.length != _store.sections.length) return false;
    if (last.assignments.length != _store.assignments.length) return false;
    for (var i = 0; i < _store.sections.length; i++) {
      final a = last.sections[i];
      final b = _store.sections[i];
      if (a.id != b.id ||
          a.name != b.name ||
          a.icon != b.icon ||
          a.order != b.order) {
        return false;
      }
    }
    for (final key in _store.assignments.keys) {
      if (last.assignments[key] != _store.assignments[key]) return false;
    }
    return true;
  }

  Future<void> _publish({bool allowDisposed = false}) async {
    if ((!allowDisposed && _disposed) ||
        !_remoteEnabled ||
        _signedEventRelay == null) {
      return;
    }

    // Read-before-write: merge remote state before publishing
    await _fetchAndMerge();

    // No-op suppression: skip if nothing changed
    if (_isIdenticalToLastPublished()) return;

    try {
      final payload = jsonEncode(_store.toJson());
      final ciphertext = _crypto.encrypt(payload);
      final createdAt = max(currentUnixSeconds(), _lastRemoteCreatedAt + 1);

      await _signedEventRelay.submit(
        kind: EventKind.readState,
        content: ciphertext,
        tags: [
          ['d', 'channel-sections'],
          ['t', 'channel-sections'],
        ],
        createdAt: createdAt,
      );

      _lastRemoteCreatedAt = max(_lastRemoteCreatedAt, createdAt);
      _lastPublishedStore = ChannelSectionStore(
        sections: List.of(_store.sections),
        assignments: Map.of(_store.assignments),
      );
    } catch (error) {
      debugPrint('[ChannelSectionsManager] publish failed: $error');
    }
  }

  void _persist() {
    _storage.write(pubkey, _store);
  }

  List<ChannelSection> _sortedSections() {
    final sorted = _store.sections.toList()
      ..sort((a, b) => a.order.compareTo(b.order));
    return sorted;
  }

  void _swapOrders(List<ChannelSection> sorted, int indexA, int indexB) {
    final orderA = sorted[indexA].order;
    final orderB = sorted[indexB].order;
    final idA = sorted[indexA].id;
    final idB = sorted[indexB].id;

    _store = ChannelSectionStore(
      sections: [
        for (final s in _store.sections)
          if (s.id == idA)
            ChannelSection(id: s.id, name: s.name, icon: s.icon, order: orderB)
          else if (s.id == idB)
            ChannelSection(id: s.id, name: s.name, icon: s.icon, order: orderA)
          else
            s,
      ],
      assignments: _store.assignments,
    );
    _persist();
  }
}
