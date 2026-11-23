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
import 'thread_follows_format.dart';
import '../theme/community_theme_preference.dart';

part 'read_state_follows.dart';
part 'read_state_journal.dart';
part 'read_state_capacity.dart';
part 'read_state_overrides.dart';
part 'read_state_retry.dart';

class ReadStateCrypto {
  final Uint8List conversationKey;
  final String Function(Uint8List conversationKey, String ciphertext) _decrypt;
  final int _maxRetainedLength;

  /// Plaintext by ciphertext, least recently used first. Under one NIP-44
  /// key a ciphertext always decrypts the same, and each publish and resume
  /// re-reads up to [readStateFetchLimit] of our own blobs, so repeats of the
  /// pure-Dart, UI-isolate decrypt are served from this memo.
  final Map<String, String> _plaintextByCiphertext = {};
  int _retainedLength = 0;

  /// Upper bound on the code units the memo retains (keys and plaintexts).
  /// A slot's blob costs up to about 95,000, so this holds about 80 slots;
  /// a bound below the live slots' total would evict each entry before its
  /// next use (27 slots measured about 2.6M).
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
  // Whether the relay session is connected. A reconnect re-syncs on its own,
  // so retries wait for one while disconnected.
  final bool Function() _isConnected;

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
  // The follow keys the latest scheduled journal write covers, each with the
  // pending edit generation it wrote (0: no pending edit, already synced).
  Future<Map<String, int>> _savedFollows = Future.value(const {});
  bool _saveScheduled = false;
  Object? syncError;
  // Channels whose synced clear failed. Their error outlives unrelated
  // successful syncs until the clear lands or the override ends.
  final Set<String> _failedClears = {};
  // Refused reading clears and the register each reader saw: retried once
  // after the next usable snapshot, never journaled (a restart keeps it live).
  final Map<String, OverrideRegister> _refusedReadingClears = {};

  Object? get _standingError {
    _failedClears.retainAll(activeChannelOverrides);
    return _failedClears.isEmpty
        ? null
        : StateError('Channel mark read cannot be synchronized');
  }

  // Whether the newest snapshot read succeeded. A publish that skips its own
  // snapshot clears [syncError] only on this evidence.
  bool _lastSnapshotSucceeded = false;
  // The error a failed snapshot read set: a later successful read clears
  // only this one, not a publish or capacity error.
  Object? _snapshotError;
  // Snapshot reads numbered in start order. Only a read newer than every
  // settled one decides [_lastSnapshotSucceeded], so an older read finishing
  // late cannot clear the retry a newer failure needs.
  int _snapshotsStarted = 0;
  int _newestSettledSnapshot = 0;
  int _staleThrough = 0; // Reads up to this predate the live stream's ready.
  final int Function() _now;
  // Merged whole-channel override registers, from this installation and its
  // peers. The primary carries all of them (NIP-RS, Buzz profile).
  final Map<String, OverrideRegister> _overrides = {};
  // Received groups stamped too far ahead, held until the clock reaches them.
  /// Groups stamped too far ahead, each contribution kept apart with where
  /// it came from, so each meets its own admission rule once due.
  final List<({String channelId, OverrideRegister register, bool local})>
  _deferredOverrides = [];
  // The unread record's coordinate slot and its last published contexts.
  String _unreadSlot = generateReadStateSlotId();
  Map<String, int> _unreadPublished = const {};
  // The verified head of this installation's own unread coordinate.
  NostrEvent? _unreadHead;
  // The durable, never-decreasing expiry clock; see [_expiryNow].
  int _expiryWatermark = 0;

  /// The watermark the journal last saved.
  int _savedExpiryWatermark = 0;

