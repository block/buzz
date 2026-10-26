import 'dart:async';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:buzz/shared/me/me_chat_creation.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/me/me_assistants.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/me/me_page.dart';
import 'package:buzz/shared/me/me_groups.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/theme/theme.dart';

class FakeChannels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [
    for (final (id, name, member) in [
      ('personal', 'Design agent', true),
      ('ordinary', 'General', true),
      ('inaccessible', 'Hidden chat', false),
    ])
      Channel(
        id: id,
        name: name,
        channelType: 'stream',
        visibility: 'private',
        description: '',
        createdBy: 'owner',
        createdAt: DateTime(2026),
        memberCount: 2,
        isMember: member,
      ),
  ];
}

class LabeledChannels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [
    for (final (id, name, owner, member) in [
      ('filed', 'Filed labeled chat', 'owner', true),
      ('loose', 'Unfiled labeled chat', 'owner', true),
      ('shared', 'Someone else chat', 'other', true),
      ('hidden', 'Inaccessible labeled chat', 'owner', false),
    ])
      Channel(
        id: id,
        name: name,
        channelType: 'stream',
        visibility: 'private',
        labels: const ['buzz.solo'],
        description: '',
        createdBy: owner,
        createdAt: DateTime(2026),
        memberCount: 2,
        isMember: member,
      ),
  ];
}

