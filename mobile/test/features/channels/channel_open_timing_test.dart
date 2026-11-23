import 'package:buzz/features/channels/channel_messages_provider.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:clock/clock.dart';
import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../helpers/fake_relay.dart';
import '../../helpers/recording_diagnostics.dart';

const _channel = '11111111-1111-4111-8111-111111111111';

({
  FakeRelay relay,
  RecordingDiagnostics counts,
  ProviderContainer container,
  Iterable<NetworkEvent> Function() opens,
})
_start(FakeAsync async) {
  final relay = FakeRelay();
  final counts = RecordingDiagnostics();
  final container = ProviderContainer(
    overrides: [
      ...relay.overrides,
      diagnosticReporterProvider.overrideWithValue(counts),
    ],
  );
  addTearDown(container.dispose);
  container.listen(relaySessionProvider, (_, _) {});
  async.elapse(const Duration(milliseconds: 50));
  counts.clear();
  return (
    relay: relay,
    counts: counts,
    container: container,
    opens: () => counts.events.where(
      (event) => event.endpoint == NetworkEndpoint.channelOpen,
    ),
  );
}

void main() {
  test('opening a channel is timed once, to its history being shown', () {
    fakeAsync((async) {
      final relay = FakeRelay();
      final counts = RecordingDiagnostics();
      final container = ProviderContainer(
        overrides: [
          ...relay.overrides,
          diagnosticReporterProvider.overrideWithValue(counts),
        ],
      );
      addTearDown(container.dispose);
      container.listen(relaySessionProvider, (_, _) {});
      async.elapse(const Duration(milliseconds: 50));
      counts.clear();
      Iterable<NetworkEvent> opens() => counts.events.where(
        (event) => event.endpoint == NetworkEndpoint.channelOpen,
      );

      final held = relay.holdQuery();
      final messages = container.listen(
        channelMessagesProvider(_channel),
        (_, _) {},
      );
      async.elapse(const Duration(seconds: 2));
      expect(opens(), isEmpty, reason: 'nothing is shown yet');
      held.complete();
      async.elapse(const Duration(milliseconds: 50));
      expect(messages.read().hasValue, isTrue);
      final open = opens().single;
      expect(open.backend, 'relay.test');
      expect(open.outcome, NetworkOutcome.success);
      expect(open.duration, greaterThanOrEqualTo(const Duration(seconds: 2)));

      // A catch-up after a reconnect is not an open.
      relay.drop();
      async.elapse(const Duration(seconds: 40));
      expect(opens(), hasLength(1));
    });
  });

  for (final correction in [
    const Duration(hours: 1),
    -const Duration(hours: 1),
  ]) {
    test('a device clock set ${correction.isNegative ? 'back' : 'forward'} '
        'during an open does not move its time', () {
      fakeAsync((async) {
        var shift = Duration.zero;
        final fake = async.getClock(DateTime.utc(2026));
        final wall = Clock(() => fake.now().add(shift));
        withClock(wall, () {
          // Fake time is the monotonic clock; only the wall clock moves.
          final relay = FakeRelay(elapsed: () => async.elapsed);
          final counts = RecordingDiagnostics();
          final container = ProviderContainer(
            overrides: [
              ...relay.overrides,
              diagnosticReporterProvider.overrideWithValue(counts),
            ],
          );
          addTearDown(container.dispose);
          container.listen(relaySessionProvider, (_, _) {});
          async.elapse(const Duration(milliseconds: 50));
          final held = relay.holdQuery();
          container.listen(channelMessagesProvider(_channel), (_, _) {});
          async.elapse(const Duration(seconds: 1));
          shift = correction;
          async.elapse(const Duration(seconds: 1));
          held.complete();
          async.elapse(const Duration(milliseconds: 50));
          final open = counts.events.singleWhere(
            (event) => event.endpoint == NetworkEndpoint.channelOpen,
          );
          expect(
            open.duration,
            greaterThanOrEqualTo(const Duration(seconds: 2)),
          );
          expect(open.duration, lessThan(const Duration(seconds: 3)));
        });
      });
    });
  }

  test('an open that spans a reconnect is timed from its start', () {
    fakeAsync((async) {
      final (:relay, :counts, :container, :opens) = _start(async);
      final held = relay.holdQuery();
      final messages = container.listen(
        channelMessagesProvider(_channel),
        (_, _) {},
      );
      async.elapse(const Duration(seconds: 3));
      final auth = relay.holdAuth();
      relay.drop();
      async.elapse(const Duration(seconds: 30));
      auth.complete();
      async.elapse(const Duration(milliseconds: 100));
      held.complete();
      async.elapse(const Duration(milliseconds: 50));
      expect(messages.read().hasValue, isTrue);
      final open = opens().single;
      expect(open.outcome, NetworkOutcome.success);
      expect(open.duration, greaterThanOrEqualTo(const Duration(seconds: 33)));
    });
  });

  test('an open is not counted for a member who signed in during it', () {
    fakeAsync((async) {
      final (:relay, :counts, :container, :opens) = _start(async);
      final held = relay.holdQuery();
      final messages = container.listen(
        channelMessagesProvider(_channel),
        (_, _) {},
      );
      async.elapse(const Duration(seconds: 1));
      counts.setMember('someone else');
      held.complete();
      async.elapse(const Duration(milliseconds: 50));
      expect(messages.read().hasValue, isTrue);
      expect(opens(), isEmpty);
    });
  });

  test('an open started after the reporter moved to another member '
      'is not counted for them', () {
    fakeAsync((async) {
      final (:relay, :counts, :container, :opens) = _start(async);
      // A switch names the next member before the old relay is replaced.
      counts.setMember('someone else');
      final messages = container.listen(
        channelMessagesProvider(_channel),
        (_, _) {},
      );
      async.elapse(const Duration(seconds: 1));
      expect(messages.read().hasValue, isTrue);
      expect(opens(), isEmpty);
    });
  });

  test('an open spanning a brief stay in the background is not counted', () {
    fakeAsync((async) {
      final (:relay, :counts, :container, :opens) = _start(async);
      final held = relay.holdQuery();
      final messages = container.listen(
        channelMessagesProvider(_channel),
        (_, _) {},
      );
      async.elapse(const Duration(seconds: 1));
      final session = container.read(relaySessionProvider.notifier);
      session.onAppPaused();
      async.elapse(const Duration(seconds: 2));
      session.onAppResumed();
      held.complete();
      async.elapse(const Duration(milliseconds: 50));
      expect(messages.read().hasValue, isTrue);
      expect(opens(), isEmpty, reason: 'it measured the time away');
    });
  });

  test('an open the member left still counts', () {
    fakeAsync((async) {
      final (:relay, :counts, :container, :opens) = _start(async);
      final held = relay.holdQuery();
      final page = container.listen(
        channelMessagesProvider(_channel),
        (_, _) {},
      );
      async.elapse(const Duration(seconds: 1));
      page.close();
      held.complete();
      async.elapse(const Duration(seconds: 1));
      expect(opens().single.outcome, NetworkOutcome.success);
    });
  });

  test('a return after leaving a cut-short open is a new open', () {
    fakeAsync((async) {
      final (:relay, :counts, :container, :opens) = _start(async);
      var held = relay.holdQuery();
      final page = container.listen(
        channelMessagesProvider(_channel),
        (_, _) {},
      );
      async.elapse(const Duration(seconds: 1));
      page.close();
      relay.drop();
      async.elapse(const Duration(minutes: 10));
      held.complete();
      held = relay.holdQuery();
      container.listen(channelMessagesProvider(_channel), (_, _) {});
      async.elapse(const Duration(seconds: 2));
      held.complete();
      async.elapse(const Duration(seconds: 1));
      final open = opens().single;
      expect(open.duration, lessThan(const Duration(minutes: 1)));
    });
  });

  test('an open cut short by the background is not carried past it', () {
    fakeAsync((async) {
      final (:relay, :counts, :container, :opens) = _start(async);
      final held = relay.holdQuery();
      container.listen(channelMessagesProvider(_channel), (_, _) {});
      async.elapse(const Duration(seconds: 1));
      final session = container.read(relaySessionProvider.notifier);
      session.onAppPaused();
      async.elapse(const Duration(minutes: 1));
      session.onAppResumed();
      async.elapse(const Duration(seconds: 1));
      held.complete();
      async.elapse(const Duration(seconds: 1));
      expect(
        opens().single.duration,
        lessThan(const Duration(minutes: 1)),
        reason: 'the open after the return is a new one',
      );
    });
  });

  test('a refusal on the live subscription counts as a failed open', () {
    fakeAsync((async) {
      final (:relay, :counts, :container, :opens) = _start(async);
      final held = relay.holdQuery();
      container.listen(channelMessagesProvider(_channel), (_, _) {});
      async.elapse(const Duration(seconds: 1));
      relay.closeLive(
        (filter) => filter.containsKey('#h'),
        'restricted: not a channel member',
      );
      async.elapse(const Duration(milliseconds: 50));
      held.complete();
      async.elapse(const Duration(milliseconds: 50));
      final open = opens().single;
      expect(open.outcome, NetworkOutcome.failure);
      expect(open.failure, NetworkFailure.relayRefused);
    });
  });

  test('a refusal before the history was asked for counts once', () {
    fakeAsync((async) {
      final (:relay, :counts, :container, :opens) = _start(async);
      relay.refuseReq(
        (filter) => filter.containsKey('#h'),
        'restricted: not a channel member',
      );
      container.listen(channelMessagesProvider(_channel), (_, _) {});
      async.elapse(const Duration(seconds: 1));
      final open = opens().single;
      expect(open.failure, NetworkFailure.relayRefused);
      expect(open.duration, isNull, reason: 'nothing timed it');
    });
  });

  for (final away in [false, true]) {
    test(
      'a refusal that arrives before the history asks '
      '${away ? 'after a move to the background is not counted' : 'in the foreground counts'}',
      () {
        fakeAsync((async) {
          final (:relay, :counts, :container, :opens) = _start(async);
          final req = relay.holdReq();
          container.listen(channelMessagesProvider(_channel), (_, _) {});
          async.elapse(const Duration(milliseconds: 50));
          expect(relay.reqs, isNotEmpty);
          if (away) {
            container.read(relaySessionProvider.notifier).onAppPaused();
          }
          relay.refuseReq(
            (filter) => filter.containsKey('#h'),
            'restricted: not a channel member',
          );
          req.complete();
          async.elapse(const Duration(seconds: 1));
          expect(relay.queries, isEmpty, reason: 'refused before the history');
          if (away) {
            expect(
              opens(),
              isEmpty,
              reason: 'it spanned a move to the background',
            );
          } else {
            expect(opens().single.failure, NetworkFailure.relayRefused);
          }
        });
      },
    );
  }

  for (final asked in [false, true]) {
    for (final away in [false, true]) {
      test(
        'a thread refusal ${asked ? 'during' : 'before'} the first history '
        '${away ? 'after a move to the background is not counted' : 'counts one refused open'}',
        () {
          fakeAsync((async) {
            final (:relay, :counts, :container, :opens) = _start(async);
            final req = asked ? null : relay.holdReq();
            final held = asked ? relay.holdQuery() : null;
            container.listen(channelMessagesProvider(_channel), (_, _) {});
            async.elapse(const Duration(milliseconds: 50));
            expect(relay.queries, asked ? isNotEmpty : isEmpty);
            final session = container.read(relaySessionProvider.notifier);
            if (away) {
              session.onAppPaused();
              async.elapse(const Duration(seconds: 1));
              session.onAppResumed();
            }
            container
                .read(channelMessagesProvider(_channel).notifier)
                .denyAccessFromThread(
                  Exception('restricted: not a channel member'),
                  StackTrace.current,
                );
            req?.complete();
            held?.complete();
            async.elapse(const Duration(seconds: 1));
            if (away) {
              expect(opens(), isEmpty, reason: 'it spanned the background');
            } else {
              final open = opens().single;
              expect(open.failure, NetworkFailure.relayRefused);
              expect(open.duration, asked ? isNotNull : isNull);
            }
          });
        },
      );
    }
  }

  test('a thread refusal after the channel loaded is not another open', () {
    fakeAsync((async) {
      final (:relay, :counts, :container, :opens) = _start(async);
      container.listen(channelMessagesProvider(_channel), (_, _) {});
      async.elapse(const Duration(seconds: 1));
      expect(opens().single.outcome, NetworkOutcome.success);
      container
          .read(channelMessagesProvider(_channel).notifier)
          .denyAccessFromThread(
            Exception('restricted: not a channel member'),
            StackTrace.current,
          );
      async.elapse(const Duration(seconds: 1));
      expect(opens(), hasLength(1));
    });
  });

  test('a relay that runs out of time on the history fails the open', () {
    fakeAsync((async) {
      final (:relay, :counts, :container, :opens) = _start(async);
      relay.failQuery((_) => true, body: '{"error":"query timed out"}');
      container.listen(channelMessagesProvider(_channel), (_, _) {});
      async.elapse(const Duration(seconds: 1));
      final open = opens().first;
      expect(open.outcome, NetworkOutcome.failure);
      expect(open.failure, NetworkFailure.timeout);
    });
  });

  test('an open started in the background is not counted', () {
    fakeAsync((async) {
      final (:relay, :counts, :container, :opens) = _start(async);
      final session = container.read(relaySessionProvider.notifier);
      session.onAppPaused();
      final held = relay.holdQuery();
      container.listen(channelMessagesProvider(_channel), (_, _) {});
      async.elapse(const Duration(seconds: 2));
      session.onAppResumed();
      held.complete();
      async.elapse(const Duration(milliseconds: 50));
      expect(opens(), isEmpty, reason: 'it measured the time away');
    });
  });

  test('a return after leaving before the history, once live detached, '
      'is a new open', () {
    fakeAsync((async) {
      final (:relay, :counts, :container, :opens) = _start(async);
      final req = relay.holdReq();
      final held = relay.holdQuery();
      final page = container.listen(
        channelMessagesProvider(_channel),
        (_, _) {},
      );
      async.elapse(const Duration(milliseconds: 10));
      page.close();
      req.complete();
      async.elapse(const Duration(milliseconds: 50));
      expect(relay.queries, isNotEmpty);
      relay.drop();
      async.elapse(const Duration(minutes: 10));
      held.complete();
      container.listen(channelMessagesProvider(_channel), (_, _) {});
      async.elapse(const Duration(seconds: 1));
      expect(opens().single.duration, lessThan(const Duration(seconds: 1)));
    });
  });

  test('a refused retry after a failed open counts as a new open', () {
    fakeAsync((async) {
      final (:relay, :counts, :container, :opens) = _start(async);
      relay.failQuery((_) => true, body: '{"error":"query timed out"}');
      final page = container.listen(
        channelMessagesProvider(_channel),
        (_, _) {},
      );
      async.elapse(const Duration(seconds: 1));
      expect(opens().single.failure, NetworkFailure.timeout);
      counts.clear();
      relay.refuseReq((_) => true, 'restricted: not a channel member');
      container
          .read(channelMessagesProvider(_channel).notifier)
          .retryHistoryRefresh();
      async.elapse(const Duration(seconds: 1));
      expect(page.read().hasError, isTrue);
      final open = opens().single;
      expect(open.failure, NetworkFailure.relayRefused);
      expect(open.duration, isNull, reason: 'nothing timed it');
    });
  });

  test('a refusal after the member backed out ends the open with its time', () {
    fakeAsync((async) {
      final (:relay, :counts, :container, :opens) = _start(async);
      final held = relay.holdQuery();
      final page = container.listen(
        channelMessagesProvider(_channel),
        (_, _) {},
      );
      async.elapse(const Duration(seconds: 2));
      expect(relay.queries, isNotEmpty);
      page.close();
      relay.closeLive(
        (filter) => filter.containsKey('#h'),
        'restricted: not a channel member',
      );
      async.elapse(const Duration(milliseconds: 50));
      held.complete();
      async.elapse(const Duration(milliseconds: 50));
      final open = opens().single;
      expect(open.failure, NetworkFailure.relayRefused);
      expect(open.duration, greaterThanOrEqualTo(const Duration(seconds: 2)));
    });
  });

  for (final away in [false, true]) {
    test('a refusal on reconnect before the history ends the carried open'
        '${away ? ', untimed after a stay in the background' : ''}', () {
      fakeAsync((async) {
        final (:relay, :counts, :container, :opens) = _start(async);
        relay.holdQuery();
        container.listen(channelMessagesProvider(_channel), (_, _) {});
        async.elapse(const Duration(seconds: 2));
        final auth = relay.holdAuth();
        relay.drop();
        relay.refuseReq(
          (filter) => filter.containsKey('#h'),
          'restricted: not a channel member',
        );
        if (away) {
          final session = container.read(relaySessionProvider.notifier);
          session.onAppPaused();
          async.elapse(const Duration(seconds: 1));
          session.onAppResumed();
        }
        async.elapse(const Duration(seconds: 20));
        auth.complete();
        async.elapse(const Duration(milliseconds: 100));
        final open = opens().single;
        expect(open.failure, NetworkFailure.relayRefused);
        if (away) {
          expect(open.duration, isNull, reason: 'it measured the time away');
        } else {
          expect(
            open.duration,
            greaterThanOrEqualTo(const Duration(seconds: 22)),
          );
        }
      });
    });
  }
}
