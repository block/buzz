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
  final Duration publishRetryBase;
  final Duration publishRetryMax;
  final Duration subscriptionRetryBase;
  final void Function(RemoteCommunityTheme) onRemote;
  final void Function(CommunityThemePreference) onPublished;

  Timer? _publishTimer;
  Timer? _subscriptionRetryTimer;
  void Function()? _unsubscribe;
  CommunityThemePreference? _pending;
  CommunityThemePreference? _lastPublished;
  int _lastCreatedAt = 0;
  String _lastEventId = '';
  RemoteCommunityTheme? _lastRemote;
  int _subscriptionEpoch = 0;
  int _subscriptionRetryAttempt = 0;
  int _publishRetryAttempt = 0;
  bool _publishInFlight = false;
  bool _publishRequestedWhileInFlight = false;
  bool _disposed = false;

  CommunityThemeSyncManager({
    required this.pubkey,
    required this.relaySession,
    required this.signedEventRelay,
    required this.crypto,
    required this.onRemote,
    this.onPublished = _ignorePublished,
    this.debounce = const Duration(seconds: 2),
    this.publishRetryBase = const Duration(seconds: 1),
    this.publishRetryMax = const Duration(seconds: 30),
    this.subscriptionRetryBase = const Duration(seconds: 1),
  });

  CommunityThemePreference? get pending => _pending;

  Future<CommunityThemeRemoteResult> fetchRemote() async {
    try {
      final events = await relaySession.fetchHistory(_themeFilter(limit: 1));
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

  Future<CommunityThemeRemoteResult> initialize() async {
    final subscribed = await _startLiveSubscription();
    if (_disposed) {
      return const CommunityThemeRemoteResult(
        CommunityThemeRemoteStatus.unavailable,
      );
    }
    final result = await fetchRemote();
    if (_disposed) return result;
    if (result.status == CommunityThemeRemoteStatus.valid) {
      _accept(result.remote!);
    }
    final remote = _lastRemote;
    if (remote != null) {
      return CommunityThemeRemoteResult(
        CommunityThemeRemoteStatus.valid,
        remote,
      );
    }
    if (!subscribed && result.status == CommunityThemeRemoteStatus.absent) {
      return const CommunityThemeRemoteResult(
        CommunityThemeRemoteStatus.unavailable,
      );
    }
    return result;
  }

  Future<bool> _startLiveSubscription() async {
    if (_disposed) return false;
    final epoch = ++_subscriptionEpoch;
    try {
      final unsubscribe = await relaySession.subscribe(
        _themeFilter(limit: 0),
        (event) {
          if (_disposed || epoch != _subscriptionEpoch) return;
          final remote = _decode(event);
          if (remote != null) _accept(remote);
        },
        onClosed: (message) => _handleSubscriptionClosed(epoch, message),
      );
      if (_disposed || epoch != _subscriptionEpoch) {
        unsubscribe();
        return false;
      }
      _unsubscribe = unsubscribe;
      _subscriptionRetryAttempt = 0;
      return true;
    } catch (error) {
      if (!_disposed && epoch == _subscriptionEpoch) {
        debugPrint('[CommunityThemeSync] live subscription failed: $error');
        _scheduleSubscriptionRetry();
      }
      return false;
    }
  }

  void _handleSubscriptionClosed(int epoch, String message) {
    if (_disposed || epoch != _subscriptionEpoch) return;
    debugPrint('[CommunityThemeSync] live subscription closed: $message');
    _unsubscribe = null;
    _scheduleSubscriptionRetry();
  }

  void _scheduleSubscriptionRetry() {
    if (_disposed || _subscriptionRetryTimer != null) return;
    final multiplier = 1 << min(_subscriptionRetryAttempt, 5);
    _subscriptionRetryAttempt++;
    _subscriptionRetryTimer = Timer(subscriptionRetryBase * multiplier, () {
      _subscriptionRetryTimer = null;
      unawaited(_recoverLiveSubscription());
    });
  }

  Future<void> _recoverLiveSubscription() async {
    if (_disposed) return;
    if (!await _startLiveSubscription()) return;

    // A relay CLOSED removes the retained subscription from RelaySession, so
    // reconnect replay cannot recover it. Query the replacement coordinate
    // after re-subscribing to close the gap while this stream was silent.
    final result = await fetchRemote();
    if (_disposed) return;
    if (result.status == CommunityThemeRemoteStatus.valid) {
      _accept(result.remote!);
    }
  }

  NostrFilter _themeFilter({required int limit}) => NostrFilter(
    kinds: const [EventKind.readState],
    authors: [pubkey],
    tags: const {
      '#d': [communityThemeDTag],
    },
    limit: limit,
  );

  void stage(CommunityThemePreference preference) {
    if (_disposed) return;
    _pending = preference;
    _publishRetryAttempt = 0;
    _publishTimer?.cancel();
    _publishTimer = null;
  }

  void publish(CommunityThemePreference preference) {
    stage(preference);
    publishStaged(preference);
  }

  void publishStaged(CommunityThemePreference preference) {
    if (_disposed || _pending != preference) return;
    _schedulePublish(debounce);
  }

  void _schedulePublish(Duration delay) {
    if (_disposed) return;
    _publishTimer?.cancel();
    _publishTimer = Timer(delay, () {
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
    if (_publishInFlight) {
      _publishTimer?.cancel();
      _publishTimer = null;
      _publishRequestedWhileInFlight = true;
      return;
    }
    final preference = _pending;
    if (_disposed || preference == null) return;
    if (preference == _lastPublished) {
      _pending = null;
      onPublished(preference);
      return;
    }
    _publishInFlight = true;
    try {
      final content = crypto.encrypt(jsonEncode(preference.toJson()));
      if (_disposed) return;
      final createdAt = max(
        DateTime.now().millisecondsSinceEpoch ~/ 1000,
        _lastCreatedAt + 1,
      );
      NostrEvent? signed;
      await signedEventRelay.submit(
        kind: EventKind.readState,
        content: content,
        tags: const [
          ['d', communityThemeDTag],
          ['t', communityThemeDTag],
        ],
        createdAt: createdAt,
        onSigned: (event) => signed = event,
      );
      if (_disposed) return;
      final published = signed;
      if (published == null) {
        throw StateError('Signed event coordinate unavailable');
      }
      final publishedCoordinateIsStale =
          _lastCreatedAt > published.createdAt ||
          (_lastCreatedAt == published.createdAt &&
              _lastEventId.isNotEmpty &&
              _lastEventId.compareTo(published.id) < 0);
      if (publishedCoordinateIsStale) {
        _lastPublished = null;
        _publishRetryAttempt = 0;
        if (_pending == preference) _schedulePublish(Duration.zero);
        return;
      }
      _lastCreatedAt = published.createdAt;
      _lastEventId = published.id;
      _lastPublished = preference;
      _publishRetryAttempt = 0;
      if (_pending == preference) _pending = null;
      onPublished(preference);
    } catch (error) {
      debugPrint('[CommunityThemeSync] publish failed: $error');
      if (_disposed || _pending != preference) return;
      final multiplier = 1 << min(_publishRetryAttempt, 30);
      _publishRetryAttempt++;
      final retryMs = min(
        publishRetryBase.inMilliseconds * multiplier,
        publishRetryMax.inMilliseconds,
      );
      _schedulePublish(Duration(milliseconds: retryMs));
    } finally {
      _publishInFlight = false;
      if (!_disposed &&
          _pending != null &&
          (_publishRequestedWhileInFlight || _pending != preference) &&
          _publishTimer == null) {
        _publishRequestedWhileInFlight = false;
        _schedulePublish(Duration.zero);
      } else {
        _publishRequestedWhileInFlight = false;
      }
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
            _lastEventId.isNotEmpty &&
            remote.eventId.compareTo(_lastEventId) >= 0)) {
      return;
    }
    _lastCreatedAt = remote.createdAt;
    _lastEventId = remote.eventId;
    _lastRemote = remote;
    if (_pending != null) {
      _lastPublished = null;
      return;
    }
    _lastPublished = null;
    onRemote(remote);
  }

  void dispose() {
    if (_disposed) return;
    _disposed = true;
    _subscriptionEpoch++;
    _subscriptionRetryTimer?.cancel();
    _subscriptionRetryTimer = null;
    cancelPending();
    _unsubscribe?.call();
    _unsubscribe = null;
  }
}

void _ignorePublished(CommunityThemePreference _) {}

NostrEvent _newerEvent(NostrEvent left, NostrEvent right) {
  if (right.createdAt != left.createdAt) {
    return right.createdAt > left.createdAt ? right : left;
  }
  return right.id.compareTo(left.id) < 0 ? right : left;
}
