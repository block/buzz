import 'package:buzz/features/channels/channel_messages_provider.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/relay/relay.dart';
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

  test('an open nobody waited out is not counted', () {
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
      expect(opens(), isEmpty);
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
}
