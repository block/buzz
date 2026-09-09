part of 'agent_identity_provider.dart';

// Coordinates seed discovery only. Exact signed profiles and current policy
// are resolved by readAgentAuthorization before any candidate is exposed.
Future<Set<String>> _ownedAgentKeys(
  RelaySessionNotifier session,
  String viewer,
  bool Function() current,
) async {
  final keys = <String>{};
  NostrEvent? cursor;
  for (var page = 0; page < 200; page++) {
    if (!current()) throw StateError('Agent discovery scope changed');
    final events = await session.queryRelay([
      NostrFilter(
        kinds: const [30177],
        authors: [viewer],
        limit: 500,
        until: cursor?.createdAt,
        extensions: {if (cursor != null) 'before_id': cursor.id},
      ),
    ]);
    if (!current()) throw StateError('Agent discovery scope changed');
    for (final event in events) {
      final key = event.getTagValue('d');
      if (event.kind == 30177 &&
          event.pubkey == viewer &&
          verifySignedEvent(event) &&
          key != null &&
          RegExp(r'^[0-9a-f]{64}$').hasMatch(key) &&
          event.tags.where((tag) => tag.isNotEmpty && tag[0] == 'd').length ==
              1) {
        keys.add(key);
      }
    }
    if (keys.length > 1000) {
      throw StateError('Agent discovery exceeds key budget');
    }
    if (events.length < 500) return keys;
    final next = events.last;
    if (cursor != null &&
        (next.createdAt > cursor.createdAt ||
            (next.createdAt == cursor.createdAt &&
                next.id.compareTo(cursor.id) <= 0))) {
      throw StateError('Agent discovery pagination did not advance');
    }
    cursor = next;
  }
  throw StateError('Agent discovery exceeds page budget');
}

// Separate subscription owner: a refresh must not tear down/replay its trigger.
class _AgentDirectoryUpdates extends Notifier<int> {
  Object? failure;
  @override
  int build() {
    final sessionState = ref.watch(relaySessionProvider);
    ref.watch(myPubkeyProvider);
    ref.watch(relayConfigProvider);
    failure = null;
    var disposed = false;
    var attempts = 0;
    Timer? retry;
    void Function()? unsubscribe;
    Timer? debounce;
    ref.onDispose(() {
      disposed = true;
      unsubscribe?.call();
      debounce?.cancel();
      retry?.cancel();
    });
    void changed() {
      if (disposed || debounce != null) return;
      debounce = Timer(const Duration(milliseconds: 150), () {
        debounce = null;
        if (!disposed) state++;
      });
    }

    if (sessionState.status == SessionStatus.connected) {
      final session = ref.read(relaySessionProvider.notifier);
      Future<void> subscribe() async {
        if (disposed) return;
        attempts++;
        var active = true;
        bool current() => !disposed && active;
        void lost(Object error) {
          if (!current()) return;
          active = false;
          unsubscribe?.call();
          unsubscribe = null;
          failure = error;
          changed();
          // One budget for establishment and terminal closure per generation.
          // Exhaustion remains visible until session/account rebuild.
          if (attempts < 3) {
            retry = Timer(Duration(milliseconds: 250 * attempts), () {
              retry = null;
              unawaited(subscribe());
            });
          }
        }

        try {
          final close = await session.subscribeWithStatus(
            NostrFilter(
              kinds: const [0, 5, 10100, 30177, 39002],
              limit: 0,
              since: DateTime.now().millisecondsSinceEpoch ~/ 1000,
            ),
            (event) {
              if (!current()) return;
              if (disposed) return;
              if (event.kind == 0) {
                ref.read(userCacheProvider.notifier).cacheProfileEvent(event);
              }
              if (event.kind == 5 && !_isAgentCoordinateDeletion(event)) return;
              changed();
            },
            onClosed: (message) => lost(StateError(message)),
            onStatusChanged: (status) {
              if (!current()) return;
              if (status == RelaySubscriptionStatus.ready) failure = null;
              changed();
            },
          );
          if (!current()) {
            close();
          } else {
            unsubscribe = close;
          }
        } catch (error) {
          lost(error);
        }
      }

      unawaited(subscribe());
    }
    return 0;
  }
}

final _agentDirectoryUpdatesProvider =
    NotifierProvider<_AgentDirectoryUpdates, int>(_AgentDirectoryUpdates.new);

// Desktop build_agent_delete emits a kind:5 a-coordinate, not a 30177 event.
bool _isAgentCoordinateDeletion(NostrEvent event) =>
    verifySignedEvent(event) &&
    event.tags.any((tag) {
      if (tag.length < 2 || tag[0] != 'a') return false;
      final coordinate = tag[1].split(':');
      return coordinate.length == 3 &&
          coordinate[0] == '30177' &&
          coordinate[1] == event.pubkey &&
          RegExp(r'^[0-9a-f]{64}$').hasMatch(coordinate[2]);
    });
