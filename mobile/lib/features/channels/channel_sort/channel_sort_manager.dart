import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

import '../../../shared/crypto/nip44.dart';
import '../../../shared/relay/read_state_snapshot.dart';
import '../../../shared/relay/relay.dart';
import '../../../shared/read_state/read_state_time.dart';
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
  ChannelSortSyncState _syncState;

  /// Desktop's unified sort head; when present it replaces [_store] for
  /// display and legacy edits stop.
  ChannelSortStore? _workspace;
  int _workspaceCreatedAt = 0;
  String _workspaceEventId = '';
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
       _store = ChannelSortStorage(prefs).read(pubkey, relayUrl),
       _syncState = ChannelSortStorage(prefs).readSyncState(pubkey, relayUrl) {
    _lastRemoteCreatedAt = _syncState.updatedAt;
    _lastRemoteEventId = _syncState.eventId;
    final cached = _storage.readWorkspace(pubkey, relayUrl);
    if (cached != null) {
      _workspace = cached.store;
      _workspaceCreatedAt = cached.createdAt;
      _workspaceEventId = cached.eventId;
    }
  }

  /// The sort choices desktop renders with.
  ChannelSortStore get store => _workspace ?? _store;

  /// Whether a legacy `channel-sort` edit still reaches desktop.
  bool get editable => _workspace == null;

  ChannelSortMode sortModeFor(String groupKey) =>
      store.groups[groupKey] ?? defaultSortModeFor(groupKey);

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
    if (_syncState.hasPendingLocalChanges ||
        (!firstFetch && !secondFetch && _store.groups.isNotEmpty)) {
      _schedulePublish();
    }
  }

  /// Re-reads the retained heads once, e.g. on app foreground resume, to catch
  /// an EVENT a healthy socket never delivered. A pending or newer local edit
  /// fences only the legacy head it would replace; the unified head always
  /// applies, since it ends legacy editing.
  void refreshFromRelay() {
    if (_disposed || !_remoteEnabled) return;
    final generation = _generation;
    final pending = _syncState.hasPendingLocalChanges;
    unawaited(
      _fetchAndApply(
        legacyIsCurrent: () => !pending && generation == _generation,
      ).then((_) {
        if (!_disposed && generation != _generation) _onChanged();
      }),
    );
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
    if (_disposed || !editable || _store.groups[groupKey] == mode) return;
    final updated = ChannelSortStore(
      groups: {..._store.groups, groupKey: mode},
    );
    _store = liveSectionIds == null
        ? updated
        : stripOrphanedSectionModes(updated, liveSectionIds);
    final now = currentUnixSeconds();
    final pendingUpdatedAt = min(
      max(now, _syncState.pendingUpdatedAt + 1),
      now + _maxClockDriftSeconds,
    );
    _syncState = ChannelSortSyncState(
      updatedAt: _syncState.updatedAt,
      eventId: _syncState.eventId,
      hasPendingLocalChanges: true,
      pendingUpdatedAt: pendingUpdatedAt,
    );
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

  Future<bool?> _fetchAndApply({bool Function()? legacyIsCurrent}) async {
    if (_relaySession == null) return null;
    try {
      final events = await _relaySession.fetchHistory(_filter());
      if (_disposed) return null;
      final legacy = legacyIsCurrent?.call() ?? true;
      // The result reports only the legacy head: it drives legacy republish.
      var found = false;
      for (final event in events) {
        if (event.pubkey != pubkey) continue;
        final dTag = event.getTagValue('d');
        if (dTag == workspaceSortCoordinate) {
          _applyWorkspace(event);
        } else if (dTag == _dTag && legacy) {
          found = true;
          _applyRemote(event);
        }
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
      _unsubscribe = await _relaySession.subscribeWithStatus(
        _filter(),
        _handleIncomingEvent,
        onStatusChanged: (_) {},
        onClosed: (_) {
          if (_disposed) return;
          _unsubscribe?.call();
          _unsubscribe = null;
          _scheduleStartupRetry();
        },
        // One coordinate's newer head must not bound the other's replay.
        resumeSince: false,
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
      '#d': [_dTag, workspaceSortCoordinate],
    },
    limit: 2,
  );

  void _applyWorkspace(NostrEvent event) {
    // The cursor and lock trust this head's timestamp and ID, which only a
    // checked signature vouches for.
    if (!isAuthenticReadStateEvent(event, viewer: pubkey) ||
        event.createdAt > currentUnixSeconds() + _maxClockDriftSeconds ||
        event.createdAt < _workspaceCreatedAt ||
        (event.createdAt == _workspaceCreatedAt &&
            event.id.compareTo(_workspaceEventId) >= 0)) {
      return;
    }
    try {
      final parsed = jsonDecode(_crypto.decrypt(event.content));
      if (parsed is! Map<String, dynamic> || parsed['version'] != 1) return;
      _workspace = ChannelSortStore.fromWorkspaceJson(parsed);
      _workspaceCreatedAt = event.createdAt;
      _workspaceEventId = event.id;
      // Legacy edits no longer reach desktop; drop any still queued.
      _publishDebounce?.cancel();
      _publishDebounce = null;
      _generation++;
      if (!_disposed) {
        _storage.writeWorkspace(
          pubkey,
          relayUrl,
          _workspace!,
          event.createdAt,
          event.id,
        );
      }
    } catch (_) {
      // Keep the last readable head without advancing the cursor.
    }
  }

  void _applyRemote(NostrEvent event) {
    if (event.createdAt > currentUnixSeconds() + _maxClockDriftSeconds) return;
    if (_syncState.hasPendingLocalChanges &&
        event.createdAt <= _syncState.pendingUpdatedAt) {
      return;
    }
    final isNewer =
        event.createdAt > _lastRemoteCreatedAt ||
        (event.createdAt == _lastRemoteCreatedAt &&
            event.id.compareTo(_lastRemoteEventId) < 0);
    if (!isNewer) return;
    try {
      final parsed = jsonDecode(_crypto.decrypt(event.content));
      if (parsed is! Map<String, dynamic> || parsed['version'] != 1) return;
      final incoming = ChannelSortStore.fromJson(parsed);
      _lastRemoteCreatedAt = event.createdAt;
      _lastRemoteEventId = event.id;
      _syncState = ChannelSortSyncState(
        updatedAt: event.createdAt,
        eventId: event.id,
      );
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
    if (_disposed || event.pubkey != pubkey) return;
    final dTag = event.getTagValue('d');
    final before = _generation;
    if (dTag == workspaceSortCoordinate) {
      _applyWorkspace(event);
    } else if (dTag == _dTag) {
      _applyRemote(event);
    }
    if (_generation != before && !_disposed) _onChanged();
  }

  Future<void> _publish() async {
    if (_disposed ||
        !editable ||
        !_remoteEnabled ||
        _signedEventRelay == null) {
      return;
    }
    final generationAtStart = _generation;
    // A newer remote blob wins. If one arrives during this read, _generation
    // changes and we abort rather than overwriting it.
    final preflight = await _fetchAndApply();
    if (_disposed) return;
    if (_generation != generationAtStart) {
      // The preflight may be the first read to find the unified head.
      _onChanged();
      return;
    }
    if (preflight == null) {
      _schedulePublish();
      return;
    }
    try {
      final now = currentUnixSeconds();
      final createdAt = max(now, _lastRemoteCreatedAt + 1);
      if (createdAt > now + _maxClockDriftSeconds) {
        debugPrint(
          '[ChannelSortManager] publish delayed: relay cursor '
          'is ${createdAt - now}s ahead of local time',
        );
        _schedulePublish();
        return;
      }
      final ciphertext = _crypto.encrypt(jsonEncode(_store.toJson()));
      String? submittedEventId;
      await _signedEventRelay.submit(
        kind: EventKind.readState,
        content: ciphertext,
        tags: const [
          ['d', _dTag],
          ['t', _dTag],
        ],
        createdAt: createdAt,
        onSigned: (event) => submittedEventId = event.id,
        // A unified head can arrive during the relay's rate-limit wait.
        mayPublish: () async =>
            !_disposed && editable && _generation == generationAtStart,
      );
      if (_disposed || _generation != generationAtStart) return;
      // Keep local publications strictly ordered for this manager lifetime, as
      // desktop does, but never persist that volatile publication cursor.
      _lastRemoteCreatedAt = createdAt;
      _lastRemoteEventId = submittedEventId ?? '';
      _syncState = ChannelSortSyncState(
        updatedAt: _syncState.updatedAt,
        eventId: _syncState.eventId,
      );
      _persist();
    } catch (error) {
      debugPrint('[ChannelSortManager] publish failed: $error');
    }
  }

  void _persist() {
    _storage.write(pubkey, relayUrl, _store);
    _storage.writeSyncState(pubkey, relayUrl, _syncState);
  }

  void dispose() {
    if (_disposed) return;
    _disposed = true;
    _publishDebounce?.cancel();
    _startupRetryTimer?.cancel();
    _unsubscribe?.call();
    _unsubscribe = null;
  }
}
