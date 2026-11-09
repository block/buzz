import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../relay/relay.dart';
import '../verified_profiles/verified_profile_directory.dart';
import 'user_profile.dart';
import 'profile_event_parser.dart';
import 'profile_snapshot.dart';

/// Cache of user profiles, fetched in batches from the relay.
///
/// Verified profiles persist per identity and relay ([ProfileSnapshotStorage])
/// so names and avatars render on launch, even on a slow link. Restored
/// profiles are served immediately and revalidated on their first lookup.
///
/// Lookups requested via [get] or [preload] are coalesced into a single
/// kind:0 batch query (NIP-01 `authors` filter) every 50ms. Larger requests
/// drain sequentially in pages bounded by the relay response limit.
class UserCacheNotifier extends Notifier<Map<String, UserProfile>> {
  // The relay clamps each history response to DEFAULT_MAX_PAGE_LIMIT (1000).
  static const _maximumProfilesPerQuery = 1000;
  static const _snapshotWriteDelay = Duration(seconds: 2);

  final Set<String> _pending = {};
  final Map<String, ({int createdAt, String eventId})> _profileEventOrders = {};
  int _generation = 0;
  bool _flushInFlight = false;
  Future<void> _verificationQueue = Future.value();
  Timer? _batchTimer;
  Completer<bool>? _batchCompleter;

  /// Restored from the snapshot and not yet checked with the relay this scope.
  final Set<String> _unrevalidated = {};

  /// Lookups whose batch query is on the wire.
  final Set<String> _inFlight = {};
  void Function()? _snapshotWriter;
  Timer? _snapshotTimer;
  Map<String, UserProfile> _latest = const {};

  @override
  Map<String, UserProfile> build() {
    final config = ref.watch(relayConfigProvider);
    _generation++;
    _profileEventOrders.clear();
    _unrevalidated.clear();
    _inFlight.clear();
    ref.onDispose(() {
      _generation++;
      _pending.clear();
      _batchTimer?.cancel();
      _batchTimer = null;
      _batchCompleter?.complete(false);
      _batchCompleter = null;
      // Flush rather than drop: switching community must not lose the
      // profiles learned since the last write.
      if (_snapshotTimer?.isActive ?? false) _snapshotWriter?.call();
      _snapshotTimer?.cancel();
      _snapshotTimer = null;
      _snapshotWriter = null;
    });
    return _restoreSnapshot(config);
  }

  Map<String, UserProfile> _restoreSnapshot(RelayConfig config) {
    final storage = ref.read(profileSnapshotStorageProvider);
    final pubkey = pubkeyFromNsec(config.nsec);
    if (storage == null || pubkey == null) return {};
    final relay = config.baseUrl;
    final token = storage.capture(pubkey, relay);
    _snapshotWriter = () {
      final profiles = [
        for (final MapEntry(:key, value: order) in _profileEventOrders.entries)
          if (_latest[key] case final profile?)
            (
              profile: profile,
              createdAt: order.createdAt,
              eventId: order.eventId,
            ),
      ];
      storage
          .write(token, profiles)
          .then(
            (saved) {
              if (!saved) debugPrint('[UserCache] snapshot was not saved');
            },
            onError: (Object error, StackTrace stack) {
              debugPrint('[UserCache] snapshot write failed: $error');
            },
          );
    };
    final restored = <String, UserProfile>{};
    for (final parsed in storage.read(pubkey, relay)) {
      final key = parsed.profile.pubkey;
      restored[key] = parsed.profile;
      _profileEventOrders[key] = (
        createdAt: parsed.createdAt,
        eventId: parsed.eventId,
      );
      _unrevalidated.add(key);
    }
    return _latest = restored;
  }

  @override
  set state(Map<String, UserProfile> value) {
    super.state = _latest = value;
    if (_snapshotWriter == null) return;
    _snapshotTimer ??= Timer(_snapshotWriteDelay, () {
      _snapshotTimer = null;
      _snapshotWriter?.call();
    });
  }

  /// Whether [pubkey] has a profile that is current for this scope.
  bool _isCached(String pubkey) =>
      state.containsKey(pubkey) && !_unrevalidated.contains(pubkey);

  /// Request a profile for [pubkey]. Returns immediately from cache if
  /// available, otherwise schedules a batch fetch.
  UserProfile? get(String pubkey) {
    _trackVerified([pubkey]);
    final key = pubkey.toLowerCase();
    _scheduleFetch(key);
    return state[key];
  }

  /// Stores a profile that was fetched or updated outside the batch loader.
  void put(UserProfile profile) {
    state = {...state, profile.pubkey.toLowerCase(): profile};
  }

