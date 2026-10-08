import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

import '../../../shared/crypto/nip44.dart';
import '../../../shared/relay/relay.dart';
import '../../../shared/read_state/read_state_time.dart';
import 'channel_stars_storage.dart';

class ChannelStarsCrypto {
  final Uint8List _conversationKey;

  ChannelStarsCrypto(String nsec, String pubkey)
    : _conversationKey = _deriveKey(nsec, pubkey);

  static Uint8List _deriveKey(String nsec, String pubkey) {
    final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
    return getConversationKey(privkeyHex, pubkey);
  }

  String encrypt(String plaintext) => nip44Encrypt(_conversationKey, plaintext);

  String decrypt(String ciphertext) =>
      nip44Decrypt(_conversationKey, ciphertext);
}

class ChannelStarsManager {
  final String pubkey;
  final ChannelStarsStorage _storage;
  final ChannelStarsCrypto _crypto;
  final RelaySessionNotifier? _relaySession;
  final SignedEventRelay? _signedEventRelay;
  final bool _remoteEnabled;
  final VoidCallback _onChanged;

  ChannelStarStore _store;
  ChannelStarStore? _lastPublishedStore;
  Timer? _publishDebounce;
  int _lastRemoteCreatedAt = 0;
  String? _lastRemoteEventId;
  void Function()? _unsubscribe;
  bool _disposed = false;

  /// Base delay for the recovery-read retry backoff. Overridable in tests.
  final Duration _startupRetryBaseDelay;
  Timer? _startupRetryTimer;
  int _startupRetryAttempt = 0;
  bool _headApplied = false;
  bool _liveSettled = false;
  int _localRevision = 0;
  bool _publishPending = false;
  Future<void>? _syncInFlight;
  bool _syncAgain = false;

  ChannelStarsManager({
    required this.pubkey,
    required SharedPreferences prefs,
    required ChannelStarsCrypto crypto,
    required RelaySessionNotifier? relaySession,
    required SignedEventRelay? signedEventRelay,
    required bool remoteEnabled,
    required VoidCallback onChanged,
    @visibleForTesting
    Duration startupRetryBaseDelay = const Duration(seconds: 2),
  }) : _storage = ChannelStarsStorage(prefs),
       _crypto = crypto,
       _relaySession = relaySession,
       _signedEventRelay = signedEventRelay,
       _remoteEnabled = remoteEnabled,
       _onChanged = onChanged,
       _startupRetryBaseDelay = startupRetryBaseDelay,
       _store = ChannelStarsStorage(prefs).read(pubkey);

  ChannelStarStore get store => _store;

  Future<void> initialize() async {
    if (_disposed) return;

    if (!_remoteEnabled || _relaySession == null) {
      _onChanged();
      return;
    }

    await _syncWithRelay();
    _onChanged();
  }

  /// Re-reads the retained head once, e.g. on app foreground resume, to catch
  /// an EVENT a healthy socket never delivered. One shot: skipped while an
  /// edit is pending, not retried on failure, and leaves startup state alone.
  void refreshFromRelay() {
    if (_disposed || !_remoteEnabled || _relaySession == null) return;
    unawaited(_recoverHead());
  }

  /// Applies the retained head and opens the live subscription, retrying with
  /// bounded backoff (2s base, 30s ceiling) until both are done. Transient
  /// live-subscription CLOSED recovery belongs to [RelaySessionNotifier].
  Future<void> _syncWithRelay() {
    if (_disposed) return Future.value();
    final inFlight = _syncInFlight;
    if (inFlight != null) {
      _syncAgain = true;
      return inFlight;
    }
    final sync = _runSyncWithRelay();
    _syncInFlight = sync;
    return sync.whenComplete(() {
      _syncInFlight = null;
      if (_disposed || !_syncAgain) return;
      _syncAgain = false;
      unawaited(_syncWithRelay());
    });
  }

  Future<void> _runSyncWithRelay() async {
    if (!_headApplied) {
      final applied = await _recoverHead();
      if (_disposed) return;
      _headApplied = applied;
    }

    final subscribed = _liveSettled || await _startLiveSubscription();
    if (_disposed) return;

    if (!_headApplied || !subscribed) {
      _scheduleStartupRetry();
    } else {
      _startupRetryAttempt = 0;
    }
  }

