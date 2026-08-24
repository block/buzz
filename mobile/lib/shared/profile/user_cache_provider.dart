import 'dart:async';

import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../crypto/nip_oa.dart';
import '../relay/relay.dart';
import 'user_profile.dart';

/// In-memory cache of user profiles, fetched in batches from the relay.
///
/// Lookups requested via [get] or [preload] are coalesced into a single
/// kind:0 batch query (NIP-01 `authors` filter) every 50ms.
class UserCacheNotifier extends Notifier<Map<String, UserProfile>> {
  final Set<String> _pending = {};
  Timer? _batchTimer;
  Completer<bool>? _batchCompleter;

  @override
  Map<String, UserProfile> build() {
    ref.watch(relayConfigProvider);
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
    try {
      final session = ref.read(relaySessionProvider.notifier);
      final events = await session.fetchHistory(
        NostrFilters.profilesBatch(normalized),
      );
      final updated = Map<String, UserProfile>.from(state);
      for (final event in events) {
        final profile = _profileFromEvent(event);
        updated[profile.pubkey] = profile;
      }
      state = updated;
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
    final profile = _profileFromEvent(event);
    state = {...state, profile.pubkey: profile};
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

    var succeeded = false;
    try {
      final session = ref.read(relaySessionProvider.notifier);
      final events = await session.fetchHistory(
        NostrFilters.profilesBatch(pubkeys),
      );

      final updated = Map<String, UserProfile>.from(state);
      for (final event in events) {
        final profile = _profileFromEvent(event);
        updated[profile.pubkey] = profile;
      }

      state = updated;
      succeeded = true;
    } catch (_) {
      // Silently fail — non-gating callers will just show pubkeys.
    } finally {
      completer?.complete(succeeded);
    }
  }

  UserProfile _profileFromEvent(NostrEvent event) {
    final data = ProfileData.fromEvent(event);
    final pubkey = data.pubkey.toLowerCase();
    return UserProfile(
      pubkey: pubkey,
      displayName: data.displayName,
      avatarUrl: data.avatarUrl,
      about: data.about,
      nip05Handle: data.nip05,
      ownerPubkey: verifiedOaOwnerPubkey(event.tags, event.pubkey),
    );
  }
}

final userCacheProvider =
    NotifierProvider<UserCacheNotifier, Map<String, UserProfile>>(
      UserCacheNotifier.new,
    );
