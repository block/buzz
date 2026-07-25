import 'package:buzz/features/activity/compose_drafts_provider.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  Future<ProviderContainer> containerWithPrefs() async {
    final prefs = await SharedPreferences.getInstance();
    final container = ProviderContainer(
      overrides: [savedPrefsProvider.overrideWithValue(prefs)],
    );
    addTearDown(container.dispose);
    return container;
  }

  test('composeDraftKey separates channel and thread composers', () {
    expect(composeDraftKey('ch1'), 'ch1');
    expect(composeDraftKey('ch1', threadHeadId: 't1'), 'ch1:t1');
  });

  test('save adds a draft and textFor returns it', () async {
    SharedPreferences.setMockInitialValues({});
    final container = await containerWithPrefs();
    final notifier = container.read(composeDraftsProvider.notifier);

    notifier.save(key: 'ch1', channelId: 'ch1', text: 'hello there');

    final drafts = container.read(composeDraftsProvider);
    expect(drafts, hasLength(1));
    expect(drafts.single.text, 'hello there');
    expect(notifier.textFor('ch1'), 'hello there');
  });

  test('empty text removes the draft', () async {
    SharedPreferences.setMockInitialValues({});
    final container = await containerWithPrefs();
    final notifier = container.read(composeDraftsProvider.notifier);

    notifier.save(key: 'ch1', channelId: 'ch1', text: 'hello');
    notifier.save(key: 'ch1', channelId: 'ch1', text: '   ');

    expect(container.read(composeDraftsProvider), isEmpty);
  });

  test('drafts persist across container restarts', () async {
    SharedPreferences.setMockInitialValues({});
    final first = await containerWithPrefs();
    first
        .read(composeDraftsProvider.notifier)
        .save(key: 'ch1:t1', channelId: 'ch1', threadHeadId: 't1', text: 'wip');

    final second = await containerWithPrefs();
    final restored = second.read(composeDraftsProvider);
    expect(restored, hasLength(1));
    expect(restored.single.channelId, 'ch1');
    expect(restored.single.threadHeadId, 't1');
    expect(restored.single.text, 'wip');
  });

  test('remove deletes by key', () async {
    SharedPreferences.setMockInitialValues({});
    final container = await containerWithPrefs();
    final notifier = container.read(composeDraftsProvider.notifier);

    notifier.save(key: 'ch1', channelId: 'ch1', text: 'hello');
    notifier.remove('ch1');

    expect(container.read(composeDraftsProvider), isEmpty);
  });

  test('malformed persisted json is ignored', () async {
    SharedPreferences.setMockInitialValues({'compose_drafts_v1': '{not valid'});
    final container = await containerWithPrefs();
    expect(container.read(composeDraftsProvider), isEmpty);
  });
}
