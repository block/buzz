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
  Timer? _hydrationRecoveryTimer;
  void Function()? _unsubscribe;
  CommunityThemePreference? _pending;
  CommunityThemePreference? _lastPublished;
  int _lastCreatedAt = 0;
  String _lastEventId = '';
  RemoteCommunityTheme? _lastRemote;
  int _subscriptionEpoch = 0;
  int _subscriptionRetryAttempt = 0;
  int _hydrationRecoveryAttempt = 0;
  int _publishRetryAttempt = 0;
  bool _publishInFlight = false;
  bool _publishRequestedWhileInFlight = false;
  int? _activePublishCreatedAt;
  String? _activePublishEventId;
  int? _activePublishRemoteRevision;
  int _remoteRevision = 0;
  bool _hydrationObserved = false;
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
    if (result.status == CommunityThemeRemoteStatus.absent) {
      // A confirmed absence means the coordinate has been observed: there is no
      // desktop record to preserve, so a gated incomplete edit may publish.
      _observeHydration();
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

  /// Retry the history query while an edit is held by the pre-hydration gate
  /// and the coordinate is still unobserved. Without this, an initial
  /// `fetchRemote` that returns `unavailable` (a transient history failure)
  /// while the live subscription stays quiet would strand the edit forever:
  /// neither `initialize`'s absence path nor `_accept` ever runs, so
  /// `_hydrationObserved` never flips and every later [flush] returns early.
  void _scheduleHydrationRecovery() {
    if (_disposed ||
        _hydrationObserved ||
        _hydrationRecoveryTimer != null ||
        _pending == null) {
      return;
    }
    final multiplier = 1 << min(_hydrationRecoveryAttempt, 5);
    _hydrationRecoveryAttempt++;
    _hydrationRecoveryTimer = Timer(subscriptionRetryBase * multiplier, () {
      _hydrationRecoveryTimer = null;
      unawaited(_recoverHydration());
    });
  }

  Future<void> _recoverHydration() async {
    if (_disposed || _hydrationObserved || _pending == null) return;
    final result = await fetchRemote();
    if (_disposed || _hydrationObserved) return;
    if (result.status == CommunityThemeRemoteStatus.valid) {
      _accept(result.remote!);
    } else if (result.status == CommunityThemeRemoteStatus.absent) {
      _observeHydration();
    } else {
      // Still invalid or unavailable: keep retrying with backoff until the
      // coordinate is observed or the edit is superseded/cancelled.
      _scheduleHydrationRecovery();
    }
  }

  void _cancelHydrationRecovery() {
    _hydrationRecoveryTimer?.cancel();
    _hydrationRecoveryTimer = null;
    _hydrationRecoveryAttempt = 0;
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
    } else if (result.status == CommunityThemeRemoteStatus.absent) {
      // A confirmed absence here observes the coordinate too, releasing a gated
      // incomplete edit if the initial fetch had been unavailable.
      _observeHydration();
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

  /// Record that the relay coordinate has been observed at least once (a valid
  /// record or a confirmed absence) and release an edit held by the
  /// pre-hydration gate in [flush]. Mobile authors no desktop-only fields, so
  /// every edit is gated until the coordinate is known and cannot be merged
  /// before then.
  void _observeHydration() {
    if (_hydrationObserved) return;
    _hydrationObserved = true;
    _cancelHydrationRecovery();
    final pending = _pending;
    if (!_disposed && pending != null && !_publishInFlight) {
      // Accelerate the held edit now that the coordinate is known. Any pending
      // debounce timer is cancelled by _schedulePublish. A publish in flight is
      // left to its own finally-block, which reschedules the pending edit.
      _schedulePublish(Duration.zero);
    }
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
    _cancelHydrationRecovery();
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
    // Hold every edit until the relay coordinate has been observed, so a
    // pre-hydration edit cannot replace a desktop-authored record before we
    // have seen it. Mobile never authors the desktop-only fields, so even an
    // edit that carries them (inherited from a stale full cache) must wait:
    // without an observed coordinate there is nothing to merge the current
    // desktop values from. _observeHydration releases it.
    if (!_hydrationObserved) {
      _scheduleHydrationRecovery();
      return;
    }
    if (preference == _lastPublished) {
      _pending = null;
      onPublished(preference);
      return;
    }
    _publishInFlight = true;
    try {
      // Preserve the desktop-only fields the relay already holds when the local
      // edit omits them, so publishing the replaceable coordinate does not strip
      // a desktop client's glass and prominent-tab choices. Bookkeeping stays on
      // the local `preference` so the outbox ack contract is unaffected.
      final remote = _lastRemote?.preference;
      final remoteRevision = _remoteRevision;
      final outgoing = remote == null
          ? preference
          : preference.mergeDesktopAppearanceFrom(remote);
      final content = crypto.encrypt(jsonEncode(outgoing.toJson()));
      if (_disposed) return;
      final createdAt = max(
        DateTime.now().millisecondsSinceEpoch ~/ 1000,
        _lastCreatedAt + 1,
      );
      NostrEvent? signed;
      _activePublishRemoteRevision = remoteRevision;
      await signedEventRelay.submit(
        kind: EventKind.readState,
        content: content,
        tags: const [
          ['d', communityThemeDTag],
          ['t', communityThemeDTag],
        ],
        createdAt: createdAt,
        onSigned: (event) {
          signed = event;
          _activePublishCreatedAt = event.createdAt;
          _activePublishEventId = event.id;
        },
      );
      if (_disposed) return;
      final published = signed;
      if (published == null) {
        throw StateError('Signed event coordinate unavailable');
      }
      if (_remoteRevision != remoteRevision) {
        _lastPublished = null;
        _publishRetryAttempt = 0;
        if (_pending == preference) _schedulePublish(Duration.zero);
        return;
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
      _activePublishCreatedAt = null;
      _activePublishEventId = null;
      _activePublishRemoteRevision = null;
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
    final isActivePublishEcho =
        _publishInFlight &&
        remote.createdAt == _activePublishCreatedAt &&
        remote.eventId == _activePublishEventId;
    final competingRemoteSeen =
        _activePublishRemoteRevision != null &&
        _remoteRevision != _activePublishRemoteRevision;
    _lastCreatedAt = remote.createdAt;
    _lastEventId = remote.eventId;
    if (!isActivePublishEcho || !competingRemoteSeen) {
      _lastRemote = remote;
    }
    if (!isActivePublishEcho) {
      _remoteRevision++;
    }
    _observeHydration();
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
