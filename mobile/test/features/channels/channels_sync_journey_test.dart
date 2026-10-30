import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:fake_async/fake_async.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/fake_relay.dart';

const _channel = '11111111-1111-4111-8111-111111111111';

void main() {
  final binding = TestWidgetsFlutterBinding.ensureInitialized();

  test(
    'the channel list reconciles once per resume and stays quiet while away',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      fakeAsync((async) {
        var now = DateTime(2026, 10, 20, 12);
        final relay = FakeRelay(now: () => now);
        relay.seed([
          relay.sign(
            kind: 39002,
            tags: [
              ['d', _channel],
              ['p', relay.pubkey],
            ],
          ),
          relay.sign(
            kind: 39000,
            tags: [
              ['d', _channel],
              ['name', 'general'],
              ['t', 'stream'],
              ['public'],
            ],
          ),
        ]);
        final container = ProviderContainer(
          overrides: [
            ...relay.overrides,
            savedPrefsProvider.overrideWithValue(prefs),
          ],
        );
        addTearDown(container.dispose);
        final channels = container.listen(channelsProvider, (_, _) {});
        // A membership read is paginated; count each read's first page.
        int membershipReads() => relay.queries
            .expand((filters) => filters)
            .where(
              (f) =>
                  f.containsKey('#p') &&
                  f['kinds'].contains(39002) &&
                  f['until'] == null,
            )
            .length;
        void lifecycle(AppLifecycleState state) =>
            binding.handleAppLifecycleStateChanged(state);
        void settle() => async.elapse(const Duration(milliseconds: 50));

        settle();
        expect(channels.read().requireValue.single.name, 'general');
        expect(membershipReads(), 1);

        // Backgrounded just before a backstop tick, while the session is
        // still connected through its grace period: the 60s backstop sends
        // nothing until the app returns.
        async.elapse(const Duration(seconds: 58));
        expect(membershipReads(), 1);
        lifecycle(AppLifecycleState.inactive);
        lifecycle(AppLifecycleState.hidden);
        lifecycle(AppLifecycleState.paused);
        async.elapse(const Duration(minutes: 3));
        expect(
          container.read(relaySessionProvider).status,
          isNot(SessionStatus.connected),
        );
        expect(membershipReads(), 1);

        lifecycle(AppLifecycleState.hidden);
        lifecycle(AppLifecycleState.inactive);
        lifecycle(AppLifecycleState.resumed);
        settle();
        expect(
          container.read(relaySessionProvider).status,
          SessionStatus.connected,
        );
        expect(membershipReads(), 2);

        // The OS suspends the app, so the session's pause timer never fires
        // and it still believes it is connected when the app resumes. It
        // reconnects; the list reconciles once, after AUTH, not before too.
        lifecycle(AppLifecycleState.inactive);
        lifecycle(AppLifecycleState.hidden);
        lifecycle(AppLifecycleState.paused);
        now = now.add(const Duration(minutes: 10));
        final auth = relay.holdAuth();
        lifecycle(AppLifecycleState.hidden);
        lifecycle(AppLifecycleState.inactive);
        lifecycle(AppLifecycleState.resumed);
        settle();
        expect(membershipReads(), 2);
        auth.complete();
        settle();
        expect(
          container.read(relaySessionProvider).status,
          SessionStatus.connected,
        );
        expect(membershipReads(), 3);
        expect(relay.reqsBeforeAuth, isEmpty);
        expect(channels.read().requireValue.single.name, 'general');
      });
    },
  );
}
