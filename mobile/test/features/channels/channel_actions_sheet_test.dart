import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_actions_sheet.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channel_sections/channel_sections_provider.dart';
import 'package:buzz/features/channels/channel_sections/channel_sections_storage.dart';
import 'package:buzz/features/channels/channel_sections/workspace_layout.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/manage_channel_sheet.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/buzz_action_tile.dart';
import 'package:buzz/shared/widgets/app_list_card.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:hooks_riverpod/misc.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:shared_preferences_platform_interface/shared_preferences_platform_interface.dart';

const _currentPubkey = 'me';

Channel _channel({
  String type = 'stream',
  bool isArchived = false,
  bool isMember = true,
}) => Channel(
  id: 'channel-id',
  name: type == 'dm' ? 'Alice' : 'general',
  channelType: type,
  visibility: 'open',
  description: '',
  createdBy: 'owner',
  createdAt: DateTime(2025),
  memberCount: 2,
  isMember: isMember,
  archivedAt: isArchived ? DateTime(2025, 1, 2) : null,
);

Widget _app({
  required Channel channel,
  required Future<List<ChannelMember>> Function() loadMembers,
  bool isUnread = false,
  AsyncValue<Map<String, String>> agentOwners = const AsyncValue.data(
    <String, String>{},
  ),
  ChannelActions Function(Ref ref)? createChannelActions,
  String? currentPubkey = _currentPubkey,
}) => ProviderScope(
  overrides: [
    currentPubkeyProvider.overrideWith((ref) => currentPubkey),
    channelMembersProvider(channel.id).overrideWith((ref) => loadMembers()),
    agentOwnersProvider.overrideWithValue(agentOwners),
    if (createChannelActions != null)
      channelActionsProvider.overrideWith(createChannelActions),
  ],
  child: MaterialApp(
    theme: AppTheme.light(),
    home: Scaffold(
      body: ChannelActionsSheet(channel: channel, isUnread: isUnread),
    ),
  ),
);

Widget _modalApp({
  required Channel channel,
  required Future<List<ChannelMember>> Function() loadMembers,
  required ChannelActions Function(Ref ref) createChannelActions,
  bool soloDetails = false,
  bool details = false,
  String? canvasContent,
  Future<ChannelCanvas>? pendingCanvas,
  List<Override>? sectionOverrides,
  String? sectionId,
}) => ProviderScope(
  overrides: [
    currentPubkeyProvider.overrideWith((ref) => _currentPubkey),
    channelMembersProvider(channel.id).overrideWith((ref) => loadMembers()),
    ...sectionOverrides ??
        [
          channelSectionsProvider.overrideWith(
            () => _FakeChannelSectionsNotifier(const ChannelSectionStore()),
          ),
        ],
    agentOwnersProvider.overrideWithValue(
      const AsyncValue.data(<String, String>{}),
    ),
    channelCanvasProvider(channel.id).overrideWith(
      (ref) =>
          pendingCanvas ??
          ChannelCanvas(
            content: canvasContent,
            updatedAt: null,
            authorPubkey: null,
          ),
    ),
    channelActionsProvider.overrideWith(createChannelActions),
  ],
  child: MaterialApp(
    theme: AppTheme.light(),
    home: Builder(
      builder: (context) => Scaffold(
        body: TextButton(
          onPressed: () => soloDetails || details
              ? showChannelDetailsSheet(
                  context: context,
                  channel: channel,
                  currentPubkey: _currentPubkey,
                  onMemberTap: (_, _) {},
                  personalChat: soloDetails,
                )
              : showChannelActionsSheet(
                  context: context,
                  channel: channel,
                  isUnread: false,
                  sectionId: sectionId,
                ),
          child: const Text('Open actions'),
        ),
      ),
    ),
  ),
);

