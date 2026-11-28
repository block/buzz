import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/relay/app_lifecycle_provider.dart';
import 'package:buzz/shared/relay/relay_session.dart';

void main() {
  testWidgets('a network change is checked in the foreground, not after it', (
    tester,
  ) async {
    const connectivity = EventChannel(
      'dev.fluttercommunity.plus/connectivity_status',
    );
    final messenger = tester.binding.defaultBinaryMessenger;
    late MockStreamHandlerEventSink events;
    messenger.setMockStreamHandler(
      connectivity,
      MockStreamHandler.inline(
        onListen: (_, sink) {
          events = sink;
        },
      ),
    );
    addTearDown(() => messenger.setMockStreamHandler(connectivity, null));
    final session = _CountingSession();
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => session)],
    );
    addTearDown(container.dispose);
    container.read(appLifecycleProvider);
    await tester.pump();

    // Alternate networks: the plugin drops a repeat of the same result.
    var network = 'wifi';
    Future<void> networkChange(AppLifecycleState lifecycle) async {
      tester.binding.handleAppLifecycleStateChanged(lifecycle);
      network = network == 'wifi' ? 'mobile' : 'wifi';
      events.success([network]);
      await tester.pump();
    }

    await networkChange(AppLifecycleState.resumed);
    expect(session.networkChanges, 1);
    // Control Center is open: Wi-Fi was toggled from it.
    await networkChange(AppLifecycleState.inactive);
    expect(session.networkChanges, 2);
    await networkChange(AppLifecycleState.hidden);
    await networkChange(AppLifecycleState.paused);
    expect(session.networkChanges, 2);
  });
}

class _CountingSession extends RelaySessionNotifier {
  int networkChanges = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  void onNetworkChanged() => networkChanges++;

  @override
  void onAppResumed() {}

  @override
  void onAppPaused() {}
}
