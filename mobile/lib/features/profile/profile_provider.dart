import 'dart:async';
import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/profile/user_cache_provider.dart';
import '../../shared/profile/user_profile.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';

/// Signals that a profile write no longer belongs to the active community.
class ProfileCommunityChangedException extends StateError {
  ProfileCommunityChangedException()
    : super('Profile update cancelled because the active community changed.');
}

/// The current user's profile (kind:0 metadata) loaded over the relay
/// WebSocket. Returns null when no nsec is configured or when the user has
/// not yet published a profile.
class ProfileNotifier extends AsyncNotifier<UserProfile?> {
  Map<String, dynamic> _metadata = {};
  bool _hasHydrated = false;
  int _lastCreatedAt = 0;
  Future<void> _patchQueue = Future.value();

  @override
  Future<UserProfile?> build() {
    final config = ref.watch(relayConfigProvider);
    final pubkey = ref.watch(myPubkeyProvider);
    ref.watch(relaySessionProvider);
    final context = _ProfileWriteContext(
      config: config,
      pubkey: pubkey,
      session: ref.read(relaySessionProvider.notifier),
    );
    _hasHydrated = false;
    return _fetch(context);
  }

  Future<UserProfile?> _fetch(_ProfileWriteContext context) async {
    final myPk = context.pubkey;
    if (myPk == null) {
      _requireCurrentWriteContext(context);
      _metadata = {};
      _lastCreatedAt = 0;
      _hasHydrated = true;
      return null;
    }

    final session = context.session;
    final events = await session.fetchHistory(NostrFilters.profile(myPk));
    if (events.isEmpty) {
      _requireCurrentWriteContext(context);
      _metadata = {};
      _lastCreatedAt = 0;
      _hasHydrated = true;
      return null;
    }
    final latest = _latestProfileEvent(events)!;
    final metadata = _decodeProfileMetadata(latest);
    final data = ProfileData.fromEvent(latest);
    final profile = UserProfile(
      pubkey: data.pubkey,
      displayName: data.displayName,
      avatarUrl: data.avatarUrl,
      about: data.about,
      nip05Handle: data.nip05,
    );
    _requireCurrentWriteContext(context);
    _metadata = metadata;
    _lastCreatedAt = latest.createdAt;
    _hasHydrated = true;
    return profile;
  }

  Future<void> refresh() async {
    final context = _currentWriteContext();
    _hasHydrated = false;
    state = await AsyncValue.guard(() => _fetch(context));
  }

  /// Updates the current user's display name while preserving the other
  /// metadata fields in their latest kind:0 profile event.
  Future<void> updateDisplayName(String displayName) =>
      _publishProfilePatch({'display_name': displayName.trim()});

  /// Updates the current user's profile description.
  Future<void> updateAbout(String about) =>
      _publishProfilePatch({'about': about.trim()});

  /// Updates the current user's profile photo URL.
  Future<void> updateAvatarUrl(String avatarUrl) =>
      _publishProfilePatch({'picture': avatarUrl.trim()});

  Future<void> _publishProfilePatch(Map<String, dynamic> patch) {
    final context = _currentWriteContext();
    final previous = _patchQueue;
    final released = Completer<void>();
    _patchQueue = released.future;
    return () async {
      await previous;
      try {
        await _publishProfilePatchNow(patch, context);
      } finally {
        released.complete();
      }
    }();
  }

  _ProfileWriteContext _currentWriteContext() => _ProfileWriteContext(
    config: ref.read(relayConfigProvider),
    pubkey: ref.read(myPubkeyProvider),
    session: ref.read(relaySessionProvider.notifier),
  );

  Future<void> _publishProfilePatchNow(
    Map<String, dynamic> patch,
    _ProfileWriteContext context,
  ) async {
    _requireCurrentWriteContext(context);
    if (!_hasHydrated || !state.hasValue) {
      throw StateError('Cannot update profile before metadata is loaded.');
    }
    final pubkey = context.pubkey;
    if (pubkey == null) {
      throw StateError('Cannot update profile without a signing identity.');
    }
    final session = context.session;
    final currentEvents = await session.fetchHistory(
      NostrFilters.profile(pubkey),
    );
    _requireCurrentWriteContext(context);
    final currentHead = _latestProfileEvent(currentEvents);
    if (_lastCreatedAt > 0 &&
        (currentHead == null || currentHead.createdAt < _lastCreatedAt)) {
      throw StateError('Cannot confirm the latest profile metadata.');
    }
    final currentMetadata = currentHead == null
        ? <String, dynamic>{}
        : _decodeProfileMetadata(currentHead);
    final nextMetadata = {...currentMetadata, ...patch};
    final relay = SignedEventRelay(session: session, nsec: context.config.nsec);
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final currentCreatedAt = currentHead?.createdAt ?? 0;
    final previousCreatedAt = currentCreatedAt > _lastCreatedAt
        ? currentCreatedAt
        : _lastCreatedAt;
    final createdAt = now > previousCreatedAt ? now : previousCreatedAt + 1;
    NostrEvent? signedEvent;
    await relay.submit(
      kind: EventKind.profile,
      content: jsonEncode(nextMetadata),
      tags: currentHead?.tags ?? const [],
      createdAt: createdAt,
      onSigned: (event) => signedEvent = event,
    );
    _requireCurrentWriteContext(context);
    final submittedEvent = signedEvent;
    if (submittedEvent == null) {
      throw StateError('Profile update was not signed.');
    }
    final verifiedHead = _latestProfileEvent(
      await session.fetchHistory(NostrFilters.profile(pubkey)),
    );
    _requireCurrentWriteContext(context);
    if (verifiedHead?.id != submittedEvent.id) {
      throw StateError('Profile changed before the update could be confirmed.');
    }

    _metadata = nextMetadata;
    _lastCreatedAt = createdAt;
    final profile = UserProfile(
      pubkey: pubkey,
      displayName:
          _metadata['display_name'] as String? ?? _metadata['name'] as String?,
      avatarUrl: _metadata['picture'] as String?,
      about: _metadata['about'] as String?,
      nip05Handle: _metadata['nip05'] as String?,
    );
    state = AsyncData(profile);
    ref.read(userCacheProvider.notifier).put(profile);
  }

