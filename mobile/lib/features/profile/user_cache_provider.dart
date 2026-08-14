import 'dart:async';

import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/crypto/nip_oa.dart';
import '../../shared/relay/relay.dart';
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
        final data = ProfileData.fromEvent(event);
        final pk = data.pubkey.toLowerCase();
        updated[pk] = UserProfile(
          pubkey: pk,
          displayName: data.displayName,
          avatarUrl: data.avatarUrl,
          about: data.about,
          nip05Handle: data.nip05,
          ownerPubkey: verifiedOaOwnerPubkey(event.tags, event.pubkey),
        );
      }

      state = updated;
    } catch (_) {
      // Silently fail — we'll just show pubkeys.
    }
  }
}

final userCacheProvider =
    NotifierProvider<UserCacheNotifier, Map<String, UserProfile>>(
      UserCacheNotifier.new,
    );
