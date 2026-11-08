import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

import '../crypto/nip44.dart';
import '../relay/relay.dart';
import '../relay/read_state_snapshot.dart';
import 'read_state_format.dart';
import 'read_state_storage.dart';
import 'read_state_packing.dart';
import 'read_state_time.dart';
import '../theme/community_theme_preference.dart';

class ReadStateCrypto {
  final Uint8List conversationKey;
  final String Function(Uint8List conversationKey, String ciphertext) _decrypt;
  final int _maxRetainedLength;

  /// Plaintext by ciphertext, least recently used first. NIP-44 payloads
  /// carry a random nonce and a MAC, so under this one conversation key the
  /// same ciphertext always decrypts to the same plaintext. Every publish
  /// re-reads up to [readStateFetchLimit] of our own blobs, every resume
  /// replays them on the live subscription, and the relay echoes each
  /// publish back, so most decrypts here repeat a ciphertext this instance
  /// already decrypted or produced. Decrypting is pure-Dart ChaCha20 and HMAC
  /// on the UI isolate, so repeats are served from this memo.
  final Map<String, String> _plaintextByCiphertext = {};
  int _retainedLength = 0;

  /// Upper bound on the code units the memo retains, counting each ciphertext
  /// key and its plaintext, so it cannot grow without limit however large the
  /// blobs get (about 8 MB of one-byte strings at the default). Each slot's
  /// current blob costs up to about 95,000 (a 40 KiB plaintext and its
  /// base64 ciphertext), so this holds about 80 slots. The replay walks every
  /// slot in turn, so a bound below the live slots' total evicts each entry
  /// before its next use and nothing hits; an account with 27 slots measured
  /// about 2.6M.
  static const defaultMaxRetainedLength = 8000000;

  ReadStateCrypto._(
    this.conversationKey, {
    String Function(Uint8List conversationKey, String ciphertext) decrypt =
        nip44Decrypt,
    int maxRetainedLength = defaultMaxRetainedLength,
  }) : _decrypt = decrypt,
       _maxRetainedLength = maxRetainedLength;

  /// Builds a crypto around a known key with a counting or bounded memo.
  @visibleForTesting
  ReadStateCrypto.forTesting(
    Uint8List conversationKey, {
    String Function(Uint8List conversationKey, String ciphertext) decrypt =
        nip44Decrypt,
    int maxRetainedLength = defaultMaxRetainedLength,
  }) : this._(
         conversationKey,
         decrypt: decrypt,
         maxRetainedLength: maxRetainedLength,
       );

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

  String encrypt(String plaintext) {
    final ciphertext = nip44Encrypt(conversationKey, plaintext);
    // The relay echoes this blob back, and the next publish fetches it.
    _remember(ciphertext, plaintext);
    return ciphertext;
  }

  /// Throws on a ciphertext that does not decrypt, every time: failures are
  /// never memoized.
  String decrypt(String ciphertext) {
    final known = _plaintextByCiphertext.remove(ciphertext);
    if (known != null) {
      // Most recently used goes last, so superseded blobs are evicted first.
      _plaintextByCiphertext[ciphertext] = known;
      return known;
    }
    final plaintext = _decrypt(conversationKey, ciphertext);
    _remember(ciphertext, plaintext);
    return plaintext;
  }

  void _remember(String ciphertext, String plaintext) {
    final cost = ciphertext.length + plaintext.length;
    if (cost > _maxRetainedLength) return;
    final replaced = _plaintextByCiphertext.remove(ciphertext);
    if (replaced != null) {
      _retainedLength -= ciphertext.length + replaced.length;
    }
    _plaintextByCiphertext[ciphertext] = plaintext;
    _retainedLength += cost;
    while (_retainedLength > _maxRetainedLength) {
      final oldest = _plaintextByCiphertext.keys.first;
      final dropped = _plaintextByCiphertext.remove(oldest)!;
      _retainedLength -= oldest.length + dropped.length;
    }
  }