  // Synced thread follow choices. [_ownFollows] is this installation's own
  // entries (raw, so unknown fields are carried), placed into
  // [_followSlots] by [_followPlacement]. [_remoteFollows] is every follow
  // coordinate observed, by slot.
  final List<String> _followSlots = [];
  List<Set<String>> _followPlacement = [];
  final Map<String, Map<String, Object?>> _ownFollows = {};
  // Each own slot's unknown top-level fields, carried with that slot.
  final Map<String, Map<String, Object?>> _followExtra = {};
  // The newest event seen per follow coordinate.
  final Map<String, ({int createdAt, String id})> _followHeads = {};
  final Map<String, String> _followPublished = {};
  final Map<String, Map<String, ThreadFollowChoice>> _remoteFollows = {};
  final Set<String> _unreadableFollowSlots = {};
  bool _followsLoaded = false;
  // Whether this writer durably saved a complete recovery; never reset.
  bool _followsRecovered = false;
  // Pending own follow edits not yet covered by an accepted own head, by key:
  // the reservation (generation) of the edit. Their entries are in
  // [_ownFollows], which is the accepted heads plus these edits.
  final Map<String, int> _followEdits = {};
  // The edit generations each retained follow packet in [_pending] captured.
  final Map<String, Map<String, int>> _followCaptured = {};
  // The highest timestamp reserved here; every edit and packet takes a
  // strictly later one, so a wall-clock rollback cannot reuse a version.
  int _lastReserved = 0;
  // The newest full snapshot a publish read: the shared size basis that
  // follow acceptance checks against.
  List<NostrEvent> _capacitySnapshot = const [];