void main() {
  testWidgets('label-only desktop chats appear in projects and unfiled chats', (
    tester,
  ) async {
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          myPubkeyProvider.overrideWithValue('owner'),
          channelsProvider.overrideWith(LabeledChannels.new),
          meGroupsProvider.overrideWith(
            (ref) => Stream.value(
              const MeGroups(
                groups: [
                  {
                    'id': 'project',
                    'name': 'App work',
                    'defaultTemplateId': '',
                  },
                ],
                assignments: {'filed': 'project'},
              ),
            ),
          ),
        ],
        child: MaterialApp(theme: AppTheme.light(), home: const MePage()),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Filed labeled chat'), findsOneWidget);
    expect(find.text('Unfiled labeled chat'), findsOneWidget);
    expect(find.text('Someone else chat'), findsNothing);
    expect(find.text('Inaccessible labeled chat'), findsNothing);
    expect(find.text('A place to work with your agents'), findsNothing);
  });

  testWidgets('empty workspace stays centered behind the New chat keyboard', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.viewPadding = const FakeViewPadding(bottom: 34);
    tester.view.padding = const FakeViewPadding(bottom: 34);
    addTearDown(tester.view.reset);
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          meGroupsProvider.overrideWith(
            (ref) => Stream.value(const MeGroups()),
          ),
          channelsProvider.overrideWith(FakeChannels.new),
        ],
        child: MaterialApp(theme: AppTheme.light(), home: const MePage()),
      ),
    );
    await tester.pumpAndSettle();
    final heading = find.text('A place to work with your agents');
    final position = tester.getCenter(heading);
    tester.view.viewInsets = const FakeViewPadding(bottom: 300);
    tester.view.padding = FakeViewPadding.zero;
    await tester.pumpAndSettle();
    expect(tester.getCenter(heading), position);
    tester.view.viewInsets = FakeViewPadding.zero;
    tester.view.padding = const FakeViewPadding(bottom: 34);
    await tester.pumpAndSettle();
    expect(tester.getCenter(heading), position);
  });

  testWidgets(
    'live desktop placement displays projects and only accessible Me chats',
    (tester) async {
      final updates = StreamController<MeGroups>();
      addTearDown(updates.close);
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            meGroupsProvider.overrideWith((ref) => updates.stream),
            channelsProvider.overrideWith(FakeChannels.new),
          ],
          child: MaterialApp(theme: AppTheme.light(), home: const MePage()),
        ),
      );
      updates.add(
        const MeGroups(
          groups: [
            {'id': 'project', 'name': 'App work', 'defaultTemplateId': ''},
          ],
          assignments: {'personal': 'project'},
          channels: ['personal', 'inaccessible'],
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('Search chats'), findsNothing);
      expect(find.byType(TextField), findsNothing);
      expect(find.byIcon(BuzzIcons.edit), findsOneWidget);
      expect(find.byIcon(BuzzIcons.folderPlus), findsOneWidget);
      final header = tester.widget<FrostedAppBar>(find.byType(FrostedAppBar));
      expect(header.nativeLargeTitle, isTrue);
      expect(header.nativeActions!.map((action) => action.icon), [
        BuzzIcons.folderPlus,
        BuzzIcons.edit,
      ]);
      expect(find.text('App work'), findsOneWidget);
      expect(find.text('Design agent'), findsOneWidget);
      expect(find.text('General'), findsNothing);
      expect(find.text('Hidden chat'), findsNothing);
      expect(find.text('Chats'), findsOneWidget);
      await tester.tap(find.text('App work'));
      await tester.pumpAndSettle();
      expect(find.text('Design agent'), findsNothing);
      await tester.tap(find.text('App work'));
      await tester.pumpAndSettle();
      expect(find.text('Design agent'), findsOneWidget);
      updates.add(
        const MeGroups(
          groups: [
            {'id': 'project', 'name': 'App work', 'defaultTemplateId': ''},
          ],
          channels: ['personal'],
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('No chats in this project'), findsOneWidget);
      expect(find.text('Design agent'), findsOneWidget);
      await tester.tap(find.text('Chats'));
      await tester.pumpAndSettle();
      expect(find.text('Design agent'), findsNothing);
      expect(find.text('No chats in this project'), findsOneWidget);
      await tester.tap(find.text('Chats'));
      await tester.pumpAndSettle();
      expect(find.text('Design agent'), findsOneWidget);

      updates.add(const MeGroups());
      await tester.pumpAndSettle();
      expect(find.text('Design agent'), findsNothing);
      expect(find.text('A place to work with your agents'), findsOneWidget);
    },
  );
  testWidgets(
    'footer new chat offers project selection without losing its name',
    (tester) async {
      final action = ValueNotifier<VoidCallback?>(null);
      addTearDown(action.dispose);
      final sync = MeSync(
        RelaySessionNotifier(),
        RelayConfig(
          baseUrl: 'https://relay.test',
          nsec: nostr.Nip19.encode(
            prefix: nostr.Nip19Prefix.nsec,
            data:
                '0000000000000000000000000000000000000000000000000000000000000001',
          ),
        ),
      );
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            meGroupsProvider.overrideWith(
              (ref) => Stream.value(
                const MeGroups(
                  groups: [
                    {
                      'id': 'mobile',
                      'name': 'Mobile app',
                      'defaultTemplateId': '',
                    },
                  ],
                ),
              ),
            ),
            meSyncProvider.overrideWithValue(sync),
            channelsProvider.overrideWith(FakeChannels.new),
            meAssistantsProvider.overrideWith((ref) async => []),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: MePage(newChatAction: action),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.byTooltip('New chat'), findsNothing);
      expect(action.value, isNotNull);
      action.value!();
      await tester.pumpAndSettle();
      expect(find.byType(BottomSheet), findsOneWidget);
      expect(
        find.byKey(const ValueKey('new-session-name')).hitTestable(),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('new-session-assistant')).hitTestable(),
        findsOneWidget,
      );
      await tester.enterText(
        find.byKey(const ValueKey('new-session-name')),
        'Design session',
      );
      await tester.tap(find.byKey(const ValueKey('new-session-project')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Mobile app').last);
      await tester.pumpAndSettle();
      expect(find.text('Design session'), findsOneWidget);
      expect(find.text('Mobile app'), findsWidgets);
      await tester.tap(find.byKey(const ValueKey('new-session-project')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('No project').last);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Create chat'));
      await tester.pumpAndSettle();
      expect(find.text('Enter a name and choose an agent.'), findsOneWidget);
      expect(find.text('Design session'), findsOneWidget);
    },
  );

  testWidgets('iOS new chat uses the native profile form with project choices', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    const channel = MethodChannel('buzz/profile_text_editor');
    final calls = <MethodCall>[];
    var directoryLoads = 0;
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (
      call,
    ) async {
      calls.add(call);
      return null;
    });
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        channel,
        null,
      ),
    );
    final sync = MeSync(
      RelaySessionNotifier(),
      RelayConfig(
        baseUrl: 'https://relay.test',
        nsec: nostr.Nip19.encode(
          prefix: nostr.Nip19Prefix.nsec,
          data:
              '0000000000000000000000000000000000000000000000000000000000000001',
        ),
      ),
    );
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          meGroupsProvider.overrideWith(
            (ref) => Stream.value(
              const MeGroups(
                groups: [
                  {
                    'id': 'mobile',
                    'name': 'Mobile app',
                    'defaultTemplateId': '',
                  },
                ],
              ),
            ),
          ),
          meSyncProvider.overrideWithValue(sync),
          channelsProvider.overrideWith(FakeChannels.new),
          meAssistantsProvider.overrideWith((ref) async {
            directoryLoads++;
            return [];
          }),
        ],
        child: MaterialApp(theme: AppTheme.light(), home: const MePage()),
      ),
    );
    await tester.pumpAndSettle();

    final bar = tester.widget<FrostedAppBar>(find.byType(FrostedAppBar));
    bar.nativeActions!
        .firstWhere((action) => action.label == 'New chat')
        .onPressed!();
    await tester.pumpAndSettle();
    expect(directoryLoads, 1);
    expect(calls, hasLength(1));
    expect(calls.single.method, 'presentChat');
    final args = calls.single.arguments as Map;
    expect(args['title'], 'New chat');
    expect(args['placeholder'], 'Chat name');
    expect(args['assistants'], isEmpty);
    expect(args['projects'], [
      {'id': 'mobile', 'name': 'Mobile app'},
    ]);
    expect(args['selectedProject'], isNull);
    expect(args['containerCornerRadius'], Radii.container);
    expect(find.byType(BottomSheet), findsNothing);
    await tester.pumpWidget(const SizedBox.shrink());
    debugDefaultTargetPlatformOverride = null;
  });

  for (final resume in [true, false]) {
    testWidgets('iOS new chat recovers a durable creation: resume=$resume', (
      tester,
    ) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      const channel = MethodChannel('buzz/profile_text_editor');
      final calls = <MethodCall>[];
      var directoryLoads = 0;
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (
        call,
      ) async {
        calls.add(call);
        return null;
      });
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          channel,
          null,
        ),
      );
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final sync = MeSync(
        RelaySessionNotifier(),
        RelayConfig(
          baseUrl: 'https://relay.test',
          nsec: nostr.Nip19.encode(
            prefix: nostr.Nip19Prefix.nsec,
            data:
                '0000000000000000000000000000000000000000000000000000000000000001',
          ),
        ),
        prefs: prefs,
      );
      await const MeChatCreation(
        'same-id',
        'Unfinished chat',
        'agent',
        project: 'mobile',
      ).save(sync);
      final restarted = MeSync(
        RelaySessionNotifier(),
        sync.config,
        prefs: prefs,
      );
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            meGroupsProvider.overrideWith(
              (ref) => Stream.value(const MeGroups()),
            ),
            meSyncProvider.overrideWithValue(restarted),
            channelsProvider.overrideWith(FakeChannels.new),
            meAssistantsProvider.overrideWith((ref) async {
              directoryLoads++;
              return [];
            }),
          ],
          child: MaterialApp(theme: AppTheme.light(), home: const MePage()),
        ),
      );
      await tester.pumpAndSettle();

      final bar = tester.widget<FrostedAppBar>(find.byType(FrostedAppBar));
      bar.nativeActions!
          .firstWhere((action) => action.label == 'New chat')
          .onPressed!();
      await tester.pumpAndSettle();
      await tester.tap(find.text(resume ? 'Resume' : 'Discard setup'));
      await tester.pumpAndSettle();
      expect(directoryLoads, 1);
      expect(calls, hasLength(1));
      expect(calls.single.method, 'presentChat');
      final args = calls.single.arguments as Map;
      expect(args['title'], 'New chat');
      expect(args['initialValue'], resume ? 'Unfinished chat' : '');
      expect(args['selectedAssistant'], resume ? 'agent' : isNull);
      expect(args['selectedProject'], resume ? 'mobile' : isNull);
      expect(args['fieldsLocked'], resume);
      expect(args['error'], resume ? contains('Resume') : isNull);
      expect(
        MeChatCreation.pending(restarted)?.id,
        resume ? 'same-id' : isNull,
      );
      expect(args['placeholder'], 'Chat name');
      expect(args['assistants'], isEmpty);
      expect(args['containerCornerRadius'], Radii.container);
      expect(find.byType(BottomSheet), findsNothing);
      await tester.pumpWidget(const SizedBox.shrink());
      debugDefaultTargetPlatformOverride = null;
    });
  }

  testWidgets('unavailable preferences show retry instead of empty state', (
    tester,
  ) async {
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          meGroupsProvider.overrideWith(
            (ref) => Stream<MeGroups>.error(StateError('offline')),
          ),
          channelsProvider.overrideWith(FakeChannels.new),
        ],
        child: MaterialApp(theme: AppTheme.light(), home: const MePage()),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Try again'), findsOneWidget);
    expect(find.text('A place to work with your agents'), findsNothing);
  });
}
