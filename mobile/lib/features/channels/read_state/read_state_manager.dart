import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

import '../../../shared/crypto/nip44.dart';
import '../../../shared/relay/relay.dart';
import 'read_state_format.dart';
import 'read_state_storage.dart';
import 'read_state_time.dart';

class ReadStateCrypto {
  final Uint8List conversationKey;

  const ReadStateCrypto._(this.conversationKey);

  static ReadStateCrypto? tryCreate({
    required String nsec,
    required String pubkey,
  }) {
    try {
      final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
      if (privkeyHex.isEmpty || pubkey.isEmpty) {
        return null;
      }
      return ReadStateCrypto._(getConversationKey(privkeyHex, pubkey));
    } catch (e) {
      debugPrint('[ReadStateManager] crypto init failed: $e');
      return null;
    }
  }

  String encrypt(String plaintext) => nip44Encrypt(conversationKey, plaintext);

  String decrypt(String ciphertext) =>
      nip44Decrypt(conversationKey, ciphertext);
}

class ReadStateManager {
  final String pubkey;
  final ReadStateCrypto _crypto;
  final ReadStateStorage _storage;
  final RelaySessionNotifier? _relaySession;
  final SignedEventRelay? _signedEventRelay;
  final bool _remoteEnabled;
  final VoidCallback _onChanged;

  late final String _clientId;
  late String _slotId;

  final Map<String, int> _effectiveState = {};
  final Set<String> _publishableContextIds = {};
  Map<String, int> _lastPublishedContexts = {};

  Timer? _debounceTimer;
  void Function()? _unsubscribeLive;
  bool _initialized = false;
  bool _disposed = false;
  bool _isPublishing = false;
  Completer<void>? _publishCompleter;
  bool _remoteUnsupported = false;
  int _maxFetchedCreatedAt = 0;
  final Set<String> _forcedContextIds = {};
  final Map<String, int> _contextSourceCreatedAt = {};
  final Set<String> _pendingSyncedRollbacks = {};

  ReadStateManager({
    required this.pubkey,
    required SharedPreferences prefs,
    required ReadStateCrypto crypto,
    required RelaySessionNotifier? relaySession,
    required SignedEventRelay? signedEventRelay,
    required bool remoteEnabled,
    required VoidCallback onChanged,
  }) : _crypto = crypto,
       _storage = ReadStateStorage(prefs),
       _relaySession = relaySession,
       _signedEventRelay = signedEventRelay,
       _remoteEnabled = remoteEnabled,
       _onChanged = onChanged {
    _clientId = _storage.getOrCreateClientId(pubkey);
    _slotId = _storage.getOrCreateSlotId(pubkey);
    _hydrateFromLocalStorage();
  }

  Map<String, int> get effectiveContexts => Map.unmodifiable(_effectiveState);

  int? getEffectiveTimestamp(String contextId) => _effectiveState[contextId];

  Future<void> initialize() async {
    if (_initialized || _disposed) return;
    _initialized = true;
    debugPrint(
      '[ReadStateManager] initialize pubkey=${pubkey.substring(0, 8)}… clientId=${_clientId.substring(0, 8)}… slotId=$_slotId',
    );

    if (!_remoteEnabled || _relaySession == null) {
      _onChanged();
      return;
    }

    await _fetchAndMerge();
    await _startLiveSubscription();
    if (!_isIdenticalToLastPublished(_currentContexts())) {
      _schedulePublish();
    }

    _onChanged();
    debugPrint(
      '[ReadStateManager] initialize complete maxFetchedCreatedAt=$_maxFetchedCreatedAt contexts=${_effectiveState.length}',
    );
  }

  void markContextRead(String contextId, int unixTimestamp) {
    _forcedContextIds.remove(contextId);
    _advanceContext(contextId, unixTimestamp, publishable: true);
    _contextSourceCreatedAt[contextId] = max(
      currentUnixSeconds(),
      _maxFetchedCreatedAt + 1,
    );
  }

  void markContextUnread(String contextId, int lastMessageTimestamp) {
    if (_disposed || lastMessageTimestamp <= 0) return;
    final rollbackTo = lastMessageTimestamp - 1;
    _effectiveState[contextId] = rollbackTo;
    _publishableContextIds.add(contextId);
    _forcedContextIds.add(contextId);
    _persistLocalState();
    _onChanged();
    _schedulePublish();
  }

