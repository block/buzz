import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

import '../../../shared/crypto/nip44.dart';
import '../../../shared/relay/relay.dart';
import '../read_state/read_state_time.dart';
import 'channel_mutes_storage.dart';

class ChannelMutesCrypto {
  final Uint8List _conversationKey;

  ChannelMutesCrypto(String nsec, String pubkey)
    : _conversationKey = _deriveKey(nsec, pubkey);

  static Uint8List _deriveKey(String nsec, String pubkey) {
    final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
    return getConversationKey(privkeyHex, pubkey);
  }

  String encrypt(String plaintext) => nip44Encrypt(_conversationKey, plaintext);

  String decrypt(String ciphertext) =>
      nip44Decrypt(_conversationKey, ciphertext);
}

class ChannelMutesManager {
  final String pubkey;
  final ChannelMutesStorage _storage;
  final ChannelMutesCrypto _crypto;
  final RelaySessionNotifier? _relaySession;
  final SignedEventRelay? _signedEventRelay;
  final bool _remoteEnabled;
  final VoidCallback _onChanged;

  ChannelMuteStore _store;
  ChannelMuteStore? _lastPublishedStore;
  Timer? _publishDebounce;
  int _lastRemoteCreatedAt = 0;
  String? _lastRemoteEventId;
  void Function()? _unsubscribe;
  bool _disposed = false;

  ChannelMutesManager({
    required this.pubkey,
    required SharedPreferences prefs,
    required ChannelMutesCrypto crypto,
    required RelaySessionNotifier? relaySession,
    required SignedEventRelay? signedEventRelay,
    required bool remoteEnabled,
    required VoidCallback onChanged,
  }) : _storage = ChannelMutesStorage(prefs),
       _crypto = crypto,
       _relaySession = relaySession,
       _signedEventRelay = signedEventRelay,
       _remoteEnabled = remoteEnabled,
       _onChanged = onChanged,
       _store = ChannelMutesStorage(prefs).read(pubkey);

  ChannelMuteStore get store => _store;

  Future<void> initialize() async {
    if (_disposed) return;

    if (!_remoteEnabled || _relaySession == null) {
      _onChanged();
      return;
    }

    await _fetchAndMerge();
    await _startLiveSubscription();
    _onChanged();
  }

  void dispose({bool flushPending = true}) {
    if (_disposed) return;
    _disposed = true;

    final hadPending = _publishDebounce != null;
    _publishDebounce?.cancel();
    _publishDebounce = null;

    if (flushPending && hadPending && _remoteEnabled) {
      unawaited(_publish(allowDisposed: true));
    }

    _unsubscribe?.call();
    _unsubscribe = null;
  }

  void muteChannel(String channelId) {
    if (_disposed) return;
    final entry = ChannelMuteEntry(
      muted: true,
      updatedAt: currentUnixSeconds(),
    );
    _store = ChannelMuteStore(channels: {..._store.channels, channelId: entry});
    _persist();
    markDirty();
  }

  void unmuteChannel(String channelId) {
    if (_disposed) return;
    final entry = ChannelMuteEntry(
      muted: false,
      updatedAt: currentUnixSeconds(),
    );
    _store = ChannelMuteStore(channels: {..._store.channels, channelId: entry});
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

  Future<void> _fetchAndMerge() async {
    if (_relaySession == null) return;
    try {
      final events = await _relaySession.fetchHistory(
        NostrFilter(
          kinds: const [EventKind.readState],
          authors: [pubkey],
          tags: const {
            '#d': ['channel-mutes'],
          },
          limit: 1,
        ),
      );
      _mergeEvents(events);
      _persist();
      if (!_disposed) _onChanged();
    } catch (_) {
      // Local state remains usable when relay is unavailable.
    }
  }

  Future<void> _startLiveSubscription() async {
    if (_relaySession == null) return;
    try {
      _unsubscribe = await _relaySession.subscribe(
        NostrFilter(
          kinds: const [EventKind.readState],
          authors: [pubkey],
          tags: const {
            '#d': ['channel-mutes'],
          },
          limit: 1,
        ),
        _handleIncomingEvent,
      );
    } catch (_) {
      // Non-fatal — local state and history still work.
    }
  }

  void _mergeEvents(List<NostrEvent> events) {
    for (final event in events) {
      if (event.pubkey != pubkey) continue;
      _mergeEvent(event);
    }
  }

  void _mergeEvent(NostrEvent event) {
    // Only process channel-mutes d-tag events.
    final dTag = event.getTagValue('d');
    if (dTag != 'channel-mutes') return;

    try {
      final plaintext = _crypto.decrypt(event.content);
      final parsed = jsonDecode(plaintext);
      if (parsed is! Map<String, dynamic>) return;

      final incoming = ChannelMuteStore.fromJson(parsed);

      // Gate on createdAt: ignore events older than what we've already seen.
      final isNewer =
          event.createdAt > _lastRemoteCreatedAt ||
          (event.createdAt == _lastRemoteCreatedAt &&
              event.id.compareTo(_lastRemoteEventId ?? '') > 0);

      if (isNewer) {
        _lastRemoteCreatedAt = event.createdAt;
        _lastRemoteEventId = event.id;
        // Per-channel merge: keep the entry with the highest updatedAt for each channel.
        _store = mergeStores(_store, incoming);
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
    if (last.channels.length != _store.channels.length) return false;
    for (final key in _store.channels.keys) {
      final lastEntry = last.channels[key];
      final currentEntry = _store.channels[key];
      if (lastEntry == null ||
          lastEntry.muted != currentEntry!.muted ||
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
          ['d', 'channel-mutes'],
          ['t', 'channel-mutes'],
        ],
        createdAt: createdAt,
      );

      _lastRemoteCreatedAt = max(_lastRemoteCreatedAt, createdAt);
      _lastPublishedStore = ChannelMuteStore(channels: Map.of(_store.channels));
    } catch (error) {
      debugPrint('[ChannelMutesManager] publish failed: $error');
    }
  }

  void _persist() {
    _storage.write(pubkey, _store);
  }
}
