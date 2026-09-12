import 'dart:async';

import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../community/community_provider.dart';
import '../crypto/nip_oa.dart';
import '../crypto/signed_event.dart';
import '../push/push_presentation_cache.dart';
import '../relay/relay.dart';
import 'user_profile.dart';

/// A cache-issued capability bound to the request or subscription's context.
/// Capture before starting asynchronous work; never mint one in a late callback.
class ProfileAdmission {
  ProfileAdmission._(this._cache, this._config, this._generation);
  final UserCacheNotifier _cache;
  final RelayConfig _config;
  final int _generation;

  /// Whether this originating context still owns the cache.
  bool get isCurrent => _cache._admits(_config, _generation);

  /// Applies ordered evidence only while the originating context is current.
  void add(NostrEvent event) {
    if (isCurrent) _cache._acceptProfileEvent(event);
  }
}

/// In-memory cache of user profiles, fetched in batches from the relay.
///
/// Lookups requested via [get] or [preload] are coalesced into a single
/// kind:0 batch query (NIP-01 `authors` filter) every 50ms.
class UserCacheNotifier extends Notifier<Map<String, UserProfile>> {
  final Set<String> _pending = {};
  final Map<String, ({int createdAt, String eventId})> _profileEventOrders = {};
  int _generation = 0;
  Timer? _batchTimer;
  Completer<bool>? _batchCompleter;

  @override
  Map<String, UserProfile> build() {
    ref.watch(relayConfigProvider);
    _profileEventOrders.clear();
    _pending.clear();
    _generation++;
    ref.onDispose(() {
      _batchTimer?.cancel();
      _batchTimer = null;
      _batchCompleter?.complete(false);
      _batchCompleter = null;
    });
    return {};
  }

  /// Request a profile for [pubkey]. Returns immediately from cache if
  /// available, otherwise schedules a batch fetch.
  UserProfile? get(String pubkey) {
    final cached = state[pubkey.toLowerCase()];
    if (cached != null) return cached;
    _scheduleFetch(pubkey.toLowerCase());
    return null;
  }

  /// Current cache lifetime, advanced even by same-context invalidation.
  int get generation => _generation;

  /// Keys with ordered evidence, distinct from display-only seeds.
  Set<String> get profilePubkeys => _profileEventOrders.keys.toSet();

  /// Owner projection of governing evidence only; display seeds grant nothing.
  Map<String, String> get profileOwners => {
    for (final key in profilePubkeys)
      if (state[key]?.ownerPubkey case final String owner) key: owner,
  };

  /// Seeds display data only until an ordered profile has been observed.
  void put(UserProfile profile) {
    if (_profileEventOrders.containsKey(profile.pubkey.toLowerCase())) return;
    state = {...state, profile.pubkey.toLowerCase(): profile};
  }

  /// Preload profiles for a list of pubkeys (e.g. channel members).
  /// Returns whether the batch completed successfully.
  Future<bool> preload(List<String> pubkeys) {
    final normalized = pubkeys.map((pk) => pk.toLowerCase()).toSet();
    final alreadyPending = normalized.any(_pending.contains);
    final uncached = normalized
        .map((pk) => pk.toLowerCase())
        .where((pk) => !state.containsKey(pk) && !_pending.contains(pk))
        .toList();
    if (uncached.isEmpty && !alreadyPending) return Future.value(true);
    _pending.addAll(uncached);
    final completer = _batchCompleter ??= Completer<bool>();
    _batchTimer ??= Timer(const Duration(milliseconds: 50), _flushPending);
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
    final admission = captureAdmission();
    try {
      final session = ref.read(relaySessionProvider.notifier);
      final events = await session.fetchHistory(
        NostrFilters.profilesBatch(normalized),
      );
      final updated = Map<String, UserProfile>.from(state);
      if (!admission.isCurrent) return false;
      final updatedOrders = Map<String, ({int createdAt, String eventId})>.from(
        _profileEventOrders,
      );
      for (final event in events) {
        _cacheProfileEvent(event, updated, updatedOrders);
      }
      _profileEventOrders
        ..clear()
        ..addAll(updatedOrders);
      state = updated;
      return true;
    } catch (_) {
      return false;
    }
  }

