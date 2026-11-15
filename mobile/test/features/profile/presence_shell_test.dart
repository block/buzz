import 'package:buzz/app.dart';
import 'package:buzz/features/age_gate/age_signal_provider.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  testWidgets('signed-in shell publishes presence without opening Settings', (
    tester,
  ) async {
    final keys = nostr.Keys.generate();
    final session = _PublishingSession();
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final container = ProviderContainer(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        authProvider.overrideWith(_SignedIn.new),
        ageSignalProvider.overrideWith(_Allowed.new),
        relayConfigProvider.overrideWith(() => _Config(keys.nsec)),
        myPubkeyProvider.overrideWithValue(keys.public),
        appLifecycleProvider.overrideWith(_Resumed.new),
        relaySessionProvider.overrideWith(() => session),
        channelsProvider.overrideWith(_NoChannels.new),
      ],
    );
    addTearDown(container.dispose);

    await tester.pumpWidget(
      UncontrolledProviderScope(container: container, child: const App()),
    );
    for (var i = 0; i < 20 && session.presence.isEmpty; i++) {
      await tester.pump();
    }

    expect(session.presence, ['online']);
    await tester.pumpWidget(const SizedBox.shrink());
    container.dispose();
  });
}

class _PublishingSession extends RelaySessionNotifier {
  final presence = <String>[];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => const [];

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async => const [];

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async => () {};

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (event.kind == EventKind.presenceUpdate) presence.add(event.content);
    return event;
  }
}

class _SignedIn extends AuthNotifier {
  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.authenticated);
}

class _Allowed extends AgeSignalNotifier {
  @override
  AgeSignalState build() => AgeSignalState.allowed;
  @override
  Future<void> request() async {}
}

class _Config extends RelayConfigNotifier {
  _Config(this.nsec);

  final String nsec;

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example', nsec: nsec);
}

class _Resumed extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}

class _NoChannels extends ChannelsNotifier {
  @override
  bool get hasLoaded => true;

  @override
  Future<List<Channel>> build() async => const [];
}
