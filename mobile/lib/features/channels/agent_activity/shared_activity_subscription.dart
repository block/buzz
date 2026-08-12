import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../../shared/relay/relay.dart';
import 'shared_activity_models.dart';

const _sharedActivityFreshness = Duration(minutes: 5);

typedef SharedActivityKey = ({String channelId, String agentPubkey});

enum SharedActivityConnectionState { connecting, live, closed, error }

@immutable
class SharedActivityState {
  final SharedActivityConnectionState connection;
  final List<SharedActivity> activities;
  final String? errorMessage;

  const SharedActivityState({
    required this.connection,
    required this.activities,
    this.errorMessage,
  });
}

final sharedActivityNowProvider = Provider<DateTime Function()>(
  (ref) => DateTime.now,
);

/// A live-only, channel-and-agent-scoped safe activity subscription.
class SharedActivitySubscriptionNotifier extends Notifier<SharedActivityState> {
  final SharedActivityKey key;
  final SharedActivityStore _store = SharedActivityStore();

  SharedActivitySubscriptionNotifier(this.key);

  void Function()? _unsubscribe;
  Future<void>? _startFuture;
  var _connection = SharedActivityConnectionState.connecting;
  String? _errorMessage;
  var _subscriptionEpoch = 0;
  var _disposed = false;
  var _disposeRegistered = false;

  @override
  SharedActivityState build() {
    final sessionState = ref.watch(relaySessionProvider);
    _disposed = false;

    if (!_disposeRegistered) {
      _disposeRegistered = true;
      ref.onDispose(_dispose);
    }

    if (!_hasCanonicalKey) {
      _connection = SharedActivityConnectionState.error;
      _errorMessage = 'Invalid shared activity channel or agent identity';
      return _snapshot();
    }

    if (sessionState.status == SessionStatus.connected) {
      // A relay CLOSED is terminal for this authorization-scoped stream.
      // Do not silently resubscribe after membership has been rejected.
      if (_connection == SharedActivityConnectionState.closed) {
        return _snapshot();
      }
      if (_unsubscribe == null && _startFuture == null) {
        _connection = SharedActivityConnectionState.connecting;
        Future.microtask(_ensureSubscribed);
      } else if (_unsubscribe != null) {
        _connection = SharedActivityConnectionState.live;
        _errorMessage = null;
      }
    } else if (_connection != SharedActivityConnectionState.closed &&
        _connection != SharedActivityConnectionState.error) {
      _connection = SharedActivityConnectionState.connecting;
    }

    return _snapshot();
  }

  bool get _hasCanonicalKey =>
      _canonicalUuidPattern.hasMatch(key.channelId) &&
      _lowercasePubkeyPattern.hasMatch(key.agentPubkey);

  Future<void> _ensureSubscribed() {
    if (_disposed || _unsubscribe != null) return Future.value();
    final pending = _startFuture;
    if (pending != null) return pending;

    final epoch = _subscriptionEpoch;
    final future = _subscribe(epoch);
    _startFuture = future;
    return future;
  }

  Future<void> _subscribe(int epoch) async {
    try {
      if (_disposed || epoch != _subscriptionEpoch) return;
      _emit(
        connection: SharedActivityConnectionState.connecting,
        errorMessage: null,
      );

      final unsubscribe = await ref
          .read(relaySessionProvider.notifier)
          .subscribeValidatedLiveOnly(
            NostrFilter(
              kinds: const [EventKind.agentActivitySummary],
              authors: [key.agentPubkey],
              tags: {
                '#h': [key.channelId],
              },
              limit: 0,
            ),
            (event) => _verifiedActivities(event) != null,
            _handleVerifiedEvent,
            onClosed: (message) => _handleClosed(epoch, message),
          );

      if (_disposed || epoch != _subscriptionEpoch) {
        unsubscribe();
        return;
      }

      _unsubscribe = unsubscribe;
      _emit(connection: SharedActivityConnectionState.live, errorMessage: null);
    } catch (error) {
      if (_disposed || epoch != _subscriptionEpoch) return;
      _unsubscribe = null;
      _emit(
        connection: SharedActivityConnectionState.error,
        errorMessage: 'Shared activity subscription failed: $error',
      );
    } finally {
      if (epoch == _subscriptionEpoch) _startFuture = null;
    }
  }

  void _handleVerifiedEvent(NostrEvent event) {
    final activities = _verifiedActivities(event);
    if (activities == null) return;
    _store.addAll(activities);
    _emit(connection: SharedActivityConnectionState.live, errorMessage: null);
  }

  List<SharedActivity>? _verifiedActivities(NostrEvent event) {
    if (!_hasValidSignatureAndId(event) ||
        event.kind != EventKind.agentActivitySummary ||
        event.pubkey != key.agentPubkey ||
        !_isFresh(event.createdAt) ||
        !_hasExactTags(event)) {
      return null;
    }

    try {
      return parseSharedActivityFrame(event.content);
    } on FormatException {
      return null;
    }
  }

  bool _isFresh(int createdAt) {
    final eventTime = DateTime.fromMillisecondsSinceEpoch(
      createdAt * 1000,
      isUtc: true,
    );
    final difference = ref
        .read(sharedActivityNowProvider)()
        .toUtc()
        .difference(eventTime);
    return difference.abs() <= _sharedActivityFreshness;
  }

  bool _hasExactTags(NostrEvent event) {
    if (event.tags.length != 2) return false;
    final channelTag = event.tags[0];
    final agentTag = event.tags[1];
    return channelTag.length == 2 &&
        channelTag[0] == 'h' &&
        channelTag[1] == key.channelId &&
        _canonicalUuidPattern.hasMatch(channelTag[1]) &&
        agentTag.length == 2 &&
        agentTag[0] == 'agent' &&
        agentTag[1] == event.pubkey.toLowerCase() &&
        _lowercasePubkeyPattern.hasMatch(agentTag[1]);
  }

  static bool _hasValidSignatureAndId(NostrEvent event) {
    try {
      nostr.Event.fromMap(event.toJson(), verify: true);
      return true;
    } catch (_) {
      return false;
    }
  }

  void _handleClosed(int epoch, String message) {
    if (_disposed || epoch != _subscriptionEpoch) return;
    _unsubscribe = null;
    _store.clear();
    _emit(
      connection: SharedActivityConnectionState.closed,
      errorMessage: 'Shared activity subscription closed: $message',
    );
  }

  void _emit({
    required SharedActivityConnectionState connection,
    required String? errorMessage,
  }) {
    if (_disposed) return;
    _connection = connection;
    _errorMessage = errorMessage;
    state = _snapshot();
  }

  SharedActivityState _snapshot() => SharedActivityState(
    connection: _connection,
    activities: _store.items,
    errorMessage: _errorMessage,
  );

  void _dispose() {
    _disposed = true;
    _subscriptionEpoch += 1;
    _unsubscribe?.call();
    _unsubscribe = null;
    _startFuture = null;
    _store.clear();
  }
}

final sharedActivitySubscriptionProvider = NotifierProvider.autoDispose
    .family<
      SharedActivitySubscriptionNotifier,
      SharedActivityState,
      SharedActivityKey
    >(SharedActivitySubscriptionNotifier.new);

final _canonicalUuidPattern = RegExp(
  r'^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$',
);
final _lowercasePubkeyPattern = RegExp(r'^[0-9a-f]{64}$');
