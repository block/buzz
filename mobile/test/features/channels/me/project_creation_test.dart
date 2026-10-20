import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/me/me_page.dart';
import 'package:buzz/shared/me/me_assistants.dart';
import 'package:buzz/shared/me/me_groups.dart';
import 'package:buzz/shared/me/me_project_creation.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:shared_preferences_platform_interface/shared_preferences_platform_interface.dart';

class _FailingStore extends InMemorySharedPreferencesStore {
  final bool throws;
  _FailingStore(this.throws) : super.empty();
  @override
  Future<bool> setValue(String type, String key, Object value) async {
    if (throws) throw StateError('disk failure');
    return false;
  }
}

class _Channels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [];
}

class _Relay extends RelaySessionNotifier {
  List<NostrEvent> events = [];
  VoidCallback? beforeRead;
  bool failRead = false;
  bool failAfterPublish = false;
  int writes = 0;
  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    beforeRead?.call();
    if (failRead) throw StateError('interrupted before the first head');
    return events;
  }

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    events = [event];
    writes++;
    if (failAfterPublish) throw StateError('lost acknowledgement');
    return event;
  }
}

void main() {
  final config = RelayConfig(
    baseUrl: 'https://relay.test',
    nsec: nostr.Nip19.encode(
      prefix: nostr.Nip19Prefix.nsec,
      data: '0' * 63 + '1',
    ),
  );
  for (final throws in [false, true]) {
    testWidgets(
      'failed project write ($throws) reopens the entered name without claiming recovery',
      (tester) async {
        SharedPreferences.setMockInitialValues({});
        SharedPreferencesStorePlatform.instance = _FailingStore(throws);
        final prefs = await SharedPreferences.getInstance();
        final relay = _Relay();
        final sync = MeSync(relay, config, prefs: prefs);
        const channel = MethodChannel('buzz/profile_text_editor');
        var forms = 0;
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          channel,
          (call) async {
            forms++;
            if (forms == 1) return 'My entered name';
            expectSync(
              (call.arguments as Map)['initialValue'],
              'My entered name',
            );
            return null;
          },
        );
        addTearDown(
          () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
            channel,
            null,
          ),
        );
        await tester.pumpWidget(
          ProviderScope(
            overrides: [
              meSyncProvider.overrideWithValue(sync),
              meGroupsProvider.overrideWith(
                (ref) => Stream.value(const MeGroups()),
              ),
              meAssistantsProvider.overrideWith((ref) async => []),
              channelsProvider.overrideWith(_Channels.new),
            ],
            child: MaterialApp(theme: AppTheme.light(), home: const MePage()),
          ),
        );
        await tester.pumpAndSettle();
        tester
            .widget<FrostedAppBar>(find.byType(FrostedAppBar))
            .nativeActions!
            .firstWhere((a) => a.label == 'New project')
            .onPressed!();
        await tester.pumpAndSettle();
        expect(find.text('Project was not saved'), findsOneWidget);
        expect(find.textContaining('Project setup saved'), findsNothing);
        expect(MeProjectCreation.pending(sync), isNull);
        expect(relay.writes, 0);
        await tester.tap(find.text('Try again'));
        await tester.pumpAndSettle();
        expect(forms, 2);
      },
      variant: TargetPlatformVariant.only(TargetPlatform.iOS),
    );
  }
  for (final recovery in ['Resume', 'Discard setup']) {
    for (final offline in [true, false]) {
      testWidgets(
        'accepted project survives ${offline ? "offline" : "interrupted head"} restart with $recovery',
        (tester) async {
          SharedPreferences.setMockInitialValues({});
          final prefs = await SharedPreferences.getInstance();
          final relay = _Relay()..failRead = !offline;
          var sync = MeSync(relay, config, prefs: prefs, connected: !offline);
          var forms = 0;
          relay.beforeRead = () => expectSync(
            MeProjectCreation.pending(sync)?.name,
            forms == 1 ? 'Private project' : 'Replacement project',
          );
          const channel = MethodChannel('buzz/profile_text_editor');
          tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
            channel,
            (call) async {
              expectSync(call.method, 'presentProject');
              forms++;
              return forms == 1 ? 'Private project' : 'Replacement project';
            },
          );
          addTearDown(
            () => tester.binding.defaultBinaryMessenger
                .setMockMethodCallHandler(channel, null),
          );
          Future<void> mount() async {
            await tester.pumpWidget(
              ProviderScope(
                key: UniqueKey(),
                overrides: [
                  meSyncProvider.overrideWithValue(sync),
                  meGroupsProvider.overrideWith(
                    (ref) => Stream.value(const MeGroups()),
                  ),
                  meAssistantsProvider.overrideWith((ref) async => []),
                  channelsProvider.overrideWith(_Channels.new),
                ],
                child: MaterialApp(
                  theme: AppTheme.light(),
                  home: const MePage(),
                ),
              ),
            );
            await tester.pumpAndSettle();
          }

          Future<void> open() async {
            tester
                .widget<FrostedAppBar>(find.byType(FrostedAppBar))
                .nativeActions!
                .firstWhere((a) => a.label == 'New project')
                .onPressed!();
            await tester.pumpAndSettle();
          }

          await mount();
          await open();
          final saved = MeProjectCreation.pending(sync);
          expect(
            saved,
            isNotNull,
            reason: 'Accepted project intent must be durable before networking',
          );
          final pending = saved!;
          expect(pending.name, 'Private project');
          expect(
            prefs.getString(MeProjectCreation.storageKey(sync)),
            isNot(contains('Private project')),
          );
          expect(relay.writes, 0);
          sync = MeSync(relay, config, prefs: prefs);
          relay.failRead = false;
          await mount();
          await open();
          expect(find.textContaining('Private project'), findsOneWidget);
          await tester.tap(find.text('Later'));
          await tester.pumpAndSettle();
          expect(MeProjectCreation.pending(sync)?.id, pending.id);
          expect(relay.writes, 0);
          await open();
          await tester.tap(find.text(recovery));
          await tester.pumpAndSettle();
          expect(forms, recovery == 'Resume' ? 1 : 2);
          expect(
            MeProjectCreation.pending(sync),
            isNull,
            reason: tester
                .widgetList<Text>(find.byType(Text))
                .map((t) => t.data)
                .join(' | '),
          );
          expect(relay.writes, 1);
          expect(
            sync.decode(relay.events.single).groups.single['id'],
            recovery == 'Resume' ? pending.id : isNot(pending.id),
          );
          expect(
            sync.decode(relay.events.single).groups.single['name'],
            recovery == 'Resume' ? pending.name : 'Replacement project',
          );
        },
        variant: TargetPlatformVariant.only(TargetPlatform.iOS),
      );
    }
  }
  test(
    'scoped offline discard compares the reviewed intent and allows a different project',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final sync = MeSync(_Relay(), config, prefs: prefs, connected: false);
      const original = MeProjectCreation('project', 'Private project');
      await original.save(sync);
      for (final other in [
        RelayConfig(baseUrl: 'https://other.test', nsec: config.nsec),
        RelayConfig(
          baseUrl: config.baseUrl,
          nsec: nostr.Nip19.encode(
            prefix: nostr.Nip19Prefix.nsec,
            data: '0' * 63 + '2',
          ),
        ),
      ]) {
        expect(
          MeProjectCreation.pending(MeSync(_Relay(), other, prefs: prefs)),
          isNull,
        );
      }
      await expectLater(
        const MeProjectCreation('different', 'Private project').discard(sync),
        throwsStateError,
      );
      expect(MeProjectCreation.pending(sync)?.id, original.id);
      await original.discard(sync);
      await const MeProjectCreation('new', 'Different project').save(sync);
      await expectLater(original.discard(sync), throwsStateError);
      expect(MeProjectCreation.pending(sync)?.id, 'new');
    },
  );
  test(
    'lost publication acknowledgement retries the same ID without duplicating it',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final relay = _Relay()..failAfterPublish = true;
      final sync = MeSync(relay, config, prefs: prefs);
      const intent = MeProjectCreation('project', 'Name');
      await expectLater(intent.complete(sync), throwsStateError);
      final restarted = MeSync(relay, config, prefs: prefs);
      relay.failAfterPublish = false;
      await expectLater(
        MeProjectCreation.pending(restarted)!.complete(restarted),
        throwsStateError,
      );
      await MeProjectCreation.pending(restarted)!.complete(restarted);
      expect(MeProjectCreation.pending(restarted), isNull);
      expect(relay.writes, 1);
      expect(restarted.decode(relay.events.single).groups, hasLength(1));
    },
  );
}