  @visibleForTesting
  int get retainedLength => _retainedLength;
}

enum _ApplyRemoteContextResult { unchanged, advanced }

class ReadStateManager {
  final String pubkey;
  final String relayUrl;
  final String _journalKey;
  final ReadStateCrypto _crypto;
  final ReadStateStorage _storage;
  final RelaySessionNotifier? _relaySession;
  final SignedEventRelay? _signedEventRelay;
  final bool _remoteEnabled;
  final VoidCallback _onChanged;

  late final Object _journalOwner;
  bool get _ownsJournal => _storage.ownsJournal(_journalKey, _journalOwner);

  late String _clientId;
  late String _slotId;

  final Map<String, int> _effectiveState = {};
  final Map<String, int> _localContexts = {};
  final List<String> _slots = [];
  List<Map<String, int>> _placement = [];
  final Map<String, Map<String, int>> _published = {};
  final Map<String, NostrEvent> _pending = {};
  Future<void> _saved = Future.value();
  bool _saveScheduled = false;
  Object? syncError;
  // Whether the latest snapshot read succeeded. A publish that skips its own
  // snapshot clears [syncError] only on this evidence.
  bool _lastSnapshotSucceeded = false;
  final int Function() _now;
  // Override keys already in this device's own slot, carried unchanged.
  // See [isOverrideContext].
  final Map<String, int> _carriedOverrides = {};

  Timer? _debounceTimer;
  void Function()? _unsubscribeLive;
  bool _initialized = false;
  bool _initialSeedComplete = false;
  // Pending first fetch and subscribe. A reconnect while it runs waits for it
  // so two passes cannot each install a live subscription and leak one.
  Future<void>? _initializing;
  // The reconnect pass running now, and the one waiting to start after it.
  Future<void>? _runningResync;
  Future<void>? _queuedResync;
  // Whether the first fetch and subscribe both succeeded. A connect that
  // lands while they run (the cold-start connect) then has nothing to redo.
  bool _initialSyncSucceeded = false;

  bool _disposed = false;
  bool _isPublishing = false;
  // Set when a publish is asked for while one is running. The running
  // publish took its snapshot first, so it publishes again when it ends.
  bool _publishAgain = false;
  Completer<void>? _publishCompleter;
  bool _remoteUnsupported = false;
  int _maxFetchedCreatedAt = 0;
  final Set<String> _pendingSyncedAdvances = {};

  ReadStateManager({
    required this.pubkey,
    required SharedPreferences prefs,
    required ReadStateCrypto crypto,
    required RelaySessionNotifier? relaySession,
    required SignedEventRelay? signedEventRelay,
    required bool remoteEnabled,
    required VoidCallback onChanged,
    int Function()? now,
    required this.relayUrl,
  }) : _journalKey =
           '$pubkey:${Uri.encodeComponent(normalizeCommunityRelayUrl(relayUrl))}',
       _now = now ?? currentUnixSeconds,
       _crypto = crypto,
       _storage = ReadStateStorage(prefs),
       _relaySession = relaySession,
       _signedEventRelay = signedEventRelay,
       _remoteEnabled = remoteEnabled,
       _onChanged = onChanged {
    _journalOwner = _storage.claimJournal(_journalKey);
    _clientId = _storage.getOrCreateClientId(pubkey, relayUrl);
    _slotId = _storage.getOrCreateSlotId(pubkey, relayUrl);
    _slots.add(_slotId);
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

    final initializing = Completer<void>();
    _initializing = initializing.future;
    try {
      final subscribed = await _startLiveSubscription();
      final fetched = await _fetchAndMerge();
      _initialSyncSucceeded = fetched && subscribed;
    } finally {
      _initializing = null;
      initializing.complete();
    }
    if (_localContexts.isNotEmpty ||
        _pending.isNotEmpty ||
        _carriedOverrides.isNotEmpty) {
      _schedulePublish();
    }

    _onChanged();
    debugPrint(
      '[ReadStateManager] initialize complete maxFetchedCreatedAt=$_maxFetchedCreatedAt contexts=${_effectiveState.length}',
    );
  }