void main() {
  for (final size in [const Size(800, 390), const Size(669, 951)]) {
    testWidgets('Duo cover or portrait $size retains the Flutter sheet', (
      tester,
    ) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = size;
      tester.view.padding = const FakeViewPadding(right: 76);
      tester.view.viewPadding = tester.view.padding;
      addTearDown(tester.view.reset);
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      await tester.pumpWidget(
        _modalApp(
          channel: _channel(),
          loadMembers: () async => [],
          createChannelActions: (ref) => _FakeChannelActions(ref),
        ),
      );
      await tester.tap(find.text('Open actions'));
      await tester.pumpAndSettle();
      expect(find.byType(BottomSheet), findsOneWidget);
      expect(find.byType(ChannelActionsSheet), findsOneWidget);
      debugDefaultTargetPlatformOverride = null;
    });
  }

  for (final soloDetails in [false, true]) {
    testWidgets(
      'Duo ${soloDetails ? 'Solo details' : 'channel actions'} uses native presentation and live permissions',
      (tester) async {
        debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
        tester.view.devicePixelRatio = 1;
        tester.view.physicalSize = const Size(951, 669);
        tester.view.padding = const FakeViewPadding(right: 76);
        tester.view.viewPadding = tester.view.padding;
        addTearDown(tester.view.reset);
        addTearDown(() => debugDefaultTargetPlatformOverride = null);
        const bridge = MethodChannel('buzz/channel_actions_sheet');
        final clipboard = <String>[];
        // Copy feedback creates the app's native toast composition layer.
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          SystemChannels.platform_views,
          (call) async {
            if (call.method == 'create') {
              expect((call.arguments as Map)['viewType'], 'buzz/toast_layer');
            }
            return null;
          },
        );
        addTearDown(
          () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
            SystemChannels.platform_views,
            null,
          ),
        );
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          SystemChannels.platform,
          (call) async {
            if (call.method == 'Clipboard.setData') {
              clipboard.add((call.arguments as Map)['text'] as String);
            }
            return null;
          },
        );
        addTearDown(
          () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
            SystemChannels.platform,
            null,
          ),
        );
        final result = Completer<String?>();
        final members = Completer<List<ChannelMember>>();
        final calls = <MethodCall>[];
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(bridge, (
          call,
        ) async {
          calls.add(call);
          return call.method == 'present' ? result.future : null;
        });
        addTearDown(
          () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
            bridge,
            null,
          ),
        );
        await tester.pumpWidget(
          _modalApp(
            channel: _channel(),
            soloDetails: soloDetails,
            loadMembers: () => members.future,
            createChannelActions: (ref) => _FakeChannelActions(ref),
          ),
        );
        await tester.tap(find.text('Open actions'));
        await tester.pumpAndSettle();
        if (soloDetails) {
          expect(calls.last.arguments.toString(), contains('general'));
          expect(
            calls.last.arguments.toString(),
            isNot(contains('Move to section')),
          );
        }
        expect(calls.where((call) => call.method == 'present'), hasLength(1));
        expect(find.byType(Dialog), findsNothing);
        expect(find.byType(BottomSheet), findsNothing);
        expect(
          calls.last.arguments.toString(),
          isNot(contains('Delete channel')),
        );
        members.complete([
          ChannelMember(
            pubkey: _currentPubkey,
            role: 'owner',
            joinedAt: DateTime(2025),
          ),
        ]);
        await tester.pumpAndSettle();
        expect(calls.last.arguments.toString(), contains('Delete channel'));
        final displayModel = Map<String, dynamic>.from(
          calls
                  .lastWhere(
                    (call) =>
                        call.method == 'update' || call.method == 'present',
                  )
                  .arguments
              as Map,
        );
        result.complete('Copy channel ID');
        await tester.pumpAndSettle();
        expect(clipboard, ['channel-id']);
        expect(find.byType(ChannelActionsSheet), findsNothing);
        expect(find.byType(ChannelDetailsPage), findsNothing);
        expect(find.text('Open actions'), findsOneWidget);
        expect(tester.takeException(), isNull);
        // Render exactly the model handed to the embedded Flutter engine.
        tester.view.physicalSize = const Size(540, 580);
        final selected = <String>[];
        await tester.pumpWidget(
          ProviderScope(
            key: const ValueKey('embedded-sheet-engine'),
            child: MaterialApp(
              theme: utilitySurfaceThemeData(AppTheme.light()),
              home: ChannelSheetContent(
                data: displayModel,
                onSelect: selected.add,
              ),
            ),
          ),
        );
        await tester.pumpAndSettle();
        expect(find.byType(BuzzActionTile), findsNWidgets(soloDetails ? 3 : 2));
        expect(find.byType(AppListCard), findsWidgets);
        if (soloDetails) {
          expect(
            find.byKey(const ValueKey('channel-details-avatar')),
            findsOneWidget,
          );
          expect(find.text('general'), findsOneWidget);
        }
        await tester.tap(find.text('Star'));
        expect(selected, ['Star']);
        await tester.pumpWidget(const SizedBox());
        await tester.pumpAndSettle();
        expect(tester.takeException(), isNull);
        debugDefaultTargetPlatformOverride = null;
      },
    );
  }

  testWidgets('owner sees the complete regular-channel action set', (
    tester,
  ) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        loadMembers: () async => [
          ChannelMember(
            pubkey: _currentPubkey,
            role: 'owner',
            joinedAt: DateTime(2025),
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    for (final label in [
      'Star',
      'Mark Unread',
      'Move to section…',
      'Mute channel',
      'Manage channel',
      'Copy channel name',
      'Copy channel ID',
      'Leave channel',
      'Archive channel',
      'Delete channel',
    ]) {
      expect(find.text(label), findsOneWidget, reason: label);
    }

    final moveTop = tester.getTopLeft(find.text('Move to section…')).dy;
    final muteTop = tester.getTopLeft(find.text('Mute channel')).dy;
    final manageTop = tester.getTopLeft(find.text('Manage channel')).dy;
    final copyNameTop = tester.getTopLeft(find.text('Copy channel name')).dy;
    final copyIdTop = tester.getTopLeft(find.text('Copy channel ID')).dy;
    expect(moveTop, lessThan(muteTop));
    expect(muteTop, lessThan(manageTop));
    expect(manageTop, lessThan(copyNameTop));
    expect(copyNameTop, lessThan(copyIdTop));
  });

  testWidgets('unread channel uses the Mark Read label', (tester) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        isUnread: true,
        loadMembers: () async => const [],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Mark Read'), findsOneWidget);
    expect(find.text('Mark Unread'), findsNothing);
  });

  group('Mark Read', () {
    late _KnownTimesChannels channels;
    late _RecordingReadState readState;

    Future<void> pumpSheet(WidgetTester tester, Channel channel) async {
      channels = _KnownTimesChannels();
      readState = _RecordingReadState();
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            currentPubkeyProvider.overrideWith((ref) => _currentPubkey),
            channelMembersProvider(
              channel.id,
            ).overrideWith((ref) async => const <ChannelMember>[]),
            agentOwnersProvider.overrideWithValue(
              const AsyncValue.data(<String, String>{}),
            ),
            channelsProvider.overrideWith(() => channels),
            readStateProvider.overrideWith(() => readState),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Builder(
              builder: (context) => Scaffold(
                body: TextButton(
                  onPressed: () => showChannelActionsSheet(
                    context: context,
                    channel: channel,
                    isUnread: true,
                  ),
                  child: const Text('Open actions'),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Open actions'));
      await tester.pumpAndSettle();
    }

    testWidgets('covers a message that arrived while the sheet was open', (
      tester,
    ) async {
      final opened = DateTime.utc(2026, 1, 1);
      final channel = Channel(
        id: 'channel-id',
        name: 'general',
        channelType: 'stream',
        visibility: 'open',
        description: '',
        createdBy: 'owner',
        createdAt: DateTime(2025),
        memberCount: 2,
        isMember: true,
        lastMessageAt: opened,
      );
      await pumpSheet(tester, channel);
      final openedAt = opened.millisecondsSinceEpoch ~/ 1000;
      channels.latest = openedAt + 30;

      await tester.tap(find.text('Mark Read'));
      await tester.pumpAndSettle();

      expect(readState.marks, [('channel-id', openedAt + 30, true)]);
    });

    testWidgets('with no known message time only releases forced unread', (
      tester,
    ) async {
      await pumpSheet(tester, _channel());

      await tester.tap(find.text('Mark Read'));
      await tester.pumpAndSettle();

      expect(readState.marks, [('channel-id', 0, true)]);
    });
  });

  testWidgets('admin can archive but cannot delete', (tester) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        loadMembers: () async => [
          ChannelMember(
            pubkey: _currentPubkey,
            role: 'admin',
            joinedAt: DateTime(2025),
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Archive channel'), findsOneWidget);
    expect(find.text('Delete channel'), findsNothing);
  });

  testWidgets('verified owner agent grants archive and delete', (tester) async {
    const agentPubkey = 'agent';
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        agentOwners: const AsyncValue.data({agentPubkey: _currentPubkey}),
        loadMembers: () async => [
          ChannelMember(
            pubkey: agentPubkey,
            role: 'owner',
            joinedAt: DateTime(2025),
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Archive channel'), findsOneWidget);
    expect(find.text('Delete channel'), findsOneWidget);
  });

  testWidgets('unresolved identity grants no lifecycle actions', (
    tester,
  ) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        currentPubkey: null,
        loadMembers: () async => [
          ChannelMember(
            pubkey: 'ordinary-owner',
            role: 'owner',
            joinedAt: DateTime(2025),
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Archive channel'), findsNothing);
    expect(find.text('Delete channel'), findsNothing);
  });

  testWidgets('archived owner can unarchive but cannot delete', (tester) async {
    late _FakeChannelActions actions;
    await tester.pumpWidget(
      _app(
        channel: _channel(isArchived: true),
        loadMembers: () async => [
          ChannelMember(
            pubkey: _currentPubkey,
            role: 'owner',
            joinedAt: DateTime(2025),
          ),
        ],
        createChannelActions: (ref) => actions = _FakeChannelActions(ref),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Archive channel'), findsNothing);
    expect(find.text('Unarchive channel'), findsOneWidget);
    expect(find.text('Delete channel'), findsNothing);

    await tester.tap(find.text('Unarchive channel'));
    await tester.pumpAndSettle();
    expect(find.text('Unarchive #general?'), findsOneWidget);
    await tester.tap(find.widgetWithText(FilledButton, 'Unarchive'));
    await tester.pumpAndSettle();

    expect(actions.unarchivedChannelId, 'channel-id');
  });

  testWidgets('owned non-owner agent grants no lifecycle actions', (
    tester,
  ) async {
    const agentPubkey = 'agent';
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        agentOwners: const AsyncValue.data({agentPubkey: _currentPubkey}),
        loadMembers: () async => [
          ChannelMember(
            pubkey: agentPubkey,
            role: 'bot',
            joinedAt: DateTime(2025),
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Archive channel'), findsNothing);
    expect(find.text('Delete channel'), findsNothing);
  });

  testWidgets('agent ownership loading keeps lifecycle actions pending', (
    tester,
  ) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        agentOwners: const AsyncValue.loading(),
        loadMembers: () async => const [],
      ),
    );
    await tester.pump();

    expect(find.text('Loading channel actions…'), findsOneWidget);
    expect(find.text('Archive channel'), findsNothing);
    expect(find.text('Delete channel'), findsNothing);
  });

  for (final (details, early) in [
    (false, false),
    (true, false),
    (true, true),
  ]) {
    testWidgets('desktop workspaces close filing (details: $details, '
        'before the picker draws: $early)', (tester) async {
      tester.view.physicalSize = const Size(1200, 3000);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(
        _modalApp(
          channel: _channel(),
          details: details,
          loadMembers: () async => const [],
          createChannelActions: (ref) => _FakeChannelActions(ref),
        ),
      );
      await tester.tap(find.text('Open actions'));
      await tester.pumpAndSettle();
      void lock() =>
          (ProviderScope.containerOf(
                    tester.element(find.text('Open actions')),
                  ).read(channelSectionsProvider.notifier)
                  as _FakeChannelSectionsNotifier)
              .lock();
      await tester.tap(find.text('Move to section…'));
      if (early) lock();
      await tester.pumpAndSettle();
      if (!early) {
        expect(find.text('New section…'), findsOneWidget);
        // Desktop saves a unified layout while the picker is open.
        lock();
        await tester.pumpAndSettle();
      }
      expect(find.text('New section…'), findsNothing);
      // Details stays open beneath the picker; the actions sheet reopens.
      if (!details) {
        await tester.tap(find.text('Open actions'));
        await tester.pumpAndSettle();
      }
      expect(find.text('Copy channel name'), findsOneWidget);
      expect(find.text('Move to section…'), findsNothing);
    });
  }

  for (final (pick, laterExists) in [
    ('Work', false),
    ('Remove from section', false),
    ('New section…', false),
    ('New section…', true),
  ]) {
    testWidgets('Move to section: $pick files the channel after the actions '
        'sheet closes (a "Later" section already exists: $laterExists)', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(1200, 3000);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final keys = nostr.Keys.generate();
      const relayUrl = 'https://relay.example';
      final storeKey = channelSectionsKey(keys.public, relayUrl);
      // Records every snapshot saved for this layout, so one pick must be
      // exactly one write.
      final saves = _LayoutSaves('flutter.$storeKey', {
        'flutter.$storeKey': jsonEncode(
          ChannelSectionStore(
            sections: [
              const ChannelSection(id: 'work', name: 'Work', order: 0),
              if (laterExists)
                const ChannelSection(id: 'old-later', name: 'Later', order: 1),
            ],
            assignments: {
              if (pick == 'Remove from section') 'channel-id': 'work',
            },
          ).toJson(),
        ),
      });
      final previousStore = SharedPreferencesStorePlatform.instance;
      SharedPreferencesStorePlatform.instance = saves;
      SharedPreferences.resetStatic();
      addTearDown(() {
        SharedPreferencesStorePlatform.instance = previousStore;
        SharedPreferences.resetStatic();
      });
      final prefs = await SharedPreferences.getInstance();
      await tester.pumpWidget(
        _modalApp(
          channel: _channel(),
          loadMembers: () async => const [],
          createChannelActions: (ref) => _FakeChannelActions(ref),
          sectionId: pick == 'Remove from section' ? 'work' : null,
          sectionOverrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            relayConfigProvider.overrideWith(
              () => _SignedInConfig(relayUrl, keys.nsec),
            ),
            activeCommunityProvider.overrideWith((ref) async => null),
          ],
        ),
      );
      final container = ProviderScope.containerOf(
        tester.element(find.text('Open actions')),
      );
      await tester.tap(find.text('Open actions'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Move to section…'));
      await tester.pumpAndSettle();
      expect(find.byType(ChannelActionsSheet), findsNothing);

      await tester.tap(find.text(pick));
      await tester.pumpAndSettle();
      if (pick == 'New section…') {
        await tester.enterText(find.byType(TextField), 'Later');
        await tester.tap(find.text('Create'));
        await tester.pumpAndSettle();
      }

      expect(tester.takeException(), isNull);
      // The provider shows the move at once, and it is saved in one write.
      final shown = container.read(channelSectionsProvider).store;
      final saved = ChannelSectionStore.fromJson(
        jsonDecode(saves.snapshots.single) as Map<String, dynamic>,
      );
      expect(prefs.getString(storeKey), saves.snapshots.single);
      for (final store in [shown, saved]) {
        final filed = store.assignments['channel-id'];
        switch (pick) {
          case 'Work':
            expect(filed, 'work');
          case 'Remove from section':
            expect(store.assignments, isNot(contains('channel-id')));
          default:
            final created = store.sections.singleWhere(
              (s) => s.name == 'Later' && s.id != 'old-later',
            );
            expect(filed, created.id);
        }
      }
    });
  }

  for (final (pick, newKey, beforeDialog) in [
    ('Work', true, false),
    ('Work', false, false),
    ('New section…', true, false),
    ('New section…', false, false),
    // The switch lands after the tap but before the name dialog's first frame.
    ('New section…', true, true),
    ('New section…', false, true),
  ]) {
    testWidgets('Move to section: $pick closes and files nothing once the '
        'community changes behind it (new signing key: $newKey, before the '
        'dialog builds: $beforeDialog)', (tester) async {
      tester.view.physicalSize = const Size(1200, 3000);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final keys = nostr.Keys.generate();
      final other = nostr.Keys.generate();
      const relayUrl = 'https://relay.example';
      const otherRelayUrl = 'https://other.example';
      final mine = channelSectionsKey(keys.public, relayUrl);
      // A portable identity keeps its key in another community's relay.
      final theirs = newKey
          ? channelSectionsKey(other.public, relayUrl)
          : channelSectionsKey(keys.public, otherRelayUrl);
      final workOnly = jsonEncode(
        const ChannelSectionStore(
          sections: [ChannelSection(id: 'work', name: 'Work', order: 0)],
          assignments: {},
        ).toJson(),
      );
      SharedPreferences.setMockInitialValues({
        mine: workOnly,
        theirs: workOnly,
      });
      final prefs = await SharedPreferences.getInstance();
      await tester.pumpWidget(
        _modalApp(
          channel: _channel(),
          loadMembers: () async => const [],
          createChannelActions: (ref) => _FakeChannelActions(ref),
          sectionOverrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            relayConfigProvider.overrideWith(
              () => _SignedInConfig(relayUrl, keys.nsec),
            ),
            activeCommunityProvider.overrideWith((ref) async => null),
          ],
        ),
      );
      final container = ProviderScope.containerOf(
        tester.element(find.text('Open actions')),
      );
      await tester.tap(find.text('Open actions'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Move to section…'));
      await tester.pumpAndSettle();
      final config =
          container.read(relayConfigProvider.notifier) as _SignedInConfig;
      void switchToThird() => config.signInAs(
        nostr.Keys.generate().nsec,
        baseUrl: 'https://third.example',
      );
      // A second switch follows, so a stale route is asked to close more
      // than once. Before the dialog builds, it lands just after the first
      // frame builds the dialog.
      void switchCommunity() {
        config.signInAs(
          newKey ? other.nsec : keys.nsec,
          baseUrl: newKey ? relayUrl : otherRelayUrl,
        );
        if (beforeDialog) {
          tester.binding.addPostFrameCallback((_) => switchToThird());
        } else {
          switchToThird();
        }
      }

      if (beforeDialog) {
        await tester.tap(find.text(pick));
      } else if (pick == 'New section…') {
        await tester.tap(find.text(pick));
        await tester.pumpAndSettle();
        await tester.enterText(find.byType(TextField), 'Later');
      }
      switchCommunity();
      await tester.pumpAndSettle();

      // The stale picker or name dialog closes rather than offering a pick
      // that would be dropped.
      expect(find.text(pick), findsNothing);
      expect(find.text('Create'), findsNothing);
      // Only the stale route closes; the page under it stays.
      expect(find.text('Open actions'), findsOneWidget);

      expect(tester.takeException(), isNull);
      expect(
        container.read(channelSectionsProvider).store.assignments,
        isEmpty,
      );
      expect(prefs.getString(mine), workOnly);
      expect(prefs.getString(theirs), workOnly);
    });
  }

  testWidgets('member sees neither owner action', (tester) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        loadMembers: () async => [
          ChannelMember(
            pubkey: _currentPubkey,
            role: 'member',
            joinedAt: DateTime(2025),
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Archive channel'), findsNothing);
    expect(find.text('Delete channel'), findsNothing);
    expect(find.text('Move to section…'), findsOneWidget);
    expect(find.text('Leave channel'), findsOneWidget);
  });

  testWidgets('shows loading and unavailable capability states', (
    tester,
  ) async {
    final pending = Completer<List<ChannelMember>>();
    await tester.pumpWidget(
      _app(channel: _channel(), loadMembers: () => pending.future),
    );
    await tester.pump();
    expect(find.text('Loading channel actions…'), findsOneWidget);

    pending.completeError(Exception('relay unavailable'));
    await tester.pumpAndSettle();
    expect(find.text('Channel actions unavailable'), findsOneWidget);
  });

  testWidgets('Manage contains editing and canvas, not mute or leave', (
    tester,
  ) async {
    await tester.pumpWidget(
      _modalApp(
        channel: _channel(),
        loadMembers: () async => const [],
        createChannelActions: (ref) => _FakeChannelActions(ref),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Open actions'));
    await tester.pumpAndSettle();

    await tester.tap(find.text('Manage channel'));
    await tester.pumpAndSettle();

    final manageSheet = find.byType(ManageChannelSheet);
    expect(manageSheet, findsOneWidget);
    expect(
      find.descendant(
        of: manageSheet,
        matching: find.byKey(const ValueKey('manage-channel-name')),
      ),
      findsOneWidget,
    );
    expect(
      find.descendant(of: manageSheet, matching: find.text('Add Canvas')),
      findsOneWidget,
    );
    expect(
      find.descendant(of: manageSheet, matching: find.text('Mute channel')),
      findsNothing,
    );
    expect(
      find.descendant(of: manageSheet, matching: find.text('Leave channel')),
      findsNothing,
    );
    expect(
      find.descendant(of: manageSheet, matching: find.text('Topic')),
      findsNothing,
    );
    expect(
      find.descendant(of: manageSheet, matching: find.text('Purpose')),
      findsNothing,
    );
  });

  testWidgets('Manage refreshes metadata after saving from actions', (
    tester,
  ) async {
    await tester.pumpWidget(
      _modalApp(
        channel: _channel(),
        loadMembers: () async => [
          ChannelMember(
            pubkey: _currentPubkey,
            role: 'owner',
            joinedAt: DateTime(2025),
          ),
        ],
        createChannelActions: (ref) => _FakeChannelActions(
          ref,
          onUpdateChannel: (channelId, name, description) async {},
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Open actions'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Manage channel'));
    await tester.pumpAndSettle();

    expect(
      tester.getSize(find.byType(BottomSheet).last).height,
      greaterThan(480),
    );
    await tester.enterText(
      find.byKey(const ValueKey('manage-channel-name')),
      'renamed',
    );
    await tester.pump();
    await tester.tap(find.byKey(const ValueKey('manage-channel-save-details')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Manage channel'));
    await tester.pumpAndSettle();
    final nameField = tester.widget<TextField>(
      find.byKey(const ValueKey('manage-channel-name')),
    );
    expect(nameField.controller?.text, 'renamed');
  });

  testWidgets(
    'native channel editor shares the profile form and retains failed drafts',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      const bridge = MethodChannel('buzz/profile_text_editor');
      final presentations = <Map<Object?, Object?>>[];
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(bridge, (call) async {
            presentations.add(
              Map<Object?, Object?>.from(call.arguments as Map),
            );
            // Cancel instead of looping if a failed save keeps re-presenting.
            if (call.method != 'presentChannel' || presentations.length > 2) {
              return {'action': 'cancel'};
            }
            return {
              'action': 'save',
              'name': 'renamed',
              'description': 'Updated description',
            };
          });
      addTearDown(
        () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
            .setMockMethodCallHandler(bridge, null),
      );
      final updates = <(String?, String?)>[];
      await tester.pumpWidget(
        _modalApp(
          channel: _channel(),
          loadMembers: () async => [
            ChannelMember(
              pubkey: _currentPubkey,
              role: 'owner',
              joinedAt: DateTime(2025),
            ),
          ],
          // Record rather than assert here: the sheet catches update errors.
          createChannelActions: (ref) => _FakeChannelActions(
            ref,
            onUpdateChannel: (_, name, description) async {
              updates.add((name, description));
              if (updates.length == 1) throw Exception('offline');
            },
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Open actions'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Manage channel'));
      await tester.pumpAndSettle();
      expect(updates, [
        ('renamed', 'Updated description'),
        ('renamed', 'Updated description'),
      ]);
      expect(presentations, hasLength(2));
      expect(presentations.first['title'], 'Edit channel');
      expect(presentations.first['multiline'], false);
      expect(presentations.last['initialValue'], 'renamed');
      expect(presentations.last['allowUnchangedSubmission'], true);
      expect(presentations.last['description'], 'Updated description');
      expect(find.byType(ManageChannelSheet), findsNothing);
      debugDefaultTargetPlatformOverride = null;
    },
  );

  testWidgets('native metadata editor opens before a cold canvas finishes', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final canvas = Completer<ChannelCanvas>();
    const bridge = MethodChannel('buzz/profile_text_editor');
    final opened = <(bool, Object?)>[];
    var saved = false;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(bridge, (call) async {
          opened.add((
            canvas.isCompleted,
            (call.arguments as Map)['canvasLoaded'],
          ));
          if (opened.length > 1) return null;
          return {
            'action': 'save',
            'name': 'renamed',
            'description': 'New description',
          };
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(bridge, null),
    );
    await tester.pumpWidget(
      _modalApp(
        channel: _channel(),
        pendingCanvas: canvas.future,
        loadMembers: () async => [
          ChannelMember(
            pubkey: _currentPubkey,
            role: 'owner',
            joinedAt: DateTime(2025),
          ),
        ],
        createChannelActions: (ref) => _FakeChannelActions(
          ref,
          onUpdateChannel: (_, name, description) async {
            saved = true;
          },
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Open actions'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Manage channel'));
    await tester.pump();
    // Presented once, before the canvas resolved, and marked not loaded.
    expect(opened, [(false, false)]);
    expect(saved, isTrue);
    canvas.complete(
      const ChannelCanvas(content: null, updatedAt: null, authorPubkey: null),
    );
    await tester.pumpAndSettle();
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('native canvas round trip retains both channel drafts', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    const bridge = MethodChannel('buzz/profile_text_editor');
    final canvasPresentations = <Map<Object?, Object?>>[];
    final channelPresentations = <Map<Object?, Object?>>[];
    final updates = <(String?, String?)>[];
    final canvases = <String>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(bridge, (call) async {
          final args = Map<Object?, Object?>.from(call.arguments as Map);
          if (call.method == 'present') {
            canvasPresentations.add(args);
            return 'New canvas';
          }
          channelPresentations.add(args);
          // Cancel instead of looping if a failed save keeps re-presenting.
          if (channelPresentations.length > 2) return {'action': 'cancel'};
          return {
            'action': channelPresentations.length == 1 ? 'canvas' : 'save',
            'name': 'Draft name',
            'description': 'Draft description',
          };
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(bridge, null),
    );
    await tester.pumpWidget(
      _modalApp(
        channel: _channel(),
        loadMembers: () async => [
          ChannelMember(
            pubkey: _currentPubkey,
            role: 'owner',
            joinedAt: DateTime(2025),
          ),
        ],
        createChannelActions: (ref) => _FakeChannelActions(
          ref,
          // Record rather than assert here: the sheet catches save errors.
          onUpdateChannel: (_, name, description) async =>
              updates.add((name, description)),
          onSetCanvas: (content) async => canvases.add(content),
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Open actions'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Manage channel'));
    await tester.pumpAndSettle();
    expect(canvasPresentations, [containsPair('title', 'Canvas')]);
    expect(canvasPresentations.single['multiline'], true);
    expect(channelPresentations, hasLength(2));
    expect(channelPresentations.last, {
      ...channelPresentations.last,
      'canvasContent': 'New canvas',
      'canvasLoaded': true,
      'initialValue': 'Draft name',
      'description': 'Draft description',
      'originalName': 'general',
      'originalDescription': '',
    });
    expect(updates, [('Draft name', 'Draft description')]);
    expect(canvases, ['New canvas']);
    expect(find.byType(ManageChannelSheet), findsNothing);
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets(
    'canvas preview opens its content and removal retries without losing drafts',
    (tester) async {
      final removals = <String>[];
      await tester.pumpWidget(
        _modalApp(
          channel: _channel(),
          canvasContent: 'Shared team notes',
          loadMembers: () async => [
            ChannelMember(
              pubkey: _currentPubkey,
              role: 'owner',
              joinedAt: DateTime(2025),
            ),
          ],
          createChannelActions: (ref) => _FakeChannelActions(
            ref,
            onSetCanvas: (content) async {
              removals.add(content);
              if (removals.length == 1) throw Exception('offline');
            },
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Open actions'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Manage channel'));
      await tester.pumpAndSettle();
      expect(find.text('Shared team notes'), findsOneWidget);
      expect(find.text('Add Canvas'), findsNothing);
      expect(find.text('Edit canvas'), findsNothing);
      await tester.enterText(
        find.byKey(const ValueKey('manage-channel-name')),
        'Draft name',
      );
      await tester.tap(find.byKey(const ValueKey('manage-channel-canvas')));
      await tester.pumpAndSettle();
      expect(
        tester
            .widget<TextField>(find.byKey(const ValueKey('text-editor-input')))
            .controller!
            .text,
        'Shared team notes',
      );
      Navigator.of(
        tester.element(find.byKey(const ValueKey('text-editor-input'))),
      ).pop();
      await tester.pumpAndSettle();
      for (var attempt = 0; attempt < 2; attempt++) {
        await tester.ensureVisible(find.text('Remove Canvas'));
        await tester.tap(find.text('Remove Canvas'));
        await tester.pumpAndSettle();
        expect(removals, List.filled(attempt, ''));
        await tester.tap(find.text('Remove'));
        await tester.pumpAndSettle();
        if (attempt == 0) {
          expect(find.text('Shared team notes'), findsOneWidget);
          expect(
            find.text("Couldn't remove the canvas. Try again."),
            findsOneWidget,
          );
        }
      }
      expect(find.text('Add Canvas'), findsOneWidget);
      expect(find.text('Remove Canvas'), findsNothing);
      expect(
        tester
            .widget<TextField>(
              find.byKey(const ValueKey('manage-channel-name')),
            )
            .controller!
            .text,
        'Draft name',
      );
    },
  );

  testWidgets(
    'native removal retains canvas on failure and clears after retry',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      const bridge = MethodChannel('buzz/profile_text_editor');
      final presentations = <(String, Map<Object?, Object?>)>[];
      final attempts = <String>[];
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(bridge, (call) async {
            presentations.add((
              call.method,
              Map<Object?, Object?>.from(call.arguments as Map),
            ));
            if (presentations.length >= 3) return null;
            return {
              'action': 'removeCanvas',
              'name': 'Draft name',
              'description': 'Draft description',
            };
          });
      addTearDown(
        () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
            .setMockMethodCallHandler(bridge, null),
      );
      await tester.pumpWidget(
        _modalApp(
          channel: _channel(),
          canvasContent: 'Existing canvas',
          loadMembers: () async => [],
          createChannelActions: (ref) => _FakeChannelActions(
            ref,
            onSetCanvas: (content) async {
              attempts.add(content);
              if (attempts.length == 1) throw Exception('offline');
            },
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Open actions'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Manage channel'));
      await tester.pumpAndSettle();
      expect(presentations.map((p) => p.$1), List.filled(3, 'presentChannel'));
      expect(presentations.map((p) => p.$2['canvasLoaded']), [
        true,
        true,
        true,
      ]);
      expect(presentations.map((p) => p.$2['canvasContent']), [
        'Existing canvas',
        'Existing canvas',
        '',
      ]);
      for (final (_, args) in presentations.skip(1)) {
        expect(args['initialValue'], 'Draft name');
        expect(args['description'], 'Draft description');
      }
      expect(attempts, ['', '']);
      debugDefaultTargetPlatformOverride = null;
    },
  );

  testWidgets('non-member cannot edit canvas from Manage channel', (
    tester,
  ) async {
    await tester.pumpWidget(
      _modalApp(
        channel: _channel(isMember: false),
        loadMembers: () async => const [],
        createChannelActions: (ref) => _FakeChannelActions(ref),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Open actions'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Manage channel'));
    await tester.pumpAndSettle();

    final editCanvas = find.byKey(const ValueKey('manage-channel-canvas'));
    expect(editCanvas, findsOneWidget);
    expect(tester.widget<FilledButton>(editCanvas).onPressed, isNull);
  });

  testWidgets('regular members can move channels but cannot edit metadata', (
    tester,
  ) async {
    await tester.pumpWidget(
      _modalApp(
        channel: _channel(),
        loadMembers: () async => [
          ChannelMember(
            pubkey: _currentPubkey,
            role: 'member',
            joinedAt: DateTime(2025),
          ),
        ],
        createChannelActions: (ref) => _FakeChannelActions(ref),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Open actions'));
    await tester.pumpAndSettle();

    expect(find.text('Move to section…'), findsOneWidget);
    await tester.tap(find.text('Manage channel'));
    await tester.pumpAndSettle();

    final nameField = find.byKey(const ValueKey('manage-channel-name'));
    final descriptionField = find.byKey(
      const ValueKey('manage-channel-description'),
    );
    expect(tester.widget<TextField>(nameField).enabled, isFalse);
    expect(tester.widget<TextField>(descriptionField).enabled, isFalse);
    expect(
      tester
          .widget<FilledButton>(
            find.byKey(const ValueKey('manage-channel-canvas')),
          )
          .onPressed,
      isNotNull,
    );
  });

  testWidgets('DM omits quick actions, then shows mute and copy rows', (
    tester,
  ) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(type: 'dm'),
        loadMembers: () async => const [],
      ),
    );
    await tester.pumpAndSettle();

    for (final label in [
      'Mute channel',
      'Copy channel name',
      'Copy channel ID',
    ]) {
      expect(find.text(label), findsOneWidget, reason: label);
    }
    for (final label in [
      'Star',
      'Unstar',
      'Mark Unread',
      'Mark Read',
      'Move to section…',
      'Manage channel',
      'Leave channel',
      'Archive channel',
      'Delete channel',
    ]) {
      expect(find.text(label), findsNothing, reason: label);
    }

    final muteTop = tester.getTopLeft(find.text('Mute channel')).dy;
    final copyNameTop = tester.getTopLeft(find.text('Copy channel name')).dy;
    final copyIdTop = tester.getTopLeft(find.text('Copy channel ID')).dy;
    expect(muteTop, lessThan(copyNameTop));
    expect(copyNameTop, lessThan(copyIdTop));
  });
}

class _LayoutSaves extends InMemorySharedPreferencesStore {
  _LayoutSaves(this.key, Map<String, Object> data) : super.withData(data);

  final String key;
  final snapshots = <String>[];

  @override
  Future<bool> setValue(String valueType, String key, Object value) {
    if (key == this.key) snapshots.add(value as String);
    return super.setValue(valueType, key, value);
  }
}

class _SignedInConfig extends RelayConfigNotifier {
  _SignedInConfig(this.baseUrl, this.nsec);
  final String baseUrl;
  final String nsec;

  @override
  RelayConfig build() => RelayConfig(baseUrl: baseUrl, nsec: nsec);

  void signInAs(String other, {required String baseUrl}) =>
      state = RelayConfig(baseUrl: baseUrl, nsec: other);
}

class _FakeChannelSectionsNotifier extends ChannelSectionsNotifier {
  _FakeChannelSectionsNotifier(this._store);

  final ChannelSectionStore _store;

  @override
  ChannelSectionsState build() =>
      ChannelSectionsState(isReady: true, store: _store, version: 1);

  void lock() => state = ChannelSectionsState(
    isReady: true,
    store: _store,
    workspace: WorkspaceRecord.fromJson({
      'version': 1,
      'sections': <Object>[],
      'assignments': <String, Object>{},
      'sectionOrder': <String>[],
    }),
    version: 2,
  );
}

class _FakeChannelActions extends ChannelActions {
  final Future<void> Function(
    String channelId,
    String? name,
    String? description,
  )?
  onUpdateChannel;

  final Future<void> Function(String)? onSetCanvas;

  _FakeChannelActions(Ref ref, {this.onUpdateChannel, this.onSetCanvas})
    : super(
        ref: ref,
        session: ref.read(relaySessionProvider.notifier),
        signedEventRelay: SignedEventRelay(
          session: ref.read(relaySessionProvider.notifier),
          nsec: null,
        ),
        currentPubkey: _currentPubkey,
      );

  @override
  Future<void> updateChannel({
    required String channelId,
    String? name,
    String? description,
  }) async {
    await onUpdateChannel?.call(channelId, name, description);
  }

  @override
  Future<void> setCanvas({
    required String channelId,
    required String content,
  }) async {
    await onSetCanvas?.call(content);
  }

  String? unarchivedChannelId;

  @override
  Future<void> leaveChannel(
    String channelId, {
    Future<bool> Function()? mayPublish,
  }) async {}

  @override
  Future<void> archiveChannel(String channelId) async {}

  @override
  Future<void> unarchiveChannel(String channelId) async {
    unarchivedChannelId = channelId;
  }
}

/// Lists the sheet's channel with whatever [latest] holds when asked.
class _KnownTimesChannels extends ChannelsNotifier {
  int? latest;

  @override
  Future<List<Channel>> build() async => const [];

  @override
  Channel? channelById(String channelId) => Channel(
    id: channelId,
    name: 'general',
    channelType: 'stream',
    visibility: 'open',
    description: '',
    createdBy: 'owner',
    createdAt: DateTime(2025),
    memberCount: 2,
    isMember: true,
    lastMessageAt: latest == null
        ? null
        : DateTime.fromMillisecondsSinceEpoch(latest! * 1000, isUtc: true),
  );

  @override
  void clearObservedUnreadCoveredByRead(String channelId, int readAt) {}
}

class _RecordingReadState extends ReadStateNotifier {
  final marks = <(String, int, bool)>[];

  @override
  ReadStateState build() => const ReadStateState.inert();

  @override
  void markContextRead(
    String contextId,
    int unixTimestamp, {
    bool clearForcedMessages = false,
    bool explicit = false,
  }) => marks.add((contextId, unixTimestamp, clearForcedMessages));
}
