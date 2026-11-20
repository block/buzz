import 'dart:async';

import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';

/// One live subscription per visible parent, with independent discovery and
/// participation history budgets. Live-only events survive history recovery.
final huddleLifecycleProvider = StreamProvider.autoDispose
    .family<List<NostrEvent>, String>((ref, channelId) {
      ref.watch(relayConfigProvider);
      ref.watch(myPubkeyProvider);
      final status = ref.watch(relaySessionProvider.select((s) => s.status));
      if (status != SessionStatus.connected) return Stream.value(const []);
      final session = ref.read(relaySessionProvider.notifier);
      final stream = StreamController<List<NostrEvent>>();
      final events = <String, NostrEvent>{};
      var disposed = false;
      var reading = false;
      var readAgain = false;
      void Function()? stop;
      ref.onDispose(() {
        disposed = true;
        stop?.call();
        unawaited(stream.close());
      });
      void merge(Iterable<NostrEvent> incoming) {
        if (disposed) return;
        for (final event in incoming) {
          events[event.id] = event;
        }
        // Participation traffic has its own bound and cannot evict starts.
        for (final participation in [false, true]) {
          final retained =
              events.values
                  .where(
                    (event) =>
                        (event.kind == 48101 || event.kind == 48102) ==
                        participation,
                  )
                  .toList()
                ..sort((a, b) {
                  final time = b.createdAt.compareTo(a.createdAt);
                  return time != 0 ? time : b.id.compareTo(a.id);
                });
          final limit = participation ? 2000 : 200;
          for (final event in retained.skip(limit)) {
            events.remove(event.id);
          }
        }
        stream.add(List.unmodifiable(events.values));
      }

      Future<void> reconcile() async {
        if (disposed) return;
        if (reading) {
          readAgain = true;
          return;
        }
        reading = true;
        do {
          readAgain = false;
          // Each read stands on its own: participation history only enriches
          // the roster, so its failure must not hide an active start.
          Object? discoveryError;
          StackTrace? discoveryStack;
          final history = await Future.wait([
            session
                .fetchHistory(NostrFilters.huddleLifecycle(channelId))
                .then<List<NostrEvent>?>(
                  (events) => events,
                  onError: (Object error, StackTrace stack) {
                    discoveryError = error;
                    discoveryStack = stack;
                    return null;
                  },
                ),
            session
                .fetchHistory(
                  NostrFilter(
                    kinds: const [48101, 48102],
                    tags: {
                      '#h': [channelId],
                    },
                    limit: 200,
                  ),
                )
                .then<List<NostrEvent>?>(
                  (events) => events,
                  onError: (Object _, StackTrace _) => null,
                ),
          ]);
          final succeeded = history.nonNulls;
          if (succeeded.isNotEmpty) merge(succeeded.expand((batch) => batch));
          final error = discoveryError;
          if (error != null && !disposed && events.isEmpty) {
            stream.addError(error, discoveryStack);
          }
        } while (!disposed && readAgain);
        reading = false;
      }

      unawaited(() async {
        try {
          var readySeen = false;
          final unsubscribe = await session.subscribeWithStatus(
            NostrFilter(
              kinds: const [48100, 48101, 48102, 48103],
              tags: {
                '#h': [channelId],
              },
              limit: 0,
            ),
            (event) => merge([event]),
            resumeSince: false,
            onStatusChanged: (status) {
              if (status == RelaySubscriptionStatus.ready) {
                readySeen = true;
                unawaited(reconcile());
              }
            },
            onClosed: (message) {
              if (!disposed) stream.addError(Exception(message));
            },
          );
          if (disposed) {
            unsubscribe();
            return;
          }
          stop = unsubscribe;
          if (!readySeen) await reconcile();
        } catch (error, stack) {
          if (!disposed) stream.addError(error, stack);
        }
      }());
      return stream.stream;
    });