  void markContextRead(String contextId, int unixTimestamp) {
    if (_disposed || isOverrideContext(contextId) || unixTimestamp < 0) return;
    if (_advanceContext(
      contextId,
      unixTimestamp,
      publishable: !contextId.startsWith(msgContextPrefix),
    )) {
      _persistLocalState();
      _onChanged();
      if (!contextId.startsWith(msgContextPrefix)) _schedulePublish();
    }
  }

  void seedContextRead(String contextId, int unixTimestamp) {
    seedContextsRead({contextId: unixTimestamp});
  }

  /// Baseline the first channel-list landing once per identity/community.
  /// The journal persists even an empty scope: later channels remain unread
  /// after process death, rather than becoming a new "first" snapshot.
  void seedInitialContextsRead(Map<String, int> contexts) {
    if (_disposed || _initialSeedComplete) return;
    _initialSeedComplete = true;
    seedContextsRead(contexts);
    _persistLocalState();
  }

  /// Advance local-only seeds together, saving and notifying once per batch.
  void seedContextsRead(Map<String, int> contexts) {
    var changed = false;
    for (final entry in contexts.entries) {
      if (_advanceContext(entry.key, entry.value, publishable: false)) {
        changed = true;
      }
    }
    if (!changed) return;
    _persistLocalState();
    _onChanged();
  }

  Future<void> flush() async {
    _debounceTimer?.cancel();
    _debounceTimer = null;
    if (!_remoteEnabled || _remoteUnsupported || _disposed) return;
    await _publish();
  }

  /// Fetch and resubscribe after a reconnect. Passes run one at a time, so
  /// each replaces the previous live subscription instead of leaking one.
  /// Reconnects while a pass waits to start share that pass.
  Future<void> reinitializeRemote() {
    if (_disposed || !_remoteEnabled || !_initialized) return Future.value();
    final queued = _queuedResync;
    if (queued != null) return queued;
    final running = _runningResync;
    final initializing = _initializing;
    final pass = Completer<void>();
    _queuedResync = pass.future;
    unawaited(() async {
      try {
        if (running != null) {
          await running;
        } else if (initializing != null) {
          await initializing;
          // Connected while the first sync ran: it already used this
          // connection.
          if (_initialSyncSucceeded) return;
        }
        if (_disposed) return;
        if (identical(_queuedResync, pass.future)) _queuedResync = null;
        _runningResync = pass.future;
        await _resyncRemote();
      } catch (e) {
        debugPrint('[ReadStateManager] reinitializeRemote failed: $e');
      } finally {
        if (identical(_queuedResync, pass.future)) _queuedResync = null;
        if (identical(_runningResync, pass.future)) _runningResync = null;
        pass.complete();
      }
    }());
    return pass.future;
  }

