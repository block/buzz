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

  ChannelSectionsManager({
    required this.pubkey,
    required SharedPreferences prefs,
    required ChannelSectionsCrypto crypto,
    required RelaySessionNotifier? relaySession,
    required SignedEventRelay? signedEventRelay,
    required bool remoteEnabled,
    required VoidCallback onChanged,
  }) : _storage = ChannelSectionsStorage(prefs),
       _crypto = crypto,
       _relaySession = relaySession,
       _signedEventRelay = signedEventRelay,
       _remoteEnabled = remoteEnabled,
       _onChanged = onChanged,
       _store = ChannelSectionsStorage(prefs).read(pubkey);

  ChannelSectionStore get store => _store;

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
            ChannelSection(id: s.id, name: newName.trim(), order: s.order)
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

  Future<void> _fetchAndMerge() async {
    if (_relaySession == null) return;
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
            '#d': ['channel-sections'],
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
      if (a.id != b.id || a.name != b.name || a.order != b.order) return false;
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
            ChannelSection(id: s.id, name: s.name, order: orderB)
          else if (s.id == idB)
            ChannelSection(id: s.id, name: s.name, order: orderA)
          else
            s,
      ],
      assignments: _store.assignments,
    );
    _persist();
  }
}