  void _scheduleStartupRetry() {
    if (_disposed) return;
    _startupRetryTimer?.cancel();
    final delayMs = min(
      _startupRetryBaseDelay.inMilliseconds << min(_startupRetryAttempt, 5),
      30000,
    );
    _startupRetryAttempt++;
    debugPrint(
      '[ChannelStarsManager] relay sync incomplete; '
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
    _syncAgain = false;

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

  void starChannel(String channelId) {
    if (_disposed) return;
    final entry = ChannelStarEntry(
      starred: true,
      updatedAt: currentUnixSeconds(),
    );
    _store = ChannelStarStore(channels: {..._store.channels, channelId: entry});
    unawaited(_persist());
    _onChanged();
    markDirty();
  }

  void unstarChannel(String channelId) {
    if (_disposed) return;
    final entry = ChannelStarEntry(
      starred: false,
      updatedAt: currentUnixSeconds(),
    );
    _store = ChannelStarStore(channels: {..._store.channels, channelId: entry});
    unawaited(_persist());
    _onChanged();
    markDirty();
  }

  void markDirty() {
    if (!_remoteEnabled || _disposed) return;
    _localRevision++;
    _publishPending = true;
    _publishDebounce?.cancel();
    _publishDebounce = Timer(const Duration(seconds: 5), () {
      _publishDebounce = null;
      unawaited(
        _publish().whenComplete(
          () => _publishPending = _publishDebounce != null,
        ),
      );
    });
  }

  /// A recovery read that loses to a local edit is deferred, never applied:
  /// a peer entry stamped by a skewed clock could otherwise overwrite it.
  Future<bool> _recoverHead() async {
    if (_publishPending) return false;
    final revision = _localRevision;
    final events = await _fetchHead();
    if (events == null || _disposed) return false;
    if (revision != _localRevision || _publishPending) return false;
    final applied = await _applyEvents(events);
    if (!_disposed) _onChanged();
    return applied;
  }

  Future<List<NostrEvent>?> _fetchHead() async {
    try {
      return await _relaySession!.fetchHistory(_filter());
    } catch (error) {
      debugPrint('[ChannelStarsManager] fetch failed: $error');
      return null;
    }
  }

  Future<bool> _startLiveSubscription() async {
    try {
      final unsubscribe = await _relaySession!.subscribe(
        _filter(),
        _handleIncomingEvent,
        // Only terminal rejections reach here; re-sending the same REQ would
        // be rejected again, so the live lane stays settled.
        onClosed: (message) {
          _liveSettled = true;
          debugPrint(
            '[ChannelStarsManager] live subscription closed: $message',
          );
        },
      );
      if (_disposed) {
        unsubscribe();
        return false;
      }
      _unsubscribe = unsubscribe;
      return _liveSettled = true;
    } catch (error) {
      debugPrint('[ChannelStarsManager] live subscription failed: $error');
      return _liveSettled;
    }
  }

  NostrFilter _filter() => NostrFilter(
    kinds: const [EventKind.readState],
    authors: [pubkey],
    tags: const {
      '#d': ['channel-stars'],
    },
    limit: 1,
  );

  /// Relay retains `ORDER BY created_at DESC, id ASC`: at equal second the
  /// lower event ID wins.
  bool _isAfterCursor(int createdAt, String id) =>
      createdAt > _lastRemoteCreatedAt ||
      (createdAt == _lastRemoteCreatedAt &&
          id.compareTo(_lastRemoteEventId ?? '') < 0);

  /// Returns false only when a newer head could not be persisted; the cursor
  /// then stays put so the same head is retried. An undecodable head is
  /// skipped without holding startup open.
  Future<bool> _applyEvents(List<NostrEvent> events) async {
    var applied = true;
    for (final event in events) {
      if (event.pubkey != pubkey || event.getTagValue('d') != 'channel-stars') {
        continue;
      }
      if (!_isAfterCursor(event.createdAt, event.id)) continue;
      try {
        final parsed = jsonDecode(_crypto.decrypt(event.content));
        if (parsed is! Map<String, dynamic>) throw const FormatException();
        // Per-channel merge: keep the entry with the highest updatedAt.
        _store = mergeStores(_store, ChannelStarStore.fromJson(parsed));
      } catch (error) {
        debugPrint('[ChannelStarsManager] undecodable head: $error');
        continue;
      }
      if (!await _persist()) {
        applied = false;
        continue;
      }
      if (_isAfterCursor(event.createdAt, event.id)) {
        _lastRemoteCreatedAt = event.createdAt;
        _lastRemoteEventId = event.id;
      }
    }
    return applied;
  }

  void _handleIncomingEvent(NostrEvent event) {
    if (_disposed) return;
    unawaited(
      _applyEvents([event]).then((_) => _disposed ? null : _onChanged()),
    );
  }

  bool _isIdenticalToLastPublished() {
    final last = _lastPublishedStore;
    if (last == null) return false;
    if (last.channels.length != _store.channels.length) return false;
    for (final key in _store.channels.keys) {
      final lastEntry = last.channels[key];
      final currentEntry = _store.channels[key];
      if (lastEntry == null ||
          lastEntry.starred != currentEntry!.starred ||
          lastEntry.updatedAt != currentEntry.updatedAt) {
        return false;
      }
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
    final events = await _fetchHead();
    if (events != null) await _applyEvents(events);
    if (!_disposed) _onChanged();

    // No-op suppression: skip if nothing changed
    if (_isIdenticalToLastPublished()) return;

    try {
      final payload = jsonEncode(_store.toJson());
      final ciphertext = _crypto.encrypt(payload);
      final createdAt = max(currentUnixSeconds(), _lastRemoteCreatedAt + 1);

      String? signedId;
      await _signedEventRelay.submit(
        kind: EventKind.readState,
        content: ciphertext,
        tags: [
          ['d', 'channel-stars'],
          ['t', 'channel-stars'],
        ],
        createdAt: createdAt,
        onSigned: (event) => signedId = event.id,
      );

      // Keep the cursor a coherent (created_at, id) pair so an OK that beats
      // its own echo cannot make later same-second heads compare against a
      // stale ID. A newer head learned during the await stays.
      if (signedId != null && _isAfterCursor(createdAt, signedId!)) {
        _lastRemoteCreatedAt = createdAt;
        _lastRemoteEventId = signedId;
      }
      _lastPublishedStore = ChannelStarStore(channels: Map.of(_store.channels));
    } catch (error) {
      debugPrint('[ChannelStarsManager] publish failed: $error');
    }
  }

  Future<bool> _persist() async {
    try {
      if (await _storage.write(pubkey, _store)) return true;
      debugPrint('[ChannelStarsManager] persist returned false');
    } catch (error) {
      debugPrint('[ChannelStarsManager] persist failed: $error');
    }
    return false;
  }
}
