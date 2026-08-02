import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

import '../../../shared/crypto/nip44.dart';
import '../../../shared/relay/relay.dart';
import '../read_state/read_state_time.dart';
import 'channel_sort_storage.dart';

const _dTag = 'channel-sort';
const _maxClockDriftSeconds = 300;

class ChannelSortCrypto {
  final Uint8List _conversationKey;

  ChannelSortCrypto(String nsec, String pubkey)
    : _conversationKey = _deriveKey(nsec, pubkey);

  static Uint8List _deriveKey(String nsec, String pubkey) {
    final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
    return getConversationKey(privkeyHex, pubkey);
  }

  String encrypt(String plaintext) => nip44Encrypt(_conversationKey, plaintext);
  String decrypt(String ciphertext) =>
      nip44Decrypt(_conversationKey, ciphertext);
}

/// Desktop-compatible encrypted NIP-78 sync for per-group sort preferences.
/// Remote state is ordinary whole-blob LWW: unlike the rejected #2829 design,
/// local state never vetoes a newer remote blob or leapfrogs an unseen edit.
class ChannelSortManager {
  final String pubkey;
  final String relayUrl;
  final ChannelSortStorage _storage;
  final ChannelSortCrypto _crypto;
  final RelaySessionNotifier? _relaySession;
  final SignedEventRelay? _signedEventRelay;
  final bool _remoteEnabled;
  final VoidCallback _onChanged;
  final Duration _startupRetryBaseDelay;
  final Duration _publishDelay;

  ChannelSortStore _store;
  Timer? _publishDebounce;
  Timer? _startupRetryTimer;
  int _startupRetryAttempt = 0;
  int _lastRemoteCreatedAt = 0;
  String _lastRemoteEventId = '';
  int _generation = 0;
  void Function()? _unsubscribe;
  bool _disposed = false;

  ChannelSortManager({
    required this.pubkey,
    required this.relayUrl,
    required SharedPreferences prefs,
    required ChannelSortCrypto crypto,
    required RelaySessionNotifier? relaySession,
    required SignedEventRelay? signedEventRelay,
    required bool remoteEnabled,
    required VoidCallback onChanged,
    @visibleForTesting
    Duration startupRetryBaseDelay = const Duration(seconds: 2),
    @visibleForTesting Duration publishDelay = const Duration(seconds: 2),
  }) : _storage = ChannelSortStorage(prefs),
       _crypto = crypto,
       _relaySession = relaySession,
       _signedEventRelay = signedEventRelay,
       _remoteEnabled = remoteEnabled,
       _onChanged = onChanged,
       _startupRetryBaseDelay = startupRetryBaseDelay,
       _publishDelay = publishDelay,
       _store = ChannelSortStorage(prefs).read(pubkey, relayUrl);

  ChannelSortStore get store => _store;
  ChannelSortMode sortModeFor(String groupKey) =>
      _store.groups[groupKey] ?? kDefaultSortMode;

  Future<void> initialize() async {
    if (_disposed || !_remoteEnabled || _relaySession == null) {
      if (!_disposed) _onChanged();
      return;
    }
    await _syncWithRelay();
    if (!_disposed) _onChanged();
  }