  void seedContextRead(String contextId, int unixTimestamp) {
    _advanceContext(contextId, unixTimestamp, publishable: false);
  }

  Future<void> flush() async {
    _debounceTimer?.cancel();
    _debounceTimer = null;
    if (!_remoteEnabled || _remoteUnsupported || _disposed) return;
    await _publish();
  }

  Future<void> reinitializeRemote() async {
    if (_disposed || !_remoteEnabled) return;
    debugPrint('[ReadStateManager] reinitializeRemote');
    if (_isPublishing) {
      await _publishCompleter?.future;
    }
    _unsubscribeLive?.call();
    _unsubscribeLive = null;
    await _fetchAndMerge();
    await _startLiveSubscription();
    if (!_isIdenticalToLastPublished(_currentContexts())) {
      _schedulePublish();
    }
    _onChanged();
  }

  void dispose({bool flushPending = true}) {
    if (_disposed) return;
    _disposed = true;

    final hadPendingPublish = _debounceTimer != null;
    _debounceTimer?.cancel();
    _debounceTimer = null;

    if (flushPending &&
        hadPendingPublish &&
        _remoteEnabled &&
        !_remoteUnsupported) {
      unawaited(_publish(allowDisposed: true));
    }

    _unsubscribeLive?.call();
    _unsubscribeLive = null;
  }

  void _advanceContext(
    String contextId,
    int unixTimestamp, {
    required bool publishable,
  }) {
    if (_disposed || unixTimestamp < 0) return;

    final current = _effectiveState[contextId] ?? 0;
    if (unixTimestamp <= current) {
      if (!publishable || _publishableContextIds.contains(contextId)) {
        return;
      }

      _publishableContextIds.add(contextId);
      _persistLocalState();
      _onChanged();
      _schedulePublish();
      return;
    }

    _effectiveState[contextId] = unixTimestamp;
    if (publishable) {
      _publishableContextIds.add(contextId);
    }
    _persistLocalState();
    _onChanged();
    if (publishable) {
      _schedulePublish();
    }
  }

  Future<void> _fetchAndMerge() async {
    try {
      final events = await _relaySession!.fetchHistory(
        NostrFilter(
          kinds: const [EventKind.readState],
          authors: [pubkey],
          tags: const {
            '#t': ['read-state'],
          },
          since: currentUnixSeconds() - readStateHorizonSeconds,
          limit: readStateFetchLimit,
        ),
      );
      _mergeEvents(events);
      _persistLocalState();
      _onChanged();
    } catch (e) {
      debugPrint('[ReadStateManager] fetchAndMerge failed: $e');
    }
  }

  void _mergeEvents(List<NostrEvent> events) {
    ReadStateBlob? ownBlob;
    var ownBlobCreatedAt = 0;

    for (final event in events) {
      final decoded = decodeReadStateEvent(
        event,
        pubkey: pubkey,
        decrypt: _crypto.decrypt,
      );
      if (decoded == null) continue;

      if (_isPlausibleCreatedAt(event.createdAt)) {
        _maxFetchedCreatedAt = max(_maxFetchedCreatedAt, event.createdAt);
      }

      if (decoded.dTag == '$readStateDTagPrefix$_slotId' &&
          decoded.blob.clientId != _clientId) {
        _rotateSlotId();
      }

      for (final entry in decoded.blob.contexts.entries) {
        if (_forcedContextIds.contains(entry.key)) continue;
        final sourceCreatedAt = _contextSourceCreatedAt[entry.key] ?? 0;
        final current = _effectiveState[entry.key] ?? 0;
        if (event.createdAt > sourceCreatedAt) {
          _effectiveState[entry.key] = entry.value;
          _contextSourceCreatedAt[entry.key] = event.createdAt;
        } else if (event.createdAt == sourceCreatedAt &&
            entry.value != current) {
          _effectiveState[entry.key] = entry.value;
        }
        _publishableContextIds.add(entry.key);
      }

      if (decoded.blob.clientId == _clientId &&
          event.createdAt > ownBlobCreatedAt) {
        ownBlob = decoded.blob;
        ownBlobCreatedAt = event.createdAt;
      }
    }

    if (ownBlob != null) {
      _lastPublishedContexts = Map<String, int>.from(ownBlob.contexts);
      _publishableContextIds.addAll(ownBlob.contexts.keys);
    }
  }