  /// Captures authority for profile ingress before a request/subscription starts.
  ProfileAdmission captureAdmission() {
    final config = ref.read(relayConfigProvider);
    final _ = state;
    return ProfileAdmission._(this, config, _generation);
  }

  bool _admits(RelayConfig config, int generation) {
    if (!ref.mounted || ref.read(relayConfigProvider) != config) return false;
    final _ = state; // Resolve lazy invalidation before comparing.
    return generation == _generation;
  }

  /// Applies a live kind:0 profile event to the cache.
  ///
  /// Surfaces that keep a participant-scoped profile subscription can use this
  /// to update names and avatars without discarding the rest of the cache.
  void _acceptProfileEvent(NostrEvent event) {
    if (event.kind != 0) return;
    final updated = Map<String, UserProfile>.from(state);
    if (_cacheProfileEvent(event, updated)) state = updated;
  }

  void _scheduleFetch(String pubkey) {
    if (state.containsKey(pubkey) || _pending.contains(pubkey)) return;
    _pending.add(pubkey);
    _batchCompleter ??= Completer<bool>();
    _batchTimer ??= Timer(const Duration(milliseconds: 50), _flushPending);
  }

  Future<void> _flushPending() async {
    _batchTimer = null;
    if (_pending.isEmpty) return;

    final pubkeys = _pending.toList();
    _pending.clear();
    final completer = _batchCompleter;
    _batchCompleter = null;

    final admission = captureAdmission();
    var succeeded = false;
    try {
      final communityID = ref.read(activeCommunityProvider).value?.id;
      final session = ref.read(relaySessionProvider.notifier);
      final events = await session.fetchHistory(
        NostrFilters.profilesBatch(pubkeys),
      );

      final updated = Map<String, UserProfile>.from(state);
      if (!admission.isCurrent) return;
      final updatedOrders = Map<String, ({int createdAt, String eventId})>.from(
        _profileEventOrders,
      );
      for (final event in events) {
        _cacheProfileEvent(event, updated, updatedOrders);
      }

      _profileEventOrders
        ..clear()
        ..addAll(updatedOrders);
      state = updated;
      if (communityID != null) {
        unawaited(cacheBuzzPushProfileEvents(communityID, events));
      }
      succeeded = true;
    } catch (_) {
      // Silently fail — non-gating callers will just show pubkeys.
    } finally {
      completer?.complete(succeeded);
    }
  }

  bool _cacheProfileEvent(
    NostrEvent event,
    Map<String, UserProfile> profiles, [
    Map<String, ({int createdAt, String eventId})>? orders,
  ]) {
    if (event.kind != 0 || !verifySignedEvent(event)) return false;
    final eventOrders = orders ?? _profileEventOrders;
    final pubkey = event.pubkey.toLowerCase();
    final current = eventOrders[pubkey];
    final isNewer =
        current == null ||
        event.createdAt > current.createdAt ||
        (event.createdAt == current.createdAt &&
            event.id.compareTo(current.eventId) < 0);
    if (!isNewer) return false;

    profiles[pubkey] = _profileFromEvent(event);
    eventOrders[pubkey] = (createdAt: event.createdAt, eventId: event.id);
    return true;
  }

  UserProfile _profileFromEvent(NostrEvent event) {
    ProfileData data;
    try {
      data = ProfileData.fromEvent(event);
    } catch (_) {
      data = ProfileData(pubkey: event.pubkey);
    }
    final pubkey = data.pubkey.toLowerCase();
    return UserProfile(
      pubkey: pubkey,
      displayName: data.displayName,
      avatarUrl: data.avatarUrl,
      about: data.about,
      nip05Handle: data.nip05,
      ownerPubkey: verifiedOaOwnerPubkey(event),
    );
  }
}

final userCacheProvider =
    NotifierProvider<UserCacheNotifier, Map<String, UserProfile>>(
      UserCacheNotifier.new,
    );