  Future<void> _syncWithRelay() async {
    final firstFetch = await _fetchAndApply();
    final subscribed = _unsubscribe != null || await _startLiveSubscription();
    // Fetch again after the subscription is ready. This closes the event gap
    // between history and live setup (and catches anything published while a
    // rate-limited subscription was retrying).
    final secondFetch = subscribed ? await _fetchAndApply() : null;
    if (firstFetch == null || !subscribed || secondFetch == null) {
      _scheduleStartupRetry();
      return;
    }
    _startupRetryAttempt = 0;
    if (!firstFetch && !secondFetch && _store.groups.isNotEmpty) {
      _schedulePublish();
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
    _startupRetryTimer = Timer(Duration(milliseconds: delayMs), () {
      _startupRetryTimer = null;
      unawaited(
        _syncWithRelay().then((_) {
          if (!_disposed) _onChanged();
        }),
      );
    });
  }

  void setSortModeFor(
    String groupKey,
    ChannelSortMode mode, {
    Iterable<String>? liveSectionIds,
  }) {
    if (_disposed || _store.groups[groupKey] == mode) return;
    final updated = ChannelSortStore(
      groups: {..._store.groups, groupKey: mode},
    );
    _store = liveSectionIds == null
        ? updated
        : stripOrphanedSectionModes(updated, liveSectionIds);
    _generation++;
    _persist();
    _schedulePublish();
    _onChanged();
  }

  void _schedulePublish() {
    if (!_remoteEnabled || _disposed) return;
    _publishDebounce?.cancel();
    _publishDebounce = Timer(_publishDelay, () {
      _publishDebounce = null;
      unawaited(_publish());
    });
  }

  Future<bool?> _fetchAndApply() async {
    if (_relaySession == null) return null;
    try {
      final events = await _relaySession.fetchHistory(_filter());
      var found = false;
      for (final event in events) {
        if (event.pubkey != pubkey || event.getTagValue('d') != _dTag) continue;
        found = true;
        _applyRemote(event);
      }
      return found;
    } catch (error) {
      debugPrint('[ChannelSortManager] fetch failed: $error');
      return null;
    }
  }

  Future<bool> _startLiveSubscription() async {
    if (_relaySession == null) return false;
    try {
      _unsubscribe = await _relaySession.subscribe(
        _filter(),
        _handleIncomingEvent,
        onClosed: (_) {
          if (_disposed) return;
          _unsubscribe?.call();
          _unsubscribe = null;
          _scheduleStartupRetry();
        },
      );
      return true;
    } catch (error) {
      debugPrint('[ChannelSortManager] subscribe failed: $error');
      return false;
    }
  }

  NostrFilter _filter() => NostrFilter(
    kinds: const [EventKind.readState],
    authors: [pubkey],
    tags: const {
      '#d': [_dTag],
    },
    limit: 1,
  );

  void _applyRemote(NostrEvent event) {
    if (event.createdAt > currentUnixSeconds() + _maxClockDriftSeconds) return;
    final isNewer =
        event.createdAt > _lastRemoteCreatedAt ||
        (event.createdAt == _lastRemoteCreatedAt &&
            event.id.compareTo(_lastRemoteEventId) > 0);
    if (!isNewer) return;
    try {
      final parsed = jsonDecode(_crypto.decrypt(event.content));
      if (parsed is! Map<String, dynamic> || parsed['version'] != 1) return;
      final incoming = ChannelSortStore.fromJson(parsed);
      _lastRemoteCreatedAt = event.createdAt;
      _lastRemoteEventId = event.id;
      _publishDebounce?.cancel();
      _publishDebounce = null;
      _store = incoming;
      _generation++;
      _persist();
    } catch (_) {
      // Ignore malformed or undecryptable blobs without advancing the cursor.
    }
  }

  void _handleIncomingEvent(NostrEvent event) {
    if (_disposed ||
        event.pubkey != pubkey ||
        event.getTagValue('d') != _dTag) {
      return;
    }
    final before = _generation;
    _applyRemote(event);
    if (_generation != before && !_disposed) _onChanged();
  }

  Future<void> _publish() async {
    if (_disposed || !_remoteEnabled || _signedEventRelay == null) return;
    final generationAtStart = _generation;
    // A newer remote blob wins. If one arrives during this read, _generation
    // changes and we abort rather than overwriting it.
    await _fetchAndApply();
    if (_disposed || _generation != generationAtStart) return;
    try {
      final now = currentUnixSeconds();
      final createdAt = max(now, _lastRemoteCreatedAt + 1);
      if (createdAt > now + _maxClockDriftSeconds) return;
      final ciphertext = _crypto.encrypt(jsonEncode(_store.toJson()));
      await _signedEventRelay.submit(
        kind: EventKind.readState,
        content: ciphertext,
        tags: const [
          ['d', _dTag],
          ['t', _dTag],
        ],
        createdAt: createdAt,
      );
      if (_disposed || _generation != generationAtStart) return;
      _lastRemoteCreatedAt = createdAt;
    } catch (error) {
      debugPrint('[ChannelSortManager] publish failed: $error');
    }
  }

  void _persist() => _storage.write(pubkey, relayUrl, _store);

  void dispose() {
    if (_disposed) return;
    _disposed = true;
    _publishDebounce?.cancel();
    _startupRetryTimer?.cancel();
    _unsubscribe?.call();
    _unsubscribe = null;
  }
}