  /// Resolves profiles through verified batch admission.
  ///
  /// Unlike [preload], failures propagate to callers whose result requires
  /// profile hydration. Set [refreshCached] for a full revalidation; otherwise
  /// cached profiles remain reusable within this scope.
  Future<Map<String, UserProfile>> resolve(
    List<String> pubkeys, {
    bool refreshCached = false,
  }) async {
    final requested = pubkeys.map((pubkey) => pubkey.toLowerCase()).toSet();
    _trackVerified(requested);
    final toFetch = requested
        .where((pubkey) => refreshCached || !_isCached(pubkey))
        .toList();
    final generation = _generation;
    final session = ref.read(relaySessionProvider.notifier);
    for (final batch in _profileQueryBatches(toFetch)) {
      if (!_isCurrent(generation)) {
        throw StateError('Profile request belongs to a retired cache');
      }
      final events = await session.fetchHistory(
        NostrFilters.profilesBatch(batch),
      );
      if (!await _verifyAndMerge(events, generation)) {
        throw StateError('Profile request belongs to a retired cache');
      }
    }
    if (!_isCurrent(generation)) {
      throw StateError('Profile request belongs to a retired cache');
    }
    _unrevalidated.removeAll(toFetch);
    return {for (final pubkey in requested) pubkey: ?state[pubkey]};
  }

  /// Preload profiles for a list of pubkeys (e.g. channel members).
  /// Returns whether the batch completed successfully.
  Future<bool> preload(List<String> pubkeys) {
    _trackVerified(pubkeys);
    final normalized = pubkeys.map((pk) => pk.toLowerCase()).toSet();
    // Restored profiles being revalidated already render; don't wait on them.
    final alreadyPending = normalized.any(
      (pk) => _pending.contains(pk) && !state.containsKey(pk),
    );
    final uncached = normalized
        .where((pk) => !state.containsKey(pk) && !_pending.contains(pk))
        .toList();
    // Restored profiles already render; revalidate them without making the
    // caller wait on the network.
    normalized.where(_unrevalidated.contains).forEach(_scheduleFetch);
    if (uncached.isEmpty && !alreadyPending) return Future.value(true);
    _pending.addAll(uncached);
    final completer = _batchCompleter ??= Completer<bool>();
    _scheduleBatch();
    return completer.future;
  }

  /// Force-refresh profiles for identity-sensitive gates.
  ///
  /// Unlike [preload], this fetches cached pubkeys too so stale human profiles
  /// cannot be trusted after a verified agent-owner profile was published.
  Future<bool> refresh(List<String> pubkeys) async {
    final normalized = pubkeys
        .map((pubkey) => pubkey.toLowerCase())
        .where((pubkey) => pubkey.isNotEmpty)
        .toSet()
        .toList();
    if (normalized.isEmpty) return true;
    _trackVerified(normalized);
    final generation = _generation;
    try {
      final session = ref.read(relaySessionProvider.notifier);
      for (final batch in _profileQueryBatches(normalized)) {
        if (!_isCurrent(generation)) return false;
        final events = await session.fetchHistory(
          NostrFilters.profilesBatch(batch),
        );
        if (!_isCurrent(generation)) return false;
        if (!await _verifyAndMerge(events, generation)) return false;
      }
      _unrevalidated.removeAll(normalized);
      return true;
    } catch (_) {
      return false;
    }
  }

  /// Applies a live kind:0 profile event to the cache.
  ///
  /// Surfaces that keep a participant-scoped profile subscription can use this
  /// to update names and avatars without discarding the rest of the cache.
  void cacheProfileEvent(NostrEvent event) {
    if (event.kind != 0) return;
    final updated = Map<String, UserProfile>.from(state);
    if (_cacheProfileEvent(event, updated)) state = updated;
  }

  /// Anyone whose name is needed may also have a NIP-FI verified name.
  void _trackVerified(Iterable<String> pubkeys) =>
      ref.read(verifiedProfilesProvider.notifier).track(pubkeys);

  void _scheduleFetch(String pubkey) {
    if (_isCached(pubkey) ||
        _pending.contains(pubkey) ||
        _inFlight.contains(pubkey)) {
      return;
    }
    _pending.add(pubkey);
    _batchCompleter ??= Completer<bool>();
    _scheduleBatch();
  }

