import 'package:buzz/features/activity/inbox_local_state_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

class _SwitchableRelayConfig extends RelayConfigNotifier {
  @override
  RelayConfig build() => const RelayConfig(baseUrl: 'https://relay-a.example');

  void switchTo(String baseUrl) => state = RelayConfig(baseUrl: baseUrl);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  Future<(ProviderContainer, _SwitchableRelayConfig)> start(
    Map<String, Object> stored,
  ) async {
    SharedPreferences.setMockInitialValues(stored);
    final prefs = await SharedPreferences.getInstance();
    final config = _SwitchableRelayConfig();
    final container = ProviderContainer(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        relayConfigProvider.overrideWith(() => config),
        myPubkeyProvider.overrideWithValue('pk_a'),
      ],
    );
    addTearDown(container.dispose);
    return (container, config);
  }

  test('each community keeps its own done and unread rows', () async {
    final (container, config) = await start({});
    InboxLocalState state() => container.read(inboxLocalStateProvider);

    container.read(inboxLocalStateProvider.notifier).markDone('approval-a');
    expect(state().doneIds, {'approval-a'});

    // Filling the second community's cap must not evict the first's rows.
    config.switchTo('https://relay-b.example');
    expect(state().doneIds, isEmpty);
    final notifier = container.read(inboxLocalStateProvider.notifier);
    for (var i = 0; i < 600; i++) {
      notifier.markDone('approval-b-$i');
    }

    config.switchTo('https://relay-a.example');
    expect(state().doneIds, {'approval-a'});
  });

  test('the first community to open claims the device-wide rows', () async {
    final (container, config) = await start({
      'activity_inbox_local_state_v1': '{"done":["approval-a"],"unread":[]}',
    });

    expect(container.read(inboxLocalStateProvider).doneIds, {'approval-a'});
    config.switchTo('https://relay-b.example');
    expect(container.read(inboxLocalStateProvider).doneIds, isEmpty);
    config.switchTo('https://relay-a.example');
    expect(container.read(inboxLocalStateProvider).doneIds, {'approval-a'});
  });
}