  Future<void> _resyncRemote() async {
    debugPrint('[ReadStateManager] reinitializeRemote');
    if (_isPublishing) {
      await _publishCompleter?.future;
    }
    _unsubscribeLive?.call();
    _unsubscribeLive = null;
    await _startLiveSubscription();
    await _fetchAndMerge();
    _schedulePublish();
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

  bool _advanceContext(
    String contextId,
    int unixTimestamp, {
    required bool publishable,
  }) {
    if (_disposed || unixTimestamp < 0) return false;

    final current = _effectiveState[contextId] ?? 0;
    final local = _localContexts[contextId] ?? 0;
    final localAdvanced = publishable && unixTimestamp > local;
    if (unixTimestamp <= current && !localAdvanced) return false;
    _effectiveState[contextId] = max(current, unixTimestamp);
    if (localAdvanced) _localContexts[contextId] = unixTimestamp;
    return true;
  }

  Future<bool> _fetchAndMerge() async {
    try {
      // Snapshot transport validates completeness and fences retired sessions.
      final events = await _relaySession!.fetchReadStateSnapshot();
      if (_disposed || !_servesThisCommunity) return false;
      _lastSnapshotSucceeded = true;
      _mergeEvents(events);
      await _persistLocalState();
      _onChanged();
      return true;
    } catch (error) {
      _lastSnapshotSucceeded = false;
      syncError = error;
      debugPrint('[ReadStateManager] snapshot failed: $error');
      return false;
    }
  }

  void _mergeEvents(List<NostrEvent> events) {
    if (!_servesThisCommunity) return;
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
      final slot = decoded.dTag.substring(readStateDTagPrefix.length);
      final index = _slots.indexOf(slot);
      if (index >= 0 && decoded.blob.clientId != _clientId) {
        // Keep the placement, but never replace a foreign coordinate.
        _slots[index] = generateReadStateSlotId();
        _pending.remove(slot);
        _published.remove(slot);
        _slotId = _slots.first;
      }
      final own = index >= 0 && decoded.blob.clientId == _clientId;
      if (own) {
        if (index == 0) _carryOwnOverrides(decoded.blob.contexts);
        _published[slot] = Map.of(decoded.blob.contexts);
        while (_placement.length <= index) {
          _placement.add({});
        }
      }
      for (final entry in decoded.blob.contexts.entries) {
        if (isOverrideContext(entry.key)) continue;
        if (_applyRemoteContextTimestamp(
              contextId: entry.key,
              timestamp: entry.value,
            ) ==
            _ApplyRemoteContextResult.advanced) {
          _pendingSyncedAdvances.add(entry.key);
        }
        // Historical own-slot values are recoverable local writes. Peer
        // maxima never change the locally owned values or placement.
        if (own && !entry.key.startsWith(msgContextPrefix)) {
          _localContexts[entry.key] = max(
            _localContexts[entry.key] ?? 0,
            entry.value,
          );
          if (!_placement.any((blob) => blob.containsKey(entry.key))) {
            _placement[index][entry.key] = entry.value;
          }
        }
      }
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

  Future<bool> _startLiveSubscription() async {
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
        return false;
      }
      _unsubscribeLive = unsub;
      debugPrint('[ReadStateManager] live subscription established');
      return true;
    } catch (e) {
      debugPrint('[ReadStateManager] live subscription FAILED: $e');
      return false;
    }
  }

  void _handleIncomingEvent(NostrEvent event) {
    if (_disposed || !_servesThisCommunity) return;
    _mergeEvents([event]);
    _persistLocalState();
    _onChanged();
  }

