import 'dart:async';
import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/crypto/nip_oa.dart';
import '../../shared/relay/relay.dart';
import 'user_cache_provider.dart';
import 'user_profile.dart';

/// The current user's profile (kind:0 metadata) loaded over the relay
/// WebSocket. Returns null when no nsec is configured or when the user has
/// not yet published a profile.
class ProfileNotifier extends AsyncNotifier<UserProfile?> {
  @override
  Future<UserProfile?> build() {
    ref.watch(relayConfigProvider);
    ref.watch(relaySessionProvider);
    return _fetch();
  }

  Future<UserProfile?> _fetch() async {
    final myPk = ref.read(myPubkeyProvider);
    if (myPk == null) return null;

    final session = ref.read(relaySessionProvider.notifier);
    final events = await session.fetchHistory(NostrFilters.profile(myPk));
    if (events.isEmpty) return null;
    return _userProfileFromEvent(_latest(events));
  }

  Future<void> refresh() async {
    state = await AsyncValue.guard(_fetch);
  }

  /// Publish a kind:0 profile update with the active community identity.
  ///
  /// The existing JSON object is merged before signing so fields outside this
  /// editor survive unchanged. A null [avatarUrl] preserves the current avatar.
  Future<UserProfile> saveProfile({
    required String displayName,
    String? avatarUrl,
  }) async {
    final name = displayName.trim();
    if (name.isEmpty) {
      throw ArgumentError.value(
        displayName,
        'displayName',
        'must not be empty',
      );
    }

    final config = ref.read(relayConfigProvider);
    if (config.nsec == null || config.nsec!.isEmpty) {
      throw StateError('Cannot save profile: no signing key available');
    }

    final relay = SignedEventRelay(
      session: ref.read(relaySessionProvider.notifier),
      nsec: config.nsec,
    );
    final pubkey = relay.pubkey;
    if (pubkey == null) {
      throw StateError('Cannot save profile: invalid signing key');
    }

    final session = ref.read(relaySessionProvider.notifier);
    final events = await session.fetchHistory(NostrFilters.profile(pubkey));
    final previous = events.isEmpty ? null : _latest(events);
    final metadata = _metadataFrom(previous);
    metadata['display_name'] = name;
    if (avatarUrl != null) metadata['picture'] = avatarUrl;

    NostrEvent? signed;
    await relay.submit(
      kind: EventKind.metadata,
      content: jsonEncode(metadata),
      tags: previous?.tags ?? const [],
      createdAt: _nextCreatedAt(previous),
      onSigned: (event) => signed = event,
    );
    final event = signed;
    if (event == null) {
      throw StateError('Profile event was not signed');
    }

    final profile = _userProfileFromEvent(event);
    state = AsyncValue.data(profile);
    ref.read(userCacheProvider.notifier).updateProfile(profile);
    return profile;
  }
}

final profileProvider = AsyncNotifierProvider<ProfileNotifier, UserProfile?>(
  ProfileNotifier.new,
);

NostrEvent _latest(List<NostrEvent> events) => events.reduce((a, b) {
  if (a.createdAt != b.createdAt) {
    return a.createdAt > b.createdAt ? a : b;
  }
  return a.id.compareTo(b.id) <= 0 ? a : b;
});

Map<String, dynamic> _metadataFrom(NostrEvent? event) {
  if (event == null) return {};
  final decoded = jsonDecode(event.content);
  if (decoded is! Map<String, dynamic>) {
    throw const FormatException('Existing profile metadata is not an object');
  }
  return Map<String, dynamic>.from(decoded);
}

int _nextCreatedAt(NostrEvent? previous) {
  final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
  if (previous == null || previous.createdAt < now) return now;
  return previous.createdAt + 1;
}

UserProfile _userProfileFromEvent(NostrEvent event) {
  final data = ProfileData.fromEvent(event);
  return UserProfile(
    pubkey: data.pubkey.toLowerCase(),
    displayName: data.displayName,
    avatarUrl: data.avatarUrl,
    about: data.about,
    nip05Handle: data.nip05,
    ownerPubkey: verifiedOaOwnerPubkey(event.tags, event.pubkey),
  );
}

/// Presence status for the current user.
///
/// Sends a heartbeat every 60s while the app is active by publishing a
/// kind:20001 presence event over the relay WebSocket. Watches
/// [appLifecycleProvider] to send "away" when backgrounded.
class PresenceNotifier extends AsyncNotifier<String> {
  static const _heartbeatInterval = Duration(seconds: 60);

  Timer? _heartbeatTimer;

  @override
  Future<String> build() {
    ref.watch(relaySessionProvider);
    ref.watch(profileProvider);

    final lifecycle = ref.watch(appLifecycleProvider);

    ref.onDispose(() {
      _heartbeatTimer?.cancel();
      _heartbeatTimer = null;
    });

    if (lifecycle == AppLifecycleState.resumed) {
      _startHeartbeat();
      return _setPresence('online');
    } else if (lifecycle == AppLifecycleState.paused ||
        lifecycle == AppLifecycleState.detached) {
      _heartbeatTimer?.cancel();
      _heartbeatTimer = null;
      return _setPresence('away');
    }

    // Default: we don't know. Reflect the most recent state we set, or
    // 'offline' if never set.
    return Future.value('offline');
  }

  void _startHeartbeat() {
    _heartbeatTimer?.cancel();
    _heartbeatTimer = Timer.periodic(_heartbeatInterval, (_) {
      _setPresence('online');
    });
  }

  /// Publish a kind:20001 presence event. Returns the requested status
  /// optimistically — failures are silently absorbed and the next heartbeat
  /// will retry.
  Future<String> _setPresence(String status) async {
    final sessionState = ref.read(relaySessionProvider);
    if (sessionState.status != SessionStatus.connected) return status;
    final config = ref.read(relayConfigProvider);
    final relay = SignedEventRelay(
      session: ref.read(relaySessionProvider.notifier),
      nsec: config.nsec,
    );
    try {
      await relay.submit(
        kind: EventKind.presenceUpdate,
        content: status,
        tags: const [],
      );
    } catch (_) {
      // Heartbeat will retry.
    }
    return status;
  }

  Future<void> refresh() async {
    // No-op: presence is driven by heartbeats and lifecycle, not pulled.
  }
}

final presenceProvider = AsyncNotifierProvider<PresenceNotifier, String>(
  PresenceNotifier.new,
);