  Future<void> _startLiveSubscription() async {
    try {
      _unsubscribeLive = await _relaySession!.subscribe(
        NostrFilter(
          kinds: const [EventKind.readState],
          authors: [pubkey],
          tags: const {
            '#t': ['read-state'],
          },
          limit: readStateFetchLimit,
        ),
        _handleIncomingEvent,
      );
      debugPrint('[ReadStateManager] live subscription established');
    } catch (e) {
      debugPrint('[ReadStateManager] live subscription FAILED: $e');
    }
  }

  void _handleIncomingEvent(NostrEvent event) {
    if (_disposed) return;
    debugPrint(
      '[ReadStateManager] incoming event=${event.id.substring(0, 8)}… created_at=${event.createdAt}',
    );

    final decoded = decodeReadStateEvent(
      event,
      pubkey: pubkey,
      decrypt: _crypto.decrypt,
    );
    if (decoded == null) return;

    if (_isPlausibleCreatedAt(event.createdAt)) {
      _maxFetchedCreatedAt = max(_maxFetchedCreatedAt, event.createdAt);
    }

    if (decoded.dTag == '$readStateDTagPrefix$_slotId' &&
        decoded.blob.clientId != _clientId) {
      _rotateSlotId();
    }

    var changed = false;
    for (final entry in decoded.blob.contexts.entries) {
      if (_forcedContextIds.contains(entry.key)) continue;
      final sourceCreatedAt = _contextSourceCreatedAt[entry.key] ?? 0;
      final current = _effectiveState[entry.key] ?? 0;
      if (event.createdAt > sourceCreatedAt) {
        if (_effectiveState[entry.key] != entry.value) {
          if (entry.value < current && current > 0) {
            _pendingSyncedRollbacks.add(entry.key);
            debugPrint(
              '[ReadStateManager] synced rollback ctx=${entry.key.substring(0, min(12, entry.key.length))}… from=$current to=${entry.value}',
            );
          }
          _effectiveState[entry.key] = entry.value;
          changed = true;
        }
        _contextSourceCreatedAt[entry.key] = event.createdAt;
      } else if (event.createdAt == sourceCreatedAt && entry.value != current) {
        _effectiveState[entry.key] = entry.value;
        changed = true;
      }
      if (_publishableContextIds.add(entry.key)) {
        changed = true;
      }
    }
    debugPrint(
      '[ReadStateManager] incoming result changed=$changed clientId=${decoded.blob.clientId.substring(0, min(8, decoded.blob.clientId.length))}…',
    );

    if (decoded.blob.clientId == _clientId) {
      _lastPublishedContexts = Map<String, int>.from(decoded.blob.contexts);
    }

    if (changed) {
      _persistLocalState();
      _onChanged();
    }

    if (decoded.blob.clientId != _clientId &&
        !_isIdenticalToLastPublished(_currentContexts())) {
      _schedulePublish();
    }
  }

  void _schedulePublish() {
    if (!_remoteEnabled || _remoteUnsupported || _disposed) return;

    _debounceTimer?.cancel();
    _debounceTimer = Timer(const Duration(seconds: 5), () {
      _debounceTimer = null;
      unawaited(_publish());
    });
  }