  Future<void> _flushPending() async {
    _batchTimer = null;
    if (_pending.isEmpty) return;

    final pubkeys = _pending.toList();
    _pending.clear();
    final completer = _batchCompleter;
    _batchCompleter = null;

    _flushInFlight = true;
    _inFlight.addAll(pubkeys);
    final generation = _generation;
    var succeeded = false;
    try {
      final session = ref.read(relaySessionProvider.notifier);
      var remaining = pubkeys.length;
      for (final batch in _profileQueryBatches(pubkeys)) {
        if (!_isCurrent(generation)) return;
        final events = await session.fetchHistory(
          NostrFilters.profilesBatch(batch),
        );
        if (!_isCurrent(generation)) return;
        if (!await _verifyAndMerge(events, generation)) return;
        // Confirmed by the relay, whether or not it still has a profile.
        _unrevalidated.removeAll(batch);
        remaining -= batch.length;
        if (remaining == 0) {
          // All requested pages are ready.
          succeeded = true;
          completer?.complete(true);
        }
      }
    } catch (_) {
      // Silently fail — non-gating callers will just show pubkeys, or the
      // restored profile, which the next lookup revalidates again.
    } finally {
      if (_isCurrent(generation)) _inFlight.removeAll(pubkeys);
      if (completer != null && !completer.isCompleted) {
        completer.complete(succeeded);
      }
      _flushInFlight = false;
      if (ref.mounted && _pending.isNotEmpty) _scheduleBatch();
    }
  }

  Iterable<List<String>> _profileQueryBatches(List<String> pubkeys) sync* {
    for (
      var start = 0;
      start < pubkeys.length;
      start += _maximumProfilesPerQuery
    ) {
      final end = start + _maximumProfilesPerQuery;
      yield pubkeys.sublist(start, end < pubkeys.length ? end : pubkeys.length);
    }
  }

  void _scheduleBatch() {
    if (_flushInFlight) return;
    _batchTimer ??= Timer(const Duration(milliseconds: 50), _flushPending);
  }

  Future<bool> _verifyAndMerge(List<NostrEvent> events, int generation) {
    // Both refresh and preload share one worker slot. Keep this queue across
    // community changes so an old worker cannot overlap a new community's job.
    final result = _verificationQueue.then((_) async {
      if (!_isCurrent(generation)) return false;
      // Match live-cache admission before parsing. In particular, a malformed
      // stale profile must not fail a refresh that previously ignored it.
      final pending = [
        for (final event in events)
          if (event.kind == 0 &&
              _isNewer(event.pubkey.toLowerCase(), event.createdAt, event.id))
            event,
      ];
      if (pending.isEmpty) return true;
      final profiles = await ref.read(profileEventBatchParserProvider)(pending);
      if (!_isCurrent(generation)) return false;
      _mergeParsedProfiles(profiles);
      return true;
    });
    // Callers receive the original error; only the sequencing tail recovers so
    // one rejected batch does not poison all future refreshes.
    _verificationQueue = result.then<void>(
      (_) {},
      onError: (Object error, StackTrace stack) {},
    );
    return result;
  }

  bool _isCurrent(int generation) => ref.mounted && generation == _generation;

  void _mergeParsedProfiles(List<ParsedProfileEvent> profiles) {
    // Read the current state after the isolate completes: live events and other
    // batches may have installed newer profiles while this batch was parsing.
    final updated = Map<String, UserProfile>.from(state);
    var changed = false;
    for (final parsed in profiles) {
      if (!_isNewer(parsed.profile.pubkey, parsed.createdAt, parsed.eventId)) {
        continue;
      }
      updated[parsed.profile.pubkey] = parsed.profile;
      _profileEventOrders[parsed.profile.pubkey] = (
        createdAt: parsed.createdAt,
        eventId: parsed.eventId,
      );
      changed = true;
    }
    if (changed) state = updated;
  }

  bool _isNewer(String pubkey, int createdAt, String eventId) {
    final current = _profileEventOrders[pubkey];
    return current == null ||
        createdAt > current.createdAt ||
        (createdAt == current.createdAt &&
            eventId.compareTo(current.eventId) < 0);
  }

  bool _cacheProfileEvent(NostrEvent event, Map<String, UserProfile> profiles) {
    if (event.kind != 0) return false;
    final pubkey = event.pubkey.toLowerCase();
    if (!_isNewer(pubkey, event.createdAt, event.id)) return false;
    profiles[pubkey] = parseProfileEvent(event).profile;
    _profileEventOrders[pubkey] = (
      createdAt: event.createdAt,
      eventId: event.id,
    );
    return true;
  }
}

/// Profile history parser, isolated from provider state so batch completion can
/// be delayed independently of relay delivery.
final profileEventBatchParserProvider =
    Provider<Future<List<ParsedProfileEvent>> Function(List<NostrEvent>)>(
      (ref) => parseProfileEventBatch,
    );

final userCacheProvider =
    NotifierProvider<UserCacheNotifier, Map<String, UserProfile>>(
      UserCacheNotifier.new,
    );