  Timer? _debounceTimer;
  Timer? _snapshotRetryTimer;
  int _snapshotRetryAttempt = 0;
  void Function()? _unsubscribeLive;
  // The session build that holds [_unsubscribeLive]'s subscription. A
  // rebuild drops it without a CLOSED.
  Object? _liveOwner;
  bool _initialized = false;
  bool _initialSeedComplete = false;
  // The one sync loop, while it runs. Each pass opens the live subscription
  // if it is missing, then reads the snapshot, so two passes never each
  // install a subscription and leak one. A request before a pass starts its
  // read joins that read; a request after runs one more pass.
  Future<void>? _syncing;
  bool _snapshotWanted = false;

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
    bool Function()? isConnected,
    required this.relayUrl,
  }) : _journalKey =
           '$pubkey:${Uri.encodeComponent(normalizeCommunityRelayUrl(relayUrl))}',
       _now = now ?? currentUnixSeconds,
       _crypto = crypto,
       _storage = ReadStateStorage(prefs),
       _relaySession = relaySession,
       _signedEventRelay = signedEventRelay,
       _remoteEnabled = remoteEnabled,
       _onChanged = onChanged,
       _isConnected = isConnected ?? (() => true) {
    _journalOwner = _storage.claimJournal(_journalKey);
    _clientId = _storage.getOrCreateClientId(pubkey, relayUrl);
    _slotId = _storage.getOrCreateSlotId(pubkey, relayUrl);
    _slots.add(_slotId);
    _hydrateFromLocalStorage();
  }

  /// Whether the relay refused read state for this session, so publishing
  /// stays off until the next manager (restart, sign-in, community switch).
  bool get remoteUnsupported => _remoteUnsupported;

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

    await _requestSync();
    if (_localContexts.isNotEmpty ||
        _pending.isNotEmpty ||
        _overrides.isNotEmpty ||
        _ownFollows.isNotEmpty) {
      _schedulePublish();
    }

    _onChanged();
    debugPrint(
      '[ReadStateManager] initialize complete maxFetchedCreatedAt=$_maxFetchedCreatedAt contexts=${_effectiveState.length}',
    );
  }

  /// Advance a context's frontier. A `msg:` frontier is published only for
  /// an [explicit] mark read; passive reading keeps it on this device. A
  /// channel read also clears that channel's live override group.
  void markContextRead(
    String contextId,
    int unixTimestamp, {
    bool explicit = false,
  }) {
    if (_disposed || isOverrideContext(contextId) || unixTimestamp < 0) return;
    final publishable = explicit || !contextId.startsWith(msgContextPrefix);
    // Decide the clear before the read passes the baseline; save both.
    final cleared = _clearChannelOverride(contextId) == true;
    final advanced = _advanceContext(
      contextId,
      unixTimestamp,
      publishable: publishable,
    );
    if (advanced || cleared) {
      _persistLocalState();
      _onChanged();
      if (publishable || cleared) _schedulePublish();
    }
  }

  /// Whether the last full-state load succeeded, as override actions need.
  /// Deferred future groups leave it incomplete until they resolve.
  bool get _hasFullState =>
      _lastSnapshotSucceeded &&
      _newestSettledSnapshot > _staleThrough &&
      _resolveDeferred();

  /// The live stream dropped: no read before its next ready is current.
  void liveInterrupted() => _staleThrough = 1 << 62;

  /// The merged stored frontier synced overrides use, 0 if absent: the
  /// retention floor decides message reads only, never override liveness.
  int _frontierOf(String contextId) => _effectiveState[contextId] ?? 0;

  Map<String, int> _publishableLocalContexts() {
    final horizon = _now() - readStateHorizonSeconds;
    return {
      for (final entry in _localContexts.entries)
        if (entry.value >= horizon) entry.key: entry.value,
    };
  }

  /// The read-position slot plan. Override groups travel in the unread
  /// record instead ([_planUnread]).
  List<Map<String, int>> _planSlots() => packReadStateContexts(
    _publishableLocalContexts(),
    clientId: _clientId,
    previous: _placement,
  );

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

  /// Catch up after a reconnect. The session replays the live subscription
  /// on the new socket, and its ready reads the snapshot, so this reads only
  /// when it has to open a subscription that was lost (a terminal CLOSED or a
  /// failed subscribe).
  Future<void> reinitializeRemote() async {
    if (_disposed || !_remoteEnabled || !_initialized) return;
    debugPrint('[ReadStateManager] reinitializeRemote');
    if (!_liveOpen) await _requestSync();
    _schedulePublish();
    _onChanged();
  }

  /// Asks the sync loop for a snapshot read. Completes when the loop has
  /// nothing left to do.
  Future<void> _requestSync() {
    _snapshotWanted = true;
    final syncing = _syncing;
    if (syncing != null) return syncing;
    // Set before the loop starts: a subscription can report ready from
    // inside subscribe, and that request must join this loop.
    final done = Completer<void>();
    _syncing = done.future;
    unawaited(_runSync(done));
    return done.future;
  }

  Future<void> _runSync(Completer<void> done) async {
    try {
      while (_snapshotWanted && !_disposed) {
        if (!_liveOpen) {
          await _startLiveSubscription();
          if (_disposed) return;
        }
        _snapshotWanted = false;
        // A failed read is retried by its backoff timer alone, or by a
        // later ready, resume, reconnect or Retry.
        await _fetchAndMerge();
      }
    } catch (e) {
      debugPrint('[ReadStateManager] sync failed: $e');
    } finally {
      _syncing = null;
      done.complete();
    }
  }

  void dispose({bool flushPending = true}) {
    if (_disposed) return;
    _disposed = true;

    final hadPendingPublish = _debounceTimer != null;
    _debounceTimer?.cancel();
    _debounceTimer = null;
    _snapshotRetryTimer?.cancel();
    _snapshotRetryTimer = null;

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
    final read = ++_snapshotsStarted;
    final heads = Map.of(_followHeads);
    try {
      // Snapshot transport validates completeness and fences retired sessions.
      final events = await _relaySession!.fetchReadStateSnapshot();
      if (_disposed || !_servesThisCommunity) return false;
      final newest = _settleSnapshot(read, succeeded: true);
      _mergeEvents(events);
      if (newest) _dropAbsentPeerFollows(heads, _capacitySnapshot = events);
      _followsLoaded = true;
      _retryRefusedReadingClears();
      await _persistLocalState();
      _followsRecovered = true;
      // Recheck after the save: a newer read may have failed meanwhile, and
      // its retry and error are its own.
      if (newest &&
          !_disposed &&
          _newestSettledSnapshot == read &&
          _lastSnapshotSucceeded) {
        // Hand outstanding marks to publication before the retry that would
        // have sent them is cancelled, and retire the read error this
        // success supersedes.
        if (_hasUnpublishedState()) _schedulePublish();
        _snapshotRetryAttempt = 0;
        _snapshotRetryTimer?.cancel();
        _snapshotRetryTimer = null;
        if (identical(syncError, _snapshotError)) syncError = null;
        _snapshotError = null;
      }
      _onChanged();
      return true;
    } catch (error) {
      if (_settleSnapshot(read, succeeded: false)) {
        syncError = error;
        _snapshotError = error;
      }
      debugPrint('[ReadStateManager] snapshot failed: $error');
      _scheduleSnapshotRetry();
      _onChanged();
      return false;
    }
  }

  /// Records snapshot read [read]'s outcome unless a newer read already
  /// settled. Returns whether it was recorded.
  bool _settleSnapshot(int read, {required bool succeeded}) {
    if (read < _newestSettledSnapshot) return false;
    _newestSettledSnapshot = read;
    _lastSnapshotSucceeded = succeeded;
    return true;
  }

  void _mergeEvents(List<NostrEvent> events) {
    if (!_servesThisCommunity) return;
    for (final event in events) {
      if (event.pubkey.toLowerCase() == pubkey.toLowerCase()) {
        if (threadFollowsSlotOf(event) case final slot?) {
          _mergeFollowEvent(event, slot);
          continue;
        }
      }
      final decoded = decodeReadStateEvent(
        event,
        pubkey: pubkey,
        decrypt: _crypto.decrypt,
      );
      if (decoded == null) continue;
      if (_isPlausibleCreatedAt(event.createdAt)) {
        _maxFetchedCreatedAt = max(_maxFetchedCreatedAt, event.createdAt);
      }
      if (decoded.blob.version == 2) {
        _mergeUnreadRecord(
          decoded.dTag.substring(readStateUnreadDTagPrefix.length),
          decoded.blob,
          event,
        );
        continue;
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
        if (own) {
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

  /// Whether the live subscription is open on the session's current build.
  bool get _liveOpen =>
      _unsubscribeLive != null &&
      _liveOwner == _relaySession!.liveSubscriptionOwner;

  Future<bool> _startLiveSubscription() async {
    _unsubscribeLive = null;
    try {
      // A rebuild while the subscribe waits for ready drops it, so open
      // again on the new build. Each pass needs another rebuild.
      void Function() unsub;
      Object owner;
      do {
        owner = _relaySession!.liveSubscriptionOwner;
        unsub = await _subscribeLive();
        if (_disposed) {
          unsub.call();
          return false;
        }
      } while (owner != _relaySession.liveSubscriptionOwner);
      _unsubscribeLive = unsub;
      _liveOwner = owner;
      debugPrint('[ReadStateManager] live subscription established');
      return true;
    } catch (e) {
      debugPrint('[ReadStateManager] live subscription FAILED: $e');
      return false;
    }
  }

  Future<void Function()> _subscribeLive() =>
      _relaySession!.subscribeWithStatus(
        NostrFilter(
          kinds: const [EventKind.readState],
          authors: [pubkey],
          tags: const {
            '#t': ['read-state', readStateUnreadTag, threadFollowsTag],
          },
          // Change notifications only: the snapshot that follows reads the
          // stored blobs, so a stored replay here would download them twice.
          limit: 0,
        ),
        _handleIncomingEvent,
        // With limit 0 nothing replays marks published before the REQ went
        // live, so read the snapshot every time it goes live: its EOSE, which
        // can come after the 500 ms fallback on a slow AUTH, on a reconnect's
        // replay, or after a CLOSED's retry. Only reads started after it count.
        onStatusChanged: (status) {
          if (_disposed) return;
          if (status != RelaySubscriptionStatus.ready) return liveInterrupted();
          _staleThrough = _snapshotsStarted;
          unawaited(_requestSync());
        },
        // The session dropped it, so the next reconnect opens a new one.
        onClosed: (_) => _unsubscribeLive = null,
        // A peer's mark can carry an older created_at (its clock, or a
        // publish that waited), and a since bound would drop it live.
        resumeSince: false,
      );

  void _handleIncomingEvent(NostrEvent event) {
    if (_disposed || !_servesThisCommunity) return;
    if (!isAuthenticReadStateEvent(event, viewer: pubkey)) return;
    _mergeEvents([event]);
    _persistLocalState();
    _onChanged();
    // A merged peer group can change this device's canonical primary.
    if (_hasFullState && _hasUnpublishedState()) _schedulePublish();
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
      unawaited(_publish(automatic: true));
    });
  }

  Future<void> _publish({
    bool allowDisposed = false,
    bool automatic = false,
  }) async {
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
    final errorBefore = syncError;
    try {
      do {
        _publishAgain = false;
        await _publishOnce(
          _signedEventRelay,
          allowDisposed: allowDisposed,
          automatic: automatic,
        );
      } while (_publishAgain &&
          (allowDisposed || !_disposed) &&
          !_remoteUnsupported);
    } finally {
      _isPublishing = false;
      _publishAgain = false;
      if (!_disposed) _onChanged(); // Surfaces syncError.
      completer.complete();
      if (_publishCompleter == completer) {
        _publishCompleter = null;
      }
      if (!_disposed && !identical(errorBefore, syncError)) _onChanged();
    }
  }

  Future<void> _publishOnce(
    SignedEventRelay signedEventRelay, {
    required bool allowDisposed,
    bool automatic = false,
  }) async {
    try {
      if (!_servesThisCommunity || !_ownsJournal) return;
      await _persistLocalState();
      // The snapshot below is the whole read state (megabytes on a busy
      // account), so skip it when there is nothing to send. Each resume
      // schedules a publish after its re-sync whether or not anything changed.
      // After a failed snapshot, an explicit flush runs the full publish at
      // once: its own snapshot is a retry. Automatic failed-read recovery has
      // one owner: the publish delay joins a read in flight, and leaves a
      // failed one to its backoff timer, which hands the marks back here.
      if (automatic) {
        if (_syncing case final syncing?) await syncing;
        if (!_lastSnapshotSucceeded && _snapshotRetryTimer != null) return;
      }
      if (_lastSnapshotSucceeded && !_hasUnpublishedState()) {
        // Nothing pending and every slot published, so this device is in
        // sync, as a full publish would have concluded.
        syncError = _standingError;
        return;
      }
      final checkConnection = _relaySession?.captureConnectionFence();
      var snapshot = <NostrEvent>[];
      var recovered = false;
      // Refuses only the unread step; the rest of the pass still publishes.
      StateError? unreadUnreadable;
      if (_relaySession != null) {
        final read = ++_snapshotsStarted;
        final heads = Map.of(_followHeads);
        final List<NostrEvent> events;
        try {
          events = await _relaySession.fetchReadStateSnapshot();
        } catch (error) {
          // Retry on the same backoff as a failed sync; the retry re-syncs
          // and publishes the journaled marks.
          if (_settleSnapshot(read, succeeded: false)) _snapshotError = error;
          _scheduleSnapshotRetry();
          rethrow;
        }
        final newest = _settleSnapshot(read, succeeded: true);
        if (newest) {
          // This read was the retry the backoff timer was waiting to make.
          _snapshotRetryAttempt = 0;
          _snapshotRetryTimer?.cancel();
          _snapshotRetryTimer = null;
          if (identical(syncError, _snapshotError)) syncError = null;
          _snapshotError = null;
        }
        if (!_servesThisCommunity ||
            !_ownsJournal ||
            (_disposed && !allowDisposed)) {
          return;
        }
        checkConnection?.call();
        snapshot = events;
        _mergeEvents(events);
        if (newest) _dropAbsentPeerFollows(heads, _capacitySnapshot = events);
        _followsLoaded = recovered = true;
        _retryRefusedReadingClears();
        // Unchanged slots publish nothing, so no echo would announce marks
        // this snapshot brought in.
        _onChanged();
        if (events.any(
          (event) =>
              hasValidUnreadStateTags(event) &&
              event.tags.any(
                (tag) =>
                    tag.length >= 2 &&
                    tag.first == 'd' &&
                    tag[1] == _unreadCoordinate,
              ) &&
              decodeReadStateEvent(
                    event,
                    pubkey: pubkey,
                    decrypt: _crypto.decrypt,
                  ) ==
                  null,
        )) {
          unreadUnreadable = StateError(
            'Unreadable unread-state coordinate blocks publication',
          );
        }
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
      _localContexts.removeWhere(
        (key, value) => value < _now() - readStateHorizonSeconds,
      );
      _discardOverrides();
      _placement = _planSlots();
      while (_slots.length < _placement.length) {
        _slots.add(generateReadStateSlotId());
      }
      await _persistLocalState();
      if (recovered) _followsRecovered = true;
      final planned = [
        for (final blob in _placement) Map<String, int>.of(blob),
      ];
      final prepared = <String, NostrEvent>{};
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
          createdAt: _reserve(),
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
      final unreadReady = _unreadPublishable && unreadUnreadable == null;
      final unread = _planUnread();
      final unreadCoordinate = _unreadCoordinate;
      if (unreadReady &&
          (!mapEquals(_unreadPublished, unread) ||
              _pending.containsKey(unreadCoordinate))) {
        prepared[unreadCoordinate] = NostrEvent(
          id: '0' * 64,
          pubkey: pubkey,
          sig: '0' * 128,
          kind: EventKind.readState,
          createdAt: _reserve(),
          tags: [
            ['d', unreadCoordinate],
            ['t', readStateUnreadTag],
          ],
          content: _crypto.encrypt(
            jsonEncode(
              ReadStateBlob(
                clientId: _clientId,
                contexts: unread,
                version: 2,
              ).toJson(),
            ),
          ),
        );
      }
      // Each follow candidate is frozen in one synchronous step: plaintext,
      // captured edit generations, basis head and reservation together. Only
      // then is the reservation journaled, before anything is signed.
      final followPlanned = _canPublishFollows
          ? _plannedFollowSlots()
          : const <String, String>{};
      final followCaptures = <String, Map<String, int>>{};
      final followBases = <String, String?>{};
      for (final MapEntry(key: slot, value: plaintext)
          in followPlanned.entries) {
        final coordinate = '$threadFollowsDTagPrefix$slot';
        if (_followPublished[slot] == plaintext ||
            _pending.containsKey(coordinate)) {
          continue;
        }
        final placed = _followPlacement[_followSlots.indexOf(slot)];
        followCaptures[coordinate] = {
          for (final key in placed) key: ?_followEdits[key],
        };
        followBases[coordinate] = _followHeads[slot]?.id;
        prepared[coordinate] = NostrEvent(
          id: '0' * 64,
          pubkey: pubkey,
          sig: '0' * 128,
          kind: EventKind.readState,
          createdAt: _reserve(),
          tags: [
            ['d', coordinate],
            ['t', threadFollowsTag],
          ],
          content: _crypto.encrypt(plaintext),
        );
      }
      await _persistLocalState();
      // Check replay and successor prefixes before sending. Not a reservation:
      // another device can still grow the remote snapshot concurrently.
      _checkSnapshotCapacity(snapshot, _pending.values);
      // A follow candidate that overfills the snapshot waits, visibly; read
      // positions that fit still publish.
      Object? followsFull, unreadFull;
      try {
        _checkSnapshotCapacity(snapshot, [
          ..._pending.values,
          ...prepared.values,
        ]);
      } on StateError {
        prepared.removeWhere(
          (coordinate, _) => coordinate.startsWith(threadFollowsDTagPrefix),
        );
        _checkSnapshotCapacity(snapshot, [
          ..._pending.values,
          ...prepared.values,
        ]);
        followsFull = StateError(threadFollowsFullMessage);
      }
      for (final entry in _pending.entries.toList()) {
        if (entry.key.startsWith(threadFollowsDTagPrefix) &&
            !_canPublishFollows) {
          continue; // Held with its edits; the pass reports why below.
        }
        if (entry.key == unreadCoordinate &&
            (unreadUnreadable != null || !_unreadStillCurrent(unread))) {
          continue; // Kept until no held-back group could need its carrier.
        }
        if (_relaySession == null) {
          throw StateError('No relay for pending retry');
        }
        if (!_servesThisCommunity ||
            !_ownsJournal ||
            (_disposed && !allowDisposed)) {
          return;
        }
        checkConnection?.call();
        try {
          await _relaySession.publish(entry.value);
        } on Exception catch (error) {
          if (!_isTimestampRefusal(error)) rethrow;
          await _recoverRefusedPending(entry.key, entry.value);
          // Plan again from the refreshed state; the successor goes out then.
          _publishAgain = true;
          return;
        }
        if (!_servesThisCommunity ||
            !_ownsJournal ||
            (_disposed && !allowDisposed)) {
          return;
        }
        unreadFull = _acknowledgeOrHold(entry.key, entry.value) ?? unreadFull;
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
        _maxFetchedCreatedAt = max(_maxFetchedCreatedAt, event.createdAt);
        await _persistLocalState();
      }
      if (prepared[unreadCoordinate] case final event?
          when !mapEquals(_unreadPublished, unread) &&
              _unreadStillCurrent(unread)) {
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
            _pending[unreadCoordinate] = event;
            await _persistLocalState();
            checkConnection?.call();
            if (!_unreadStillCurrent(unread)) {
              throw StateError('A held-back unread group defers publication');
            }
            if (!_servesThisCommunity ||
                !_ownsJournal ||
                (_disposed && !allowDisposed) ||
                _unreadCoordinate != unreadCoordinate) {
              throw StateError(
                'Unread-state owner or coordinate retired before publication',
              );
            }
          },
        );
        if (!_servesThisCommunity ||
            !_ownsJournal ||
            (_disposed && !allowDisposed)) {
          return;
        }
        // `OK true` (a `duplicate:` no-op included) doesn't prove this
        // became the head: a newer own head supersedes it, so its intent is
        // rebuilt above that head rather than acknowledged.
        final head = _unreadHead;
        final superseded =
            head != null && head.id != event.id && _outranks(head, event);
        if (!superseded) _unreadPublished = unread;
        _pending.remove(unreadCoordinate);
        _maxFetchedCreatedAt = max(_maxFetchedCreatedAt, event.createdAt);
        await _persistLocalState();
        if (superseded) _schedulePublish();
      }
      for (final MapEntry(key: slot, value: plaintext)
          in followPlanned.entries) {
        final coordinate = '$threadFollowsDTagPrefix$slot';
        final event = prepared[coordinate];
        if (_followPublished[slot] == plaintext) continue;
        if (followsFull != null) continue;
        if (event == null) {
          // Skipped for a pending replay; send the newer choice next pass.
          _publishAgain = true;
          continue;
        }
        if (!_servesThisCommunity ||
            !_ownsJournal ||
            (_disposed && !allowDisposed)) {
          return;
        }
        checkConnection?.call();
        final accepted = await signedEventRelay.submit(
          kind: event.kind,
          content: event.content,
          tags: event.tags,
          createdAt: event.createdAt,
          onSigned: (event) async {
            // The accepted head moved while signing: this payload is stale.
            // Recompose next pass rather than sending or re-stamping it.
            if (_followHeads[slot]?.id != followBases[coordinate] ||
                !_followSlots.contains(slot)) {
              throw StateError('Thread follow head moved before publication');
            }
            _pending[coordinate] = event;
            _followCaptured[coordinate] = followCaptures[coordinate]!;
            await _persistLocalState();
            checkConnection?.call();
            if (!_servesThisCommunity ||
                !_ownsJournal ||
                (_disposed && !allowDisposed) ||
                !_followSlots.contains(slot)) {
              throw StateError(
                'Thread follow owner or coordinate retired before publication',
              );
            }
          },
        );
        if (!_servesThisCommunity ||
            !_ownsJournal ||
            (_disposed && !allowDisposed)) {
          return;
        }
        // The OK result is a placeholder carrying only the event ID; resolve
        // it against the exact signed packet retained in onSigned. An echo
        // may already have resolved it, or a successor replaced it.
        if (_pending[coordinate] case final retained?
            when retained.id == accepted.id) {
          _acknowledge(coordinate, retained);
        }
        await _persistLocalState();
      }
      if (!_canPublishFollows && _ownFollows.isNotEmpty) {
        throw StateError(
          'Unreadable thread follow coordinate blocks publication',
        );
      }
      if (unreadUnreadable ?? followsFull ?? unreadFull case final error?) {
        throw error;
      }
      // An older send must not erase a newer read's failure.
      if (_lastSnapshotSucceeded) syncError = _standingError;
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

  Set<String> drainSyncedAdvances() {
    final drained = Set<String>.from(_pendingSyncedAdvances);
    _pendingSyncedAdvances.clear();
    return drained;
  }

  /// Switching community rebuilds the session in place before this manager
  /// is retired, so in between it must neither take in nor publish data.
  bool get _servesThisCommunity {
    final served = _relaySession?.relayUrl;
    return served == null || served == relayUrl;
  }

  bool _isPlausibleCreatedAt(int createdAt) =>
      createdAt <= _now() + readStateMaxClockDriftSeconds;
}
