import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

import '../crypto/nip44.dart';
import '../relay/relay.dart';
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

enum _ApplyRemoteContextResult { unchanged, advanced }

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
  // Override keys already in this device's own slot, carried unchanged.
  // See [isOverrideContext].
  final Map<String, int> _carriedOverrides = {};
  Map<String, int> _lastPublishedContexts = {};

  Timer? _debounceTimer;
  void Function()? _unsubscribeLive;
  bool _initialized = false;
  bool _disposed = false;
  bool _isPublishing = false;
  // Set when a publish is asked for while one is running. The running
  // publish took its snapshot first, so it publishes again when it ends.
  bool _publishAgain = false;
  Completer<void>? _publishCompleter;
  bool _remoteUnsupported = false;
  int _maxFetchedCreatedAt = 0;
  final Map<String, int> _contextSourceCreatedAt = {};
  final Set<String> _pendingSyncedAdvances = {};

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
    if (_disposed || isOverrideContext(contextId)) return;
    // Set first: retention keeps the most recently written marks, so the
    // save that follows must already see this read as the newest.
    _contextSourceCreatedAt[contextId] = max(
      currentUnixSeconds(),
      _maxFetchedCreatedAt + 1,
    );
    _advanceContext(contextId, unixTimestamp, publishable: true);
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
    if (_disposed || !_remoteEnabled || !_initialized) return;
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
        if (isOverrideContext(entry.key)) continue;
        final result = _applyRemoteContextTimestamp(
          contextId: entry.key,
          timestamp: entry.value,
          eventCreatedAt: event.createdAt,
        );
        if (result == _ApplyRemoteContextResult.advanced) {
          _pendingSyncedAdvances.add(entry.key);
          if (republishesMergedContext(entry.key)) {
            _publishableContextIds.add(entry.key);
          }
        }
      }

      if (decoded.blob.clientId == _clientId &&
          event.createdAt > ownBlobCreatedAt) {
        ownBlob = decoded.blob;
        ownBlobCreatedAt = event.createdAt;
      }
    }

    if (ownBlob != null) {
      _carryOwnOverrides(ownBlob.contexts);
      _lastPublishedContexts = Map<String, int>.from(ownBlob.contexts);
      _publishableContextIds.addAll(
        ownBlob.contexts.keys.where((key) => !isOverrideContext(key)),
      );
    }
  }

  /// Merges the complete override groups of this device's own slot into
  /// [_carriedOverrides] by `max()`, the NIP-RS merge rule. An incomplete
  /// group is rejected whole (see [completeOverrideGroups]).
  bool _carryOwnOverrides(Map<String, int> contexts) {
    var changed = false;
    for (final entry in completeOverrideGroups(contexts).entries) {
      if (entry.value > (_carriedOverrides[entry.key] ?? -1)) {
        _carriedOverrides[entry.key] = entry.value;
        changed = true;
      }
    }
    return changed;
  }

  Future<void> _startLiveSubscription() async {
    try {
      final unsub = await _relaySession!.subscribe(
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
      if (_disposed) {
        unsub.call();
        return;
      }
      _unsubscribeLive = unsub;
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

    var changed =
        decoded.blob.clientId == _clientId &&
        _carryOwnOverrides(decoded.blob.contexts);
    for (final entry in decoded.blob.contexts.entries) {
      if (isOverrideContext(entry.key)) continue;
      final result = _applyRemoteContextTimestamp(
        contextId: entry.key,
        timestamp: entry.value,
        eventCreatedAt: event.createdAt,
      );
      if (result == _ApplyRemoteContextResult.advanced) {
        _pendingSyncedAdvances.add(entry.key);
        changed = true;
      }
      if ((decoded.blob.clientId == _clientId ||
              republishesMergedContext(entry.key)) &&
          _publishableContextIds.add(entry.key)) {
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

  _ApplyRemoteContextResult _applyRemoteContextTimestamp({
    required String contextId,
    required int timestamp,
    required int eventCreatedAt,
  }) {
    final sourceCreatedAt = _contextSourceCreatedAt[contextId] ?? 0;
    final current = _effectiveState[contextId] ?? 0;
    final next = max(current, timestamp);
    final result = next == current
        ? _ApplyRemoteContextResult.unchanged
        : _ApplyRemoteContextResult.advanced;

    if (result == _ApplyRemoteContextResult.advanced) {
      _effectiveState[contextId] = next;
    }
    if (eventCreatedAt > sourceCreatedAt) {
      _contextSourceCreatedAt[contextId] = eventCreatedAt;
    }
    return result;
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
    if (_isPublishing) {
      // The running publish may have taken its snapshot before this change.
      _publishAgain = true;
      return _publishCompleter?.future;
    }

    final completer = Completer<void>();
    _publishCompleter = completer;
    _isPublishing = true;
    try {
      do {
        _publishAgain = false;
        await _publishOnce(_signedEventRelay);
      } while (_publishAgain &&
          (allowDisposed || !_disposed) &&
          !_remoteUnsupported);
    } finally {
      _isPublishing = false;
      _publishAgain = false;
      completer.complete();
      if (_publishCompleter == completer) {
        _publishCompleter = null;
      }
    }
  }

  Future<void> _publishOnce(SignedEventRelay signedEventRelay) async {
    debugPrint('[ReadStateManager] publish starting slotId=$_slotId');
    try {
      await _fetchOwnBlobBeforePublish();

      final contexts = _currentContexts();
      if (contexts == null) {
        debugPrint(
          '[ReadStateManager] publish skipped: carried override keys do not '
          'fit the slot, so it stays as it is.',
        );
        return;
      }
      if (_isIdenticalToLastPublished(contexts)) {
        return;
      }

      final blob = ReadStateBlob(clientId: _clientId, contexts: contexts);
      final ciphertext = _crypto.encrypt(jsonEncode(blob.toJson()));
      final createdAt = max(currentUnixSeconds(), _maxFetchedCreatedAt + 1);

      await signedEventRelay.submit(
        kind: EventKind.readState,
        content: ciphertext,
        tags: [
          ['d', '$readStateDTagPrefix$_slotId'],
          ['t', 'read-state'],
        ],
        createdAt: createdAt,
      );
      debugPrint('[ReadStateManager] publish accepted createdAt=$createdAt');

      for (final key in contexts.keys) {
        if (_lastPublishedContexts[key] != contexts[key]) {
          _contextSourceCreatedAt[key] = createdAt;
        }
      }
      _lastPublishedContexts = contexts;
      _maxFetchedCreatedAt = max(_maxFetchedCreatedAt, createdAt);
      _persistLocalState();
    } catch (error) {
      if (_isOversizedReadStateError(error)) {
        _remoteUnsupported = true;
        _debounceTimer?.cancel();
        _debounceTimer = null;
        debugPrint(
          '[ReadStateManager] remote read-state sync disabled because the '
          'local state exceeds the NIP-44 plaintext limit.',
        );
        return;
      }
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

  bool _isIdenticalToLastPublished(Map<String, int>? contexts) {
    // Null: the slot cannot be published, so there is nothing to send.
    if (contexts == null) return true;
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

  Set<String> drainSyncedAdvances() {
    final drained = Set<String>.from(_pendingSyncedAdvances);
    _pendingSyncedAdvances.clear();
    return drained;
  }

  /// The slot to publish, or null when the carried override keys alone do
  /// not fit, so the current slot must stay as it is.
  Map<String, int>? _currentContexts() {
    final contexts = <String, int>{};
    for (final entry in _effectiveState.entries) {
      if (_publishableContextIds.contains(entry.key)) {
        contexts[entry.key] = entry.value;
      }
    }
    // A carried group's frontier travels with it, even a merged `msg:` mark
    // that would not be republished on its own.
    for (final key in overrideGroupFrontierKeys(_carriedOverrides)) {
      if (_effectiveState[key] case final frontier?) contexts[key] = frontier;
    }
    return retainReadStateContexts(
      contexts,
      clientId: _clientId,
      recent: _contextSourceCreatedAt,
      carried: _carriedOverrides,
    );
  }

  void _hydrateFromLocalStorage() {
    final stored = _storage.read(pubkey);
    // Earlier versions merged the web app's override keys and published
    // them in this device's slot, so publishable ones are carried.
    _carriedOverrides
      ..clear()
      ..addAll(
        completeOverrideGroups({
          for (final entry in stored.contexts.entries)
            if (isOverrideContext(entry.key) &&
                stored.publishableContextIds.contains(entry.key))
              entry.key: entry.value,
        }),
      );
    _effectiveState
      ..clear()
      ..addEntries(
        stored.contexts.entries.where((entry) => !isOverrideContext(entry.key)),
      );
    _publishableContextIds
      ..clear()
      ..addAll(stored.publishableContextIds.where(_effectiveState.containsKey));
    _contextSourceCreatedAt
      ..clear()
      ..addAll(stored.sourceCreatedAt);
    _persistLocalState();
  }

  void _persistLocalState() {
    // Save a bounded copy. Memory keeps every mark for this session, so a
    // message read from old history stays read until the app restarts.
    final saved = pruneStaleContexts(
      _effectiveState,
      nowUnixSeconds: currentUnixSeconds(),
    )..addAll(_carriedOverrides);
    // A carried group's frontier is never pruned away from it.
    for (final key in overrideGroupFrontierKeys(_carriedOverrides)) {
      if (_effectiveState[key] case final frontier?) saved[key] = frontier;
    }
    _storage.write(
      pubkey,
      saved,
      {
        ..._publishableContextIds.where(saved.containsKey),
        ..._carriedOverrides.keys,
      },
      {
        for (final entry in _contextSourceCreatedAt.entries)
          if (saved.containsKey(entry.key)) entry.key: entry.value,
      },
    );
  }

  void _rotateSlotId() {
    _slotId = generateReadStateSlotId();
    _storage.writeSlotId(pubkey, _slotId);
  }

  bool _isPlausibleCreatedAt(int createdAt) =>
      createdAt <= currentUnixSeconds() + readStateMaxClockDriftSeconds;

  bool _isOversizedReadStateError(Object error) {
    final msg = error.toString().toLowerCase();
    return error is ArgumentError &&
        msg.contains('plaintext must be 1-65535 bytes');
  }

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