  Future<void> _publish({bool allowDisposed = false}) async {
    if ((!allowDisposed && _disposed) ||
        !_remoteEnabled ||
        _remoteUnsupported ||
        _signedEventRelay == null) {
      return;
    }
    if (_isPublishing) return;

    final completer = Completer<void>();
    _publishCompleter = completer;
    _isPublishing = true;
    debugPrint('[ReadStateManager] publish starting slotId=$_slotId');
    try {
      await _fetchOwnBlobBeforePublish();

      final contexts = _currentContexts();
      if (_isIdenticalToLastPublished(contexts)) {
        return;
      }

      final blob = ReadStateBlob(clientId: _clientId, contexts: contexts);
      final ciphertext = _crypto.encrypt(jsonEncode(blob.toJson()));
      final createdAt = max(currentUnixSeconds(), _maxFetchedCreatedAt + 1);

      await _signedEventRelay.submit(
        kind: EventKind.readState,
        content: ciphertext,
        tags: [
          ['d', '$readStateDTagPrefix$_slotId'],
          ['t', 'read-state'],
        ],
        createdAt: createdAt,
      );
      debugPrint('[ReadStateManager] publish accepted createdAt=$createdAt');

      _lastPublishedContexts = contexts;
      _forcedContextIds.clear();
      for (final key in contexts.keys) {
        _contextSourceCreatedAt[key] = createdAt;
      }
      _maxFetchedCreatedAt = max(_maxFetchedCreatedAt, createdAt);
      _persistLocalState();
    } catch (error) {
      if (_isPermanentReadStateRemoteError(error)) {
        _remoteUnsupported = true;
        _debounceTimer?.cancel();
        _debounceTimer = null;
        debugPrint(
          '[ReadStateManager] remote read-state sync is unavailable; '
          'using local read state.',
        );
        return;
      }
      debugPrint('[ReadStateManager] publish failed: $error');
    } finally {
      _isPublishing = false;
      completer.complete();
      if (_publishCompleter == completer) {
        _publishCompleter = null;
      }
    }
  }

  Future<void> _fetchOwnBlobBeforePublish() async {
    if (_relaySession == null) return;

    try {
      final events = await _relaySession.fetchHistory(
        NostrFilter(
          kinds: const [EventKind.readState],
          authors: [pubkey],
          tags: {
            '#d': ['$readStateDTagPrefix$_slotId'],
          },
          limit: readStateFetchLimit,
        ),
      );
      _mergeEvents(events);
      _persistLocalState();
      if (!_disposed) {
        _onChanged();
      }
    } catch (e) {
      debugPrint('[ReadStateManager] fetchOwnBlobBeforePublish failed: $e');
    }
  }

  bool _isIdenticalToLastPublished(Map<String, int> contexts) {
    if (_lastPublishedContexts.length != contexts.length) {
      return false;
    }
    for (final entry in contexts.entries) {
      if (_lastPublishedContexts[entry.key] != entry.value) {
        return false;
      }
    }
    return true;
  }

  Set<String> drainSyncedRollbacks() {
    final drained = Set<String>.from(_pendingSyncedRollbacks);
    _pendingSyncedRollbacks.clear();
    return drained;
  }

  Map<String, int> _currentContexts() {
    final contexts = <String, int>{};
    for (final entry in _effectiveState.entries) {
      if (_publishableContextIds.contains(entry.key)) {
        contexts[entry.key] = entry.value;
      }
    }
    return contexts;
  }

  void _hydrateFromLocalStorage() {
    final stored = _storage.read(pubkey);
    _effectiveState
      ..clear()
      ..addAll(stored.contexts);
    _publishableContextIds
      ..clear()
      ..addAll(stored.publishableContextIds);
    _forcedContextIds.clear();
    _contextSourceCreatedAt
      ..clear()
      ..addAll(stored.sourceCreatedAt);
    _persistLocalState();
  }

  void _persistLocalState() {
    _storage.write(
      pubkey,
      _effectiveState,
      _publishableContextIds,
      _contextSourceCreatedAt,
    );
  }

  void _rotateSlotId() {
    _slotId = generateReadStateSlotId();
    _storage.writeSlotId(pubkey, _slotId);
  }

  bool _isPlausibleCreatedAt(int createdAt) =>
      createdAt <= currentUnixSeconds() + readStateMaxClockDriftSeconds;

  bool _isPermanentReadStateRemoteError(Object error) {
    // Relay rejections come back as `Exception("<message>")` from the
    // websocket OK handler. Pattern-match on the message text since we no
    // longer have HTTP status codes.
    final msg = error.toString().toLowerCase();
    return msg.contains('unknown event kind') ||
        msg.contains('missing users:write') ||
        msg.contains('insufficient scope') ||
        msg.contains('restricted: unknown');
  }
}
