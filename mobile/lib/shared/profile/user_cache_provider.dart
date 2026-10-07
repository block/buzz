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

  @override
  Map<String, UserProfile> build() {
    ref.watch(relayConfigProvider);
    ref.onDispose(() {
      _batchTimer?.cancel();
      _batchTimer = null;
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
  void preload(List<String> pubkeys) {
    final uncached = pubkeys
        .map((pk) => pk.toLowerCase())
        .where((pk) => !state.containsKey(pk) && !_pending.contains(pk))
        .toList();
    if (uncached.isEmpty) return;
    _pending.addAll(uncached);
    _batchTimer ??= Timer(const Duration(milliseconds: 50), _flushPending);
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
    _batchTimer ??= Timer(const Duration(milliseconds: 50), _flushPending);
  }

  Future<void> _flushPending() async {
    _batchTimer = null;
    if (_pending.isEmpty) return;

    final pubkeys = _pending.toList();
    _pending.clear();

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
    } catch (_) {
      // Silently fail — we'll just show pubkeys.
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
