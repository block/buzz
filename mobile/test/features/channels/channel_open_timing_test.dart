import 'package:buzz/features/channels/channel_messages_provider.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../helpers/fake_relay.dart';
import '../../helpers/recording_diagnostics.dart';

const _channel = '11111111-1111-4111-8111-111111111111';

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
}
