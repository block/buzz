import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter/foundation.dart';

import '../relay/relay.dart';
import 'community_theme_preference.dart';

class CommunityThemeCrypto {
  final String Function(String) encrypt;
  final String Function(String) decrypt;

  const CommunityThemeCrypto({required this.encrypt, required this.decrypt});
}

enum CommunityThemeRemoteStatus { valid, absent, invalid, unavailable }

class RemoteCommunityTheme {
  final CommunityThemePreference preference;
  final int createdAt;
  final String eventId;

  const RemoteCommunityTheme({
    required this.preference,
    required this.createdAt,
    required this.eventId,
  });
}

class CommunityThemeRemoteResult {
  final CommunityThemeRemoteStatus status;
  final RemoteCommunityTheme? remote;

  const CommunityThemeRemoteResult(this.status, [this.remote]);
}

class CommunityThemeSyncManager {
  final String pubkey;
  final RelaySessionNotifier relaySession;
  final SignedEventRelay signedEventRelay;
  final CommunityThemeCrypto crypto;
  final Duration debounce;
  final void Function(RemoteCommunityTheme) onRemote;

  Timer? _publishTimer;
  void Function()? _unsubscribe;
  CommunityThemePreference? _pending;
  CommunityThemePreference? _lastPublished;
  int _lastCreatedAt = 0;
  String _lastEventId = '';
  bool _disposed = false;

  CommunityThemeSyncManager({
    required this.pubkey,
    required this.relaySession,
    required this.signedEventRelay,
    required this.crypto,
    required this.onRemote,
    this.debounce = const Duration(seconds: 2),
  });

  CommunityThemePreference? get pending => _pending;

  Future<CommunityThemeRemoteResult> fetchRemote() async {
    try {
      final events = await relaySession.fetchHistory(
        NostrFilter(
          kinds: const [EventKind.readState],
          authors: [pubkey],
          tags: const {
            '#d': [communityThemeDTag],
          },
          limit: 1,
        ),
      );
      if (events.isEmpty) {
        return const CommunityThemeRemoteResult(
          CommunityThemeRemoteStatus.absent,
        );
      }
      final event = events.reduce(_newerEvent);
      final remote = _decode(event);
      return remote == null
          ? const CommunityThemeRemoteResult(CommunityThemeRemoteStatus.invalid)
          : CommunityThemeRemoteResult(
              CommunityThemeRemoteStatus.valid,
              remote,
            );
    } catch (_) {
      return const CommunityThemeRemoteResult(
        CommunityThemeRemoteStatus.unavailable,
      );
    }
  }

  Future<CommunityThemeRemoteResult> initialize(
    CommunityThemePreference local,
  ) async {
    final result = await fetchRemote();
    if (_disposed) return result;
    if (result.status == CommunityThemeRemoteStatus.valid) {
      _accept(result.remote!);
    } else if (result.status == CommunityThemeRemoteStatus.absent) {
      publish(local);
    }
    try {
      _unsubscribe = await relaySession.subscribe(
        NostrFilter(
          kinds: const [EventKind.readState],
          authors: [pubkey],
          tags: const {
            '#d': [communityThemeDTag],
          },
          limit: 0,
        ),
        (event) {
          if (_disposed) return;
          final remote = _decode(event);
          if (remote != null) _accept(remote);
        },
      );
    } catch (_) {
      // History and the local cache remain usable without a live subscription.
    }
    return result;
  }

  void publish(CommunityThemePreference preference) {
    if (_disposed) return;
    _pending = preference;
    _publishTimer?.cancel();
    _publishTimer = Timer(debounce, () {
      _publishTimer = null;
      unawaited(flush());
    });
  }

  void cancelPending() {
    _publishTimer?.cancel();
    _publishTimer = null;
    _pending = null;
  }

  Future<void> flush() async {
    final preference = _pending;
    if (_disposed || preference == null || preference == _lastPublished) return;
    try {
      final content = crypto.encrypt(jsonEncode(preference.toJson()));
      if (_disposed) return;
      final createdAt = max(
        DateTime.now().millisecondsSinceEpoch ~/ 1000,
        _lastCreatedAt + 1,
      );
      await signedEventRelay.submit(
        kind: EventKind.readState,
        content: content,
        tags: const [
          ['d', communityThemeDTag],
          ['t', communityThemeDTag],
        ],
        createdAt: createdAt,
      );
      if (_disposed) return;
      _lastCreatedAt = createdAt;
      _lastPublished = preference;
      if (_pending == preference) _pending = null;
    } catch (error) {
      debugPrint('[CommunityThemeSync] publish failed: $error');
    }
  }

  RemoteCommunityTheme? _decode(NostrEvent event) {
    if (event.pubkey != pubkey ||
        event.getTagValue('d') != communityThemeDTag) {
      return null;
    }
    try {
      final decoded = jsonDecode(crypto.decrypt(event.content));
      if (decoded is! Map<String, dynamic>) return null;
      return RemoteCommunityTheme(
        preference: CommunityThemePreference.fromJson(decoded),
        createdAt: event.createdAt,
        eventId: event.id,
      );
    } catch (_) {
      return null;
    }
  }

  void _accept(RemoteCommunityTheme remote) {
    if (remote.createdAt < _lastCreatedAt ||
        (remote.createdAt == _lastCreatedAt &&
            remote.eventId.compareTo(_lastEventId) <= 0)) {
      return;
    }
    _lastCreatedAt = remote.createdAt;
    _lastEventId = remote.eventId;
    cancelPending();
    onRemote(remote);
  }

  void dispose() {
    if (_disposed) return;
    _disposed = true;
    cancelPending();
    _unsubscribe?.call();
    _unsubscribe = null;
  }
}

NostrEvent _newerEvent(NostrEvent left, NostrEvent right) {
  if (right.createdAt != left.createdAt) {
    return right.createdAt > left.createdAt ? right : left;
  }
  return right.id.compareTo(left.id) > 0 ? right : left;
}