  _ApplyRemoteContextResult _applyRemoteContextTimestamp({
    required String contextId,
    required int timestamp,
  }) {
    final current = _effectiveState[contextId] ?? 0;
    final next = max(current, timestamp);
    final result = next == current
        ? _ApplyRemoteContextResult.unchanged
        : _ApplyRemoteContextResult.advanced;

    if (result == _ApplyRemoteContextResult.advanced) {
      _effectiveState[contextId] = next;
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
        await _publishOnce(_signedEventRelay, allowDisposed: allowDisposed);
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

  /// Whether a publish would send anything: a pending event to retry, or a
  /// slot whose packed contexts differ from what was last published. Mirrors
  /// the packing in [_publishOnce] without changing any state.
  bool _hasUnpublishedState() {
    if (_pending.isNotEmpty) return true;
    final primary = <String, int>{..._carriedOverrides};
    for (final key in overrideGroupFrontierKeys(_carriedOverrides)) {
      if (_effectiveState[key] case final value?) primary[key] = value;
    }
    final horizon = _now() - readStateHorizonSeconds;
    final List<Map<String, int>> placement;
    try {
      placement = packReadStateContexts(
        {
          for (final entry in _localContexts.entries)
            if (entry.value >= horizon) entry.key: entry.value,
        },
        clientId: _clientId,
        previous: _placement,
        primary: primary,
      );
    } on StateError {
      // Let the real publish surface the capacity error.
      return true;
    }
    if (placement.length > _slots.length) return true;
    for (var index = 0; index < placement.length; index++) {
      final published = _published[_slots[index]] ?? const <String, int>{};
      if (!mapEquals(published, placement[index])) return true;
    }
    return false;
  }

  Future<void> _publishOnce(
    SignedEventRelay signedEventRelay, {
    required bool allowDisposed,
  }) async {
    try {
      if (!_servesThisCommunity || !_ownsJournal) return;
      await _persistLocalState();
      // The snapshot below is the whole read state (megabytes on a busy
      // account), so skip it when there is nothing to send. Each resume
      // schedules a publish after its re-sync whether or not anything changed.
      if (!_hasUnpublishedState()) {
        // Nothing pending and every slot published, so after a good snapshot
        // this device is in sync, as a full publish would have concluded.
        if (_lastSnapshotSucceeded) syncError = null;
        return;
      }
      final checkConnection = _relaySession?.captureConnectionFence();
      var snapshot = <NostrEvent>[];
      if (_relaySession != null) {
        final events = await _relaySession.fetchReadStateSnapshot();
        _lastSnapshotSucceeded = true;
        if (!_servesThisCommunity ||
            !_ownsJournal ||
            (_disposed && !allowDisposed)) {
          return;
        }
        checkConnection?.call();
        snapshot = events;
        _mergeEvents(events);
        if (events.any(
          (event) =>
              hasValidReadStateTags(event) &&
              event.tags.any(
                (tag) =>
                    tag.isNotEmpty &&
                    tag.first == 'd' &&
                    _slots.contains(
                      tag[1].substring(readStateDTagPrefix.length),
                    ),
              ) &&
              decodeReadStateEvent(
                    event,
                    pubkey: pubkey,
                    decrypt: _crypto.decrypt,
                  ) ==
                  null,
        )) {
          throw StateError(
            'Unreadable read-state coordinate blocks publication',
          );
        }
      }
      if (_disposed && !allowDisposed) return;
      final primary = <String, int>{..._carriedOverrides};
      for (final key in overrideGroupFrontierKeys(_carriedOverrides)) {
        if (_effectiveState[key] case final value?) primary[key] = value;
      }
      _localContexts.removeWhere(
        (key, value) => value < _now() - readStateHorizonSeconds,
      );
      _placement = packReadStateContexts(
        _localContexts,
        clientId: _clientId,
        previous: _placement,
        primary: primary,
      );
      while (_slots.length < _placement.length) {
        _slots.add(generateReadStateSlotId());
      }
      await _persistLocalState();
      final planned = [
        for (final blob in _placement) Map<String, int>.of(blob),
      ];
      final prepared = <String, NostrEvent>{};
      var createdAt = max(_now(), _maxFetchedCreatedAt + 1);
      for (final pending in _pending.values) {
        createdAt = max(createdAt, pending.createdAt + 1);
      }
      for (var index = 0; index < planned.length; index++) {
        final slot = _slots[index];
        final contexts = planned[index];
        if (mapEquals(_published[slot] ?? const <String, int>{}, contexts) &&
            !_pending.containsKey(slot)) {
          continue;
        }
        // Fixed-width Nostr IDs/signatures have exactly these JSON byte sizes.
        // Use the actual encrypted content and every other signed field.
        prepared[slot] = NostrEvent(
          id: '0' * 64,
          pubkey: pubkey,
          sig: '0' * 128,
          kind: EventKind.readState,
          createdAt: createdAt++,
          tags: [
            ['d', '$readStateDTagPrefix$slot'],
            ['t', 'read-state'],
          ],
          content: _crypto.encrypt(
            jsonEncode(
              ReadStateBlob(clientId: _clientId, contexts: contexts).toJson(),
            ),
          ),
        );
      }
      // Check replay and successor prefixes before sending. Not a reservation:
      // another device can still grow the remote snapshot concurrently.
      _checkSnapshotCapacity(snapshot, _pending.values);
      _checkSnapshotCapacity(snapshot, [
        ..._pending.values,
        ...prepared.values,
      ]);
      for (final entry in _pending.entries.toList()) {
        if (_relaySession == null) {
          throw StateError('No relay for pending retry');
        }
        if (!_servesThisCommunity ||
            !_ownsJournal ||
            (_disposed && !allowDisposed)) {
          return;
        }
        checkConnection?.call();
        await _relaySession.publish(entry.value);
        if (!_servesThisCommunity ||
            !_ownsJournal ||
            (_disposed && !allowDisposed)) {
          return;
        }
        _acknowledge(entry.key, entry.value);
        await _persistLocalState();
      }
      for (var index = 0; index < planned.length; index++) {
        final slot = _slots[index];
        final contexts = planned[index];
        final event = prepared[slot];
        if (event == null ||
            mapEquals(_published[slot] ?? const <String, int>{}, contexts)) {
          continue;
        }
        if (!_servesThisCommunity ||
            !_ownsJournal ||
            (_disposed && !allowDisposed)) {
          return;
        }
        checkConnection?.call();
        await signedEventRelay.submit(
          kind: event.kind,
          content: event.content,
          tags: event.tags,
          createdAt: event.createdAt,
          onSigned: (event) async {
            _pending[slot] = event;
            await _persistLocalState();
            checkConnection?.call();
            if (!_servesThisCommunity ||
                !_ownsJournal ||
                (_disposed && !allowDisposed) ||
                _slots[index] != slot) {
              throw StateError(
                'Read-state owner or coordinate retired before publication',
              );
            }
          },
        );
        if (!_servesThisCommunity ||
            !_ownsJournal ||
            (_disposed && !allowDisposed)) {
          return;
        }
        _published[slot] = contexts;
        _pending.remove(slot);
        _maxFetchedCreatedAt = event.createdAt;
        await _persistLocalState();
      }
      syncError = null;
    } catch (error) {
      // The journal retains local advances, placements and exact pending
      // events. Reconnect/flush retries; never acknowledge the whole batch.
      syncError = error;
      debugPrint(
        '[ReadStateManager] publish failed (journal retained): $error',
      );
      if (_isPermanentReadStateRemoteError(error)) _remoteUnsupported = true;
    }
  }

  void _checkSnapshotCapacity(
    List<NostrEvent> snapshot,
    Iterable<NostrEvent> writes,
  ) {
    String coordinate(NostrEvent event) =>
        '${event.kind}:${event.pubkey}:${event.tags.where((tag) => tag.length >= 2 && tag.first == 'd').map((tag) => tag[1]).firstOrNull ?? ''}';
    final projected = {for (final event in snapshot) coordinate(event): event};
    for (final event in writes) {
      projected[coordinate(event)] = event;
      if (projected.length > readStateSnapshotMaxEvents ||
          utf8
                  .encode(
                    jsonEncode(
                      projected.values.map((event) => event.toJson()).toList(),
                    ),
                  )
                  .length >
              readStateSnapshotMaxBytes) {
        throw StateError(
          'Read-state publication exceeds full snapshot capacity',
        );
      }
    }
  }

  void _acknowledge(String slot, NostrEvent event) {
    final blob = decodeReadStateBlob(_crypto.decrypt(event.content));
    if (blob == null) throw StateError('Invalid pending read-state event');
    _published[slot] = Map.of(blob.contexts);
    _pending.remove(slot);
    _maxFetchedCreatedAt = max(_maxFetchedCreatedAt, event.createdAt);
  }

  Set<String> drainSyncedAdvances() {
    final drained = Set<String>.from(_pendingSyncedAdvances);
    _pendingSyncedAdvances.clear();
    return drained;
  }

  void _hydrateFromLocalStorage() {
    final journal = _storage.readJournal(_journalKey);
    Map<String, int> contexts(Object? value) => (value as Map).map(
      (key, value) => MapEntry(key as String, value as int),
    );
    if (journal != null) {
      // An existing journal, even empty, means this is not a first landing.
      _initialSeedComplete = true;
      _clientId = journal['clientId'] as String;
      _effectiveState.addAll(contexts(journal['observed']));
      _localContexts.addAll(contexts(journal['local']));
      _carriedOverrides.addAll(contexts(journal['overrides']));
      _slots
        ..clear()
        ..addAll((journal['slots'] as List).cast<String>());
      if (_slots.isEmpty) {
        throw const FormatException('Empty read-state slot list');
      }
      _slotId = _slots.first;
      _placement = (journal['placement'] as List).map(contexts).toList();
      for (final entry in (journal['published'] as Map).entries) {
        _published[entry.key as String] = contexts(entry.value);
      }
      for (final entry in (journal['pending'] as Map).entries) {
        _pending[entry.key as String] = NostrEvent.fromJson(
          Map<String, dynamic>.from(entry.value as Map),
        );
      }
      _maxFetchedCreatedAt = journal['createdAt'] as int;
    } else {
      final stored = _storage.read(pubkey, relayUrl);
      _effectiveState.addEntries(
        stored.contexts.entries.where((entry) => !isOverrideContext(entry.key)),
      );
      // Legacy storage did not preserve provenance. Carry its existing
      // publishable values once; subsequent peer merges never enter this map.
      _localContexts.addEntries(
        stored.contexts.entries.where(
          (entry) =>
              stored.publishableContextIds.contains(entry.key) &&
              !isOverrideContext(entry.key) &&
              !entry.key.startsWith(msgContextPrefix),
        ),
      );
      _carriedOverrides.addAll(
        completeOverrideGroups({
          for (final entry in stored.contexts.entries)
            if (isOverrideContext(entry.key) &&
                stored.publishableContextIds.contains(entry.key))
              entry.key: entry.value,
        }, frontiers: stored.contexts),
      );
    }
    _persistLocalState();
  }

  Future<void> _persistLocalState() {
    if (_saveScheduled) return _saved;
    _saveScheduled = true;
    Future<void> save() async {
      _saveScheduled = false;
      if (!_ownsJournal) {
        throw StateError('Read-state journal writer retired');
      }
      final observed = pruneStaleContexts(
        _effectiveState,
        nowUnixSeconds: _now(),
      );
      for (final key in overrideGroupFrontierKeys(_carriedOverrides)) {
        if (_effectiveState[key] case final value?) observed[key] = value;
      }
      final encoded = jsonEncode({
        'v': 1,
        'clientId': _clientId,
        'observed': observed,
        'local': _localContexts,
        'overrides': _carriedOverrides,
        'slots': _slots,
        'placement': _placement,
        'published': _published,
        'pending': {
          for (final entry in _pending.entries) entry.key: entry.value.toJson(),
        },
        'createdAt': _maxFetchedCreatedAt,
      });
      await _storage.writeJournal(_journalKey, encoded, owner: _journalOwner);
    }

    // Coalesce synchronous batches without allowing a later write to race an
    // earlier write. A sender awaits this entire chain before network I/O.
    _saved = _saved.then(
      (_) => save(),
      onError: (Object _, StackTrace _) => save(),
    );
    unawaited(
      _saved.catchError((Object error) {
        syncError = error;
        debugPrint('[ReadStateManager] journal save failed: $error');
      }),
    );
    return _saved;
  }

  /// Switching community rebuilds the session in place before this manager
  /// is retired, so in between it must neither take in nor publish data.
  bool get _servesThisCommunity {
    final served = _relaySession?.relayUrl;
    return served == null || served == relayUrl;
  }

  bool _isPlausibleCreatedAt(int createdAt) =>
      createdAt <= _now() + readStateMaxClockDriftSeconds;

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
