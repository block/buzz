import 'dart:convert';
import 'package:image_picker/image_picker.dart';
import 'package:buzz/features/home/home_screen_shortcut_host.dart';
import 'package:buzz/shared/shortcuts/home_screen_shortcuts.dart';
import 'package:buzz/features/channels/compose_bar.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'dart:async';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:buzz/shared/me/me_chat_creation.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/me/me_assistants.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/me/me_page.dart';
import 'package:buzz/features/home/home_page.dart';
import 'package:buzz/shared/widgets/buzz_loading_indicator.dart';
import 'package:buzz/shared/me/me_groups.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/app_list.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';

class _CachedProfiles extends UserCacheNotifier {
  _CachedProfiles(this.profiles);
  final Map<String, UserProfile> profiles;
  @override
  Map<String, UserProfile> build() => profiles;
}

class _HeldUpload extends MediaUploadService {
  _HeldUpload()
    : super(
        baseUrl: 'https://relay.test',
        nsec: null,
        pickGalleryImage: () async => null,
        pickGalleryVideo: () async => null,
      );
  final result = Completer<BlobDescriptor>();
  var calls = 0;
  @override
  Future<BlobDescriptor> uploadImage(
    XFile image, {
    ValueChanged<double>? onProgress,
    UploadCancellationToken? cancellationToken,
  }) {
    calls++;
    return result.future;
  }
}

