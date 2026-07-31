import 'dart:async';
import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/crypto/nip_oa.dart';
import '../../shared/relay/relay.dart';
import 'user_cache_provider.dart';
import 'user_profile.dart';

Map<String, dynamic> mergeProfileMetadata(
  String? currentContent, {
  required String displayName,
  required String avatarUrl,
  required String about,
}) {
  final trimmedDisplayName = displayName.trim();
  if (trimmedDisplayName.isEmpty) {
    throw ArgumentError.value(displayName, 'displayName', 'must not be empty');
  }

  final metadata = <String, dynamic>{};
  if (currentContent != null) {
    try {
      final decoded = jsonDecode(currentContent);
      if (decoded is Map<String, dynamic>) metadata.addAll(decoded);
    } catch (_) {
      // Replace malformed metadata with a valid profile snapshot.
    }
  }

  metadata['display_name'] = trimmedDisplayName;
  _setOptionalProfileField(metadata, 'picture', avatarUrl);
  _setOptionalProfileField(metadata, 'about', about);
  return metadata;
}

void _setOptionalProfileField(
  Map<String, dynamic> metadata,
  String key,
  String value,
) {
  final trimmed = value.trim();
  if (trimmed.isEmpty) {
    metadata.remove(key);
  } else {
    metadata[key] = trimmed;
  }
}

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
    return _profileFromEvent(_latestProfileEvent(events));
  }

  Future<void> refresh() async {
    state = await AsyncValue.guard(_fetch);
  }

  Future<UserProfile> updateProfile({
    required String displayName,
    required String avatarUrl,
    required String about,
  }) async {
    final config = ref.read(relayConfigProvider);
    final myPk = pubkeyFromNsec(config.nsec);
    if (myPk == null) {
      throw StateError('Cannot update profile without an active identity');
    }

    final session = ref.read(relaySessionProvider.notifier);
    final events = await session.fetchHistory(NostrFilters.profile(myPk));
    final currentEvent = events.isEmpty ? null : _latestProfileEvent(events);
    final metadata = mergeProfileMetadata(
      currentEvent?.content,
      displayName: displayName,
      avatarUrl: avatarUrl,
      about: about,
    );
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final createdAt = currentEvent != null && currentEvent.createdAt >= now
        ? currentEvent.createdAt + 1
        : now;
    if (!_isSameRelayIdentity(config)) {
      throw StateError('Active identity changed while editing the profile');
    }
    final relay = SignedEventRelay(session: session, nsec: config.nsec);
    NostrEvent? signedEvent;

    await relay.submit(
      kind: EventKind.metadata,
      content: jsonEncode(metadata),
      tags: currentEvent?.tags ?? const [],
      createdAt: createdAt,
      onSigned: (event) => signedEvent = event,
    );

    final event = signedEvent;
    if (event == null) {
      throw StateError('Profile event was not signed');
    }
    final updated = _profileFromEvent(event);
    if (_isSameRelayIdentity(config)) {
      state = AsyncData(updated);
      ref.read(userCacheProvider.notifier).put(updated);
    }
    return updated;
  }

  bool _isSameRelayIdentity(RelayConfig expected) {
    final active = ref.read(relayConfigProvider);
    return active.nsec == expected.nsec && active.baseUrl == expected.baseUrl;
  }
}

final profileProvider = AsyncNotifierProvider<ProfileNotifier, UserProfile?>(
  ProfileNotifier.new,
);

NostrEvent _latestProfileEvent(List<NostrEvent> events) =>
    events.reduce((a, b) {
      if (a.createdAt != b.createdAt) {
        return a.createdAt > b.createdAt ? a : b;
      }
      return a.id.compareTo(b.id) <= 0 ? a : b;
    });

UserProfile _profileFromEvent(NostrEvent event) {
  final data = ProfileData.fromEvent(event);
  return UserProfile(
    pubkey: data.pubkey,
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