  void _requireCurrentWriteContext(_ProfileWriteContext context) {
    final currentConfig = ref.read(relayConfigProvider);
    final currentSession = ref.read(relaySessionProvider.notifier);
    final currentPubkey = ref.read(myPubkeyProvider);
    if (currentConfig.storedOrigin != context.config.storedOrigin ||
        currentConfig.nsec != context.config.nsec ||
        currentPubkey != context.pubkey ||
        !identical(currentSession, context.session)) {
      throw ProfileCommunityChangedException();
    }
  }
}

class _ProfileWriteContext {
  const _ProfileWriteContext({
    required this.config,
    required this.pubkey,
    required this.session,
  });

  final RelayConfig config;
  final String? pubkey;
  final RelaySessionNotifier session;
}

NostrEvent? _latestProfileEvent(List<NostrEvent> events) {
  if (events.isEmpty) return null;
  return events.reduce((current, event) {
    if (event.createdAt != current.createdAt) {
      return event.createdAt > current.createdAt ? event : current;
    }
    // Match the relay replacement tie-breaker: the lowest event id wins.
    return event.id.compareTo(current.id) < 0 ? event : current;
  });
}

Map<String, dynamic> _decodeProfileMetadata(NostrEvent event) {
  final decoded = jsonDecode(event.content);
  if (decoded is! Map<String, dynamic>) {
    throw const FormatException('Profile metadata must be a JSON object.');
  }
  return Map<String, dynamic>.from(decoded);
}

final profileProvider = AsyncNotifierProvider<ProfileNotifier, UserProfile?>(
  ProfileNotifier.new,
);

/// Presence status for the current user.
///
/// Sends a heartbeat every 60s while the app is active by publishing a
/// kind:20001 presence event over the relay WebSocket. Watches
/// [appLifecycleProvider] to send "away" when backgrounded.
class PresenceNotifier extends AsyncNotifier<String> {
  static const _heartbeatInterval = Duration(seconds: 60);
  static const _preferenceKeyPrefix = 'buzz_presence_preference_';

  Timer? _heartbeatTimer;
  String? _preferencePubkey;
  String? _manualPresence;

  @override
  Future<String> build() {
    ref.watch(relaySessionProvider);
    final pubkey = ref.watch(myPubkeyProvider)?.toLowerCase();

    if (_preferencePubkey != pubkey) {
      _preferencePubkey = pubkey;
      final stored = pubkey == null
          ? null
          : ref
                .read(savedPrefsProvider)
                .getString('$_preferenceKeyPrefix$pubkey');
      _manualPresence = stored == 'away' || stored == 'offline' ? stored : null;
    }

    final lifecycle = ref.watch(appLifecycleProvider);

    ref.onDispose(() {
      _heartbeatTimer?.cancel();
      _heartbeatTimer = null;
    });

    final manualPresence = _manualPresence;
    if (manualPresence != null) {
      _heartbeatTimer?.cancel();
      _heartbeatTimer = null;
      return _setPresence(manualPresence);
    }

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

  /// Updates the current user's presence preference and publishes it.
  ///
  /// Online restores automatic lifecycle-driven presence. Away and Offline
  /// remain selected until the user chooses another value.
  Future<void> setPresence(String status) async {
    if (status != 'online' && status != 'away' && status != 'offline') return;

    _manualPresence = status == 'online' ? null : status;
    final pubkey = ref.read(myPubkeyProvider)?.toLowerCase();
    if (pubkey != null) {
      await ref
          .read(savedPrefsProvider)
          .setString('$_preferenceKeyPrefix$pubkey', _manualPresence ?? 'auto');
    }

    if (_manualPresence == null &&
        ref.read(appLifecycleProvider) == AppLifecycleState.resumed) {
      _startHeartbeat();
    } else {
      _heartbeatTimer?.cancel();
      _heartbeatTimer = null;
    }

    state = AsyncData(status);
    await _setPresence(status);
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