class _UnavailableRelay extends RelaySessionNotifier {
  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async => throw StateError('Agent unavailable');
  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [];
}

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
    for (final (id, name, owner, member, archived) in [
      ('filed', 'Filed labeled chat', 'owner', true, false),
      ('loose', 'Unfiled labeled chat', 'owner', true, false),
      ('shared', 'Someone else chat', 'other', true, false),
      ('agent-created', 'Agent-created chat', 'agent', true, false),
      ('hidden', 'Inaccessible labeled chat', 'owner', false, false),
      ('archived', 'Archived labeled chat', 'other', true, true),
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
        archivedAt: archived ? DateTime(2026) : null,
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
          defaultAgentProvider.overrideWith((ref) => Stream.value(null)),
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
    expect(find.text('Someone else chat'), findsOneWidget);
    expect(find.text('Agent-created chat'), findsOneWidget);
    expect(find.text('Inaccessible labeled chat'), findsNothing);
    expect(find.text('Archived labeled chat'), findsNothing);
    expect(find.text('A place to work with your agents'), findsNothing);
  });

  testWidgets('long-pressing a chat opens its details as a sheet', (
    tester,
  ) async {
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          defaultAgentProvider.overrideWith((ref) => Stream.value(null)),
          myPubkeyProvider.overrideWithValue('owner'),
          currentPubkeyProvider.overrideWith((ref) => 'owner'),
          channelsProvider.overrideWith(LabeledChannels.new),
          channelMembersProvider('loose').overrideWith(
            (ref) async => [
              ChannelMember(
                pubkey: 'owner',
                role: 'owner',
                joinedAt: DateTime(2026),
              ),
            ],
          ),
          agentOwnersProvider.overrideWithValue(
            const AsyncValue.data(<String, String>{}),
          ),
          meGroupsProvider.overrideWith(
            (ref) => Stream.value(const MeGroups(groups: [], assignments: {})),
          ),
        ],
        child: MaterialApp(theme: AppTheme.light(), home: const MePage()),
      ),
    );
    await tester.pumpAndSettle();

    await tester.longPress(find.text('Unfiled labeled chat'));
    await tester.pumpAndSettle();

    expect(find.byType(BottomSheet), findsOneWidget);
    expect(
      find.descendant(
        of: find.byType(BottomSheet),
        matching: find.byKey(const ValueKey('channel-details-page-list')),
      ),
      findsOneWidget,
    );
    expect(
      find.descendant(
        of: find.byType(BottomSheet),
        matching: find.text('Unfiled labeled chat'),
      ),
      findsOneWidget,
    );

    await tester.drag(
      find.byKey(const ValueKey('channel-details-page-list')),
      const Offset(0, -600),
    );
    await tester.pumpAndSettle();
    expect(find.text('Leave channel'), findsOneWidget);
    // Solo chats move between Solo workspaces, not channel sections.
    expect(find.text('Move to section…'), findsNothing);
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
          defaultAgentProvider.overrideWith((ref) => Stream.value(null)),
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
            defaultAgentProvider.overrideWith((ref) => Stream.value(null)),
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
      expect(find.byIcon(BuzzIcons.workspace), findsOneWidget);
      final header = tester.widget<FrostedAppBar>(find.byType(FrostedAppBar));
      expect(header.nativeLargeTitle, isTrue);
      expect(header.nativeActions!.map((action) => action.icon), [
        BuzzIcons.workspace,
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
      expect(find.text('No chats in this workspace yet'), findsOneWidget);
      expect(find.text('Design agent'), findsOneWidget);
      await tester.tap(find.text('Chats'));
      await tester.pumpAndSettle();
      expect(find.text('Design agent'), findsNothing);
      expect(find.text('No chats in this workspace yet'), findsOneWidget);
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
    'new chat opens a conversation with Bestie and workspace controls',
    (tester) async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final action = ValueNotifier<VoidCallback?>(null);
      addTearDown(action.dispose);
      final bestie = StreamController<String?>();
      addTearDown(bestie.close);
      final sync = MeSync(
        _UnavailableRelay(),
        RelayConfig(
          baseUrl: 'https://relay.test',
          nsec: nostr.Keys.generate().nsec,
        ),
        prefs: prefs,
      );
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            defaultAgentProvider.overrideWith((ref) => bestie.stream),
            meSyncProvider.overrideWithValue(sync),
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
            channelsProvider.overrideWith(FakeChannels.new),
            meAssistantsProvider.overrideWith(
              (ref) async => const [
                AgentDirectoryEntry(pubkey: 'bestie', displayName: 'My Bestie'),
                AgentDirectoryEntry(
                  pubkey: 'other',
                  displayName: 'Other agent',
                ),
              ],
            ),
            agentDirectoryProvider.overrideWith((ref) async => []),
            agentOwnersProvider.overrideWith((ref) async => {}),
            customEmojiListProvider.overrideWithValue([]),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: MePage(newChatAction: action),
          ),
        ),
      );
      await tester.pumpAndSettle();
      action.value!();
      await tester.pumpAndSettle();
      expect(find.byType(ComposeBar), findsOneWidget);
      expect(find.byType(BottomSheet), findsNothing);
      // Missing agent fails before the next frame and must unlock the header.
      await tester.tap(find.text('Message Choose an agent'));
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField), 'Hello');
      await tester.tap(find.byIcon(BuzzIcons.arrowUp).hitTestable().last);
      await tester.pumpAndSettle();
      expect(
        tester
            .widget<IconButton>(find.byKey(const ValueKey('new-chat-agent')))
            .onPressed,
        isNotNull,
      );
      ScaffoldMessenger.of(
        tester.element(find.byType(ComposeBar)),
      ).clearSnackBars();
      await tester.pumpAndSettle();
      bestie.add('bestie');
      await tester.pumpAndSettle();
      expect(find.byTooltip('Change agent: My Bestie'), findsOneWidget);
      bestie.addError(StateError('Cache write failed'));
      await tester.pumpAndSettle();
      expect(find.byTooltip('Change agent: My Bestie'), findsOneWidget);
      final bar = tester.widget<FrostedAppBar>(find.byType(FrostedAppBar).last);
      expect(bar.nativeActions!.map((action) => action.label), [
        'Choose workspace',
        'Change agent: My Bestie',
      ]);
      expect(bar.nativeLeading?.symbol, 'chevron.backward');
      expect(
        bar.nativeActions!.every((action) => !action.sharesBackground),
        isTrue,
      );
      await tester.tap(find.byTooltip('Choose workspace'));
      await tester.pumpAndSettle();
      expect(find.byType(BottomSheet), findsNothing);
      expect(
        find.widgetWithText(CheckedPopupMenuItem<String>, 'No Workspace'),
        findsOneWidget,
      );
      expect(find.text('Create workspace'), findsNothing);
      await tester.tap(find.text('Mobile app').last);
      await tester.pumpAndSettle();
      expect(find.byTooltip('Workspace: Mobile app'), findsOneWidget);
      await tester.tap(find.byTooltip('Change agent: My Bestie'));
      await tester.pumpAndSettle();
      expect(tester.getSize(find.byType(ListView).last).height, lessThan(300));
      await tester.tap(find.text('Other agent'));
      await tester.pumpAndSettle();
      bestie.add(null);
      await tester.pumpAndSettle();
      expect(find.byTooltip('Change agent: Other agent'), findsOneWidget);
      await tester.tap(find.text('Hello').last);
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField), 'Plan our launch');
      await tester.pageBack();
      await tester.pumpAndSettle();
      action.value!();
      await tester.pumpAndSettle();
      expect(find.text('Plan our launch'), findsOneWidget);
      expect(MeChatCreation.pending(sync), isNull);
      await tester.tap(find.byTooltip('Choose an agent'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Other agent'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Plan our launch').last);
      await tester.pumpAndSettle();
      await tester.tap(find.byIcon(BuzzIcons.arrowUp).hitTestable().last);
      await tester.pumpAndSettle();
      expect(MeChatCreation.pending(sync), isNotNull);
      expect(find.text('Retry'), findsOneWidget);
      ScaffoldMessenger.of(tester.element(find.text('Retry'))).clearSnackBars();
      await tester.pumpAndSettle();
      await tester.tap(find.text('Retry'));
      await tester.pumpAndSettle();
      await tester.pageBack();
      await tester.pumpAndSettle();
      expect(find.text('Retry'), findsNothing);
      expect(find.byType(ComposeBar), findsNothing);

      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pumpAndSettle();
    },
  );

  testWidgets(
    'an app icon shortcut leaves a first message that is still sending',
    (tester) async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final action = ValueNotifier<VoidCallback?>(null);
      final dmAction = ValueNotifier<VoidCallback?>(() {});
      final upload = _HeldUpload();
      final sync = MeSync(
        _UnavailableRelay(),
        RelayConfig(
          baseUrl: 'https://relay.test',
          nsec: nostr.Keys.generate().nsec,
        ),
        prefs: prefs,
      );
      addTearDown(() {
        action.dispose();
        dmAction.dispose();
        upload.dispose();
        pendingHomeScreenShortcut.value = null;
      });
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            defaultAgentProvider.overrideWith((ref) => Stream.value(null)),
            meSyncProvider.overrideWithValue(sync),
            meGroupsProvider.overrideWith(
              (ref) => Stream.value(const MeGroups()),
            ),
            channelsProvider.overrideWith(FakeChannels.new),
            meAssistantsProvider.overrideWith(
              (ref) async => const [
                AgentDirectoryEntry(pubkey: 'agent', displayName: 'Pip'),
              ],
            ),
            agentDirectoryProvider.overrideWith((ref) async => []),
            agentOwnersProvider.overrideWith((ref) async => {}),
            customEmojiListProvider.overrideWithValue([]),
            mediaUploadServiceProvider.overrideWithValue(upload),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Stack(
              children: [
                MePage(newChatAction: action),
                HomeScreenShortcutHost(
                  newChat: action,
                  newDm: dmAction,
                  onAgents: (_) {},
                  openBestieConversation: (_, _, _) async {},
                ),
              ],
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      action.value!();
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Choose an agent'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Pip'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Message Pip'));
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField), 'Keep this first message');
      final field = tester.widget<TextField>(find.byType(TextField));
      field.contentInsertionConfiguration!.onContentInserted(
        KeyboardInsertedContent(
          mimeType: 'image/png',
          uri: 'content://clipboard/tiny.png',
          data: base64Decode(
            'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLttAAAAABJRU5ErkJggg==',
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.byTooltip('Remove attachment'), findsOneWidget);
      await tester.tap(find.byIcon(BuzzIcons.arrowUp).hitTestable().last);
      await tester.pump();
      await tester.pump(const Duration(seconds: 1));
      expect(upload.calls, 1);
      expect(MeChatCreation.pending(sync), isNull);

      // The draft refuses to close while its first message sends, so the
      // shortcut is dropped instead of discarding it.
      pendingHomeScreenShortcut.value = HomeScreenShortcut.newDm;
      // The upload's progress keeps animating, so pump frames, not settle.
      await tester.pump();
      await tester.pump(const Duration(seconds: 1));
      expect(find.byType(ComposeBar), findsOneWidget);
      expect(pendingHomeScreenShortcut.value, isNull);
      // A failed upload restores the message into the draft that kept it.
      upload.result.completeError(Exception('offline'));
      await tester.pumpAndSettle();
      expect(find.text('Keep this first message'), findsOneWidget);
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets('new chat tells same-name agents apart in the picker and draft', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final action = ValueNotifier<VoidCallback?>(null);
    addTearDown(action.dispose);
    final sync = MeSync(
      _UnavailableRelay(),
      RelayConfig(
        baseUrl: 'https://relay.test',
        nsec: nostr.Keys.generate().nsec,
      ),
      prefs: prefs,
    );
    var live = ['a1' * 32, 'b2' * 32];
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          defaultAgentProvider.overrideWith((ref) => Stream.value(null)),
          meSyncProvider.overrideWithValue(sync),
          meGroupsProvider.overrideWith(
            (ref) => Stream.value(const MeGroups()),
          ),
          channelsProvider.overrideWith(FakeChannels.new),
          meAssistantsProvider.overrideWith(
            (ref) async => [
              for (final key in live)
                AgentDirectoryEntry(pubkey: key, displayName: 'Atlas'),
            ],
          ),
          agentDirectoryProvider.overrideWith((ref) async => []),
          agentOwnersProvider.overrideWith((ref) async => {}),
          customEmojiListProvider.overrideWithValue([]),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: MePage(newChatAction: action),
        ),
      ),
    );
    await tester.pumpAndSettle();
    action.value!();
    await tester.pumpAndSettle();
    List<String> pickerLabels() => [
      for (final row in tester.widgetList<AppListRowRaw>(
        find.byType(AppListRowRaw),
      ))
        (row.title as Text).data!,
    ];

    await tester.tap(find.byTooltip('Choose an agent'));
    await tester.pumpAndSettle();
    final labels = pickerLabels();
    expect(labels.toSet(), hasLength(2), reason: 'Namesakes must differ');
    await tester.tap(find.text(labels.last));
    await tester.pumpAndSettle();
    expect(find.byTooltip('Change agent: ${labels.last}'), findsOneWidget);
    expect(find.text('Message ${labels.last}'), findsOneWidget);

    // Once the namesake is archived the plain name is enough.
    live = [live.last];
    ProviderScope.containerOf(
      tester.element(find.byType(ComposeBar)),
    ).invalidate(meAssistantsProvider);
    await tester.pumpAndSettle();
    expect(find.byTooltip('Change agent: Atlas'), findsOneWidget);
    expect(find.text('Message Atlas'), findsOneWidget);
    await tester.tap(find.byTooltip('Change agent: Atlas'));
    await tester.pumpAndSettle();
    expect(pickerLabels(), ['Atlas']);
  });

  testWidgets('a resumed chat for an archived agent names it apart', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final action = ValueNotifier<VoidCallback?>(null);
    addTearDown(action.dispose);
    final sync = MeSync(
      _UnavailableRelay(),
      RelayConfig(
        baseUrl: 'https://relay.test',
        nsec: nostr.Keys.generate().nsec,
      ),
      prefs: prefs,
    );
    final archived = 'a1' * 32;
    final live = 'b2' * 32;
    await MeChatCreation('same-id', 'Unfinished chat', archived).save(sync);
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          defaultAgentProvider.overrideWith((ref) => Stream.value(null)),
          meSyncProvider.overrideWithValue(sync),
          meGroupsProvider.overrideWith(
            (ref) => Stream.value(const MeGroups()),
          ),
          channelsProvider.overrideWith(FakeChannels.new),
          meAssistantsProvider.overrideWith(
            (ref) async => [
              AgentDirectoryEntry(pubkey: live, displayName: 'Atlas'),
            ],
          ),
          // The archived agent's profile stays in the client's cache.
          userCacheProvider.overrideWith(
            () => _CachedProfiles({
              archived: UserProfile(pubkey: archived, displayName: 'Atlas'),
            }),
          ),
          agentDirectoryProvider.overrideWith((ref) async => []),
          agentOwnersProvider.overrideWith((ref) async => {}),
          customEmojiListProvider.overrideWithValue([]),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: MePage(newChatAction: action),
        ),
      ),
    );
    await tester.pumpAndSettle();
    action.value!();
    await tester.pumpAndSettle();
    await tester.tap(find.text('Resume'));
    await tester.pumpAndSettle();
    final tooltip = tester
        .widgetList<IconButton>(find.byType(IconButton))
        .map((button) => button.tooltip)
        .whereType<String>()
        .singleWhere((label) => label.startsWith('Change agent: '));
    expect(tooltip, startsWith('Change agent: Atlas'));
    expect(tooltip, isNot('Change agent: Atlas'));
  });

  for (final resume in [true, false]) {
    testWidgets(
      'new chat resumes a named creation in the draft: resume=$resume',
      (tester) async {
        debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
        addTearDown(() => debugDefaultTargetPlatformOverride = null);
        const channel = MethodChannel('buzz/profile_text_editor');
        final calls = <MethodCall>[];
        var directoryLoads = 0;
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          channel,
          (call) async {
            calls.add(call);
            return null;
          },
        );
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
              savedPrefsProvider.overrideWithValue(prefs),
              agentDirectoryProvider.overrideWith((ref) async => []),
              agentOwnersProvider.overrideWith((ref) async => {}),
              customEmojiListProvider.overrideWithValue([]),
              defaultAgentProvider.overrideWith((ref) => Stream.value(null)),
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
        expect(calls, isEmpty, reason: 'No native New chat sheet remains');
        expect(find.byType(BottomSheet), findsNothing);
        // A resumed creation finishes with its saved name and agent; a
        // discarded one leaves a fresh draft.
        expect(find.text('Retry'), resume ? findsOneWidget : findsNothing);
        expect(find.byType(ComposeBar), resume ? findsNothing : findsOneWidget);
        expect(
          find.text('Unfinished chat'),
          resume ? findsOneWidget : findsNothing,
        );
        expect(
          MeChatCreation.pending(restarted)?.id,
          resume ? 'same-id' : isNull,
        );
        final agent = tester
            .widget<FrostedAppBar>(find.byType(FrostedAppBar).last)
            .nativeActions!
            .last;
        expect(
          agent.onPressed,
          resume ? isNull : isNotNull,
          reason: 'A resumed creation keeps its saved agent',
        );
        await tester.pumpWidget(const SizedBox.shrink());
        debugDefaultTargetPlatformOverride = null;
      },
    );
  }

  testWidgets('unavailable preferences show retry instead of empty state', (
    tester,
  ) async {
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          defaultAgentProvider.overrideWith((ref) => Stream.value(null)),
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

  for (final failed in [true, false]) {
    testWidgets('shared Solo chats stay listed while preferences '
        '${failed ? 'fail' : 'load'}', (tester) async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final pending = StreamController<MeGroups>();
      addTearDown(() => unawaited(pending.close()));
      await tester.pumpWidget(
        ProviderScope(
          retry: (_, _) => null,
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            myPubkeyProvider.overrideWithValue('viewer'),
            channelsProvider.overrideWith(_SharedAndOrdinary.new),
            meGroupsProvider.overrideWith(
              (ref) => failed
                  ? Stream<MeGroups>.error(StateError('offline'))
                  : pending.stream,
            ),
            defaultAgentProvider.overrideWith((ref) => Stream.value(null)),
            meAssistantsProvider.overrideWith((ref) async => []),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: HomePage(
              settingsPageBuilder: (_) => const SizedBox(),
              onAgents: (_) {},
              defaultAgentBuilder: (_) => const SizedBox(),
              openBestieConversation: (_, _, _) async {},
              mePageBuilder: (_, chat, workspace) => MePage(
                sliver: true,
                newChatAction: chat,
                newWorkspaceAction: workspace,
              ),
            ),
          ),
        ),
      );
      for (var i = 0; i < 10; i++) {
        await tester.pump(const Duration(milliseconds: 100));
      }
      expect(find.text('Normal team conversation'), findsOneWidget);
      expect(find.text('Shared Solo conversation'), findsOneWidget);
      expect(
        find.text('Couldn’t load your chats and projects.'),
        failed ? findsOneWidget : findsNothing,
      );
      expect(
        find.byType(BuzzLoadingIndicator),
        failed ? findsNothing : findsOneWidget,
      );
      expect(find.text('No chats yet'), findsNothing);
      await tester.pumpWidget(const SizedBox());
    });
  }
}

class _SharedAndOrdinary extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [
    for (final shared in [false, true])
      Channel(
        id: shared ? 'shared' : 'ordinary',
        name: shared ? 'Shared Solo conversation' : 'Normal team conversation',
        channelType: 'stream',
        visibility: 'private',
        labels: shared ? const ['buzz.solo'] : const [],
        description: '',
        createdBy: 'other',
        createdAt: DateTime(2026),
        memberCount: 2,
        isMember: true,
      ),
  ];
}
