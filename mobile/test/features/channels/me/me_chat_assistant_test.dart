import 'dart:async';
import 'dart:ui' show SemanticsAction;

import 'package:buzz/features/channels/me/me_chat_assistant.dart';
import 'package:buzz/features/channels/me/me_assistant_picker.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/shared/me/me_assistants.dart';
import 'package:buzz/shared/me/me_agent_change.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/utils/string_utils.dart';
import 'package:buzz/shared/widgets/app_list.dart';
import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

class _Profiles extends UserCacheNotifier {
  @override
  Map<String, UserProfile> build() => {
    // Self-published names differ so tests prove the owner's name wins.
    'a': const UserProfile(pubkey: 'a', displayName: 'Confusable Alpha'),
    'b': const UserProfile(pubkey: 'b', displayName: 'Confusable Beta'),
  };
}

class _CachedProfiles extends UserCacheNotifier {
  _CachedProfiles(this.profiles);
  final Map<String, UserProfile> profiles;
  @override
  Map<String, UserProfile> build() => profiles;
}

class _Harness extends HookConsumerWidget {
  final ValueChanged<List<String>> onSend;
  const _Harness(this.onSend);
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final assistant = useMeChatAssistant(
      context,
      ref,
      enabled: true,
      channelId: 'chat',
      initial: null,
      messages: const [],
    );
    return Scaffold(
      appBar: AppBar(actions: [assistant.action]),
      body: Column(
        children: [
          ?assistant.pendingNotice,
          TextButton(
            onPressed: () async => onSend(
              await meRecipients(ref, 'chat', assistant.selected, const []),
            ),
            child: const Text('Send'),
          ),
        ],
      ),
    );
  }
}

void main() {
  for (final keyFallback in [false, true]) {
    final target = keyFallback ? '0123456789abcdef' * 4 : 'b';
    // An unnamed agent reads as its compact npub, like every other picker.
    final targetName = keyFallback ? shortPubkey(target) : 'Beta';
    for (final action in ['Resume', 'Later', 'Discard choice']) {
      testWidgets(
        'header restores pending agent choice: $action fallback=$keyFallback',
        (tester) async {
          final semantics = tester.ensureSemantics();
          SharedPreferences.setMockInitialValues({});
          final prefs = await SharedPreferences.getInstance();
          final sync = MeSync(
            RelaySessionNotifier(),
            RelayConfig(
              baseUrl: 'https://relay.test',
              nsec: nostr.Nip19.encode(
                prefix: nostr.Nip19Prefix.nsec,
                data: '0' * 63 + '1',
              ),
            ),
            prefs: prefs,
          );
          await prefs.setString(MeAgentChange.selectionKey(sync, 'chat'), 'a');
          await MeAgentChange.create(sync, 'chat', target).save(sync, 'chat');
          await tester.pumpWidget(
            ProviderScope(
              overrides: [
                meSyncProvider.overrideWithValue(sync),
                meAssistantsProvider.overrideWith(
                  (ref) async => [
                    AgentDirectoryEntry(pubkey: 'a', displayName: 'Alpha'),
                    AgentDirectoryEntry(
                      pubkey: target,
                      displayName: keyFallback ? null : 'Beta',
                    ),
                  ],
                ),
                channelMembersProvider('chat').overrideWith(
                  (ref) async => [
                    for (final id in ['a', target])
                      ChannelMember(
                        pubkey: id,
                        role: 'bot',
                        joinedAt: DateTime(2026),
                      ),
                  ],
                ),
                userCacheProvider.overrideWith(_Profiles.new),
              ],
              child: MaterialApp(
                theme: AppTheme.light(),
                home: _Harness((_) {}),
              ),
            ),
          );
          await tester.pumpAndSettle();
          expect(find.text('Finish choosing $targetName'), findsOneWidget);
          expect(
            tester
                .getSemantics(
                  find.widgetWithText(
                    TextButton,
                    'Finish choosing $targetName',
                  ),
                )
                .getSemanticsData()
                .hasAction(SemanticsAction.tap),
            isTrue,
          );
          expect(
            find.byTooltip('Pending agent change: $targetName'),
            findsOneWidget,
          );
          await tester.tap(find.text('Finish choosing $targetName'));
          await tester.pumpAndSettle();
          expect(
            find.textContaining('Your choice of $targetName'),
            findsOneWidget,
          );
          expect(
            find.textContaining('keeps any access already granted'),
            findsOneWidget,
          );
          expect(find.text('Finish changing your agent?'), findsOneWidget);
          await tester.tap(find.text(action));
          await tester.pumpAndSettle();
          expect(
            prefs.getString(MeAgentChange.selectionKey(sync, 'chat')),
            action == 'Resume' ? target : 'a',
          );
          expect(
            MeAgentChange.pending(sync, 'chat') != null,
            action == 'Later',
          );
          expect(
            find.text('Finish choosing $targetName'),
            action == 'Later' ? findsOneWidget : findsNothing,
          );
          semantics.dispose();
        },
      );
    }
  }
  for (final scenario in ['valid', 'revoked', 'removed']) {
    testWidgets('parallel send checks handle $scenario recipients', (
      tester,
    ) async {
      final owned = Completer<bool>();
      final membership = Completer<List<ChannelMember>>();
      var ownershipStarted = false;
      var membershipStarted = false;
      late WidgetRef widgetRef;
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
            meSyncProvider.overrideWithValue(sync),
            meAssistantOwnershipProvider('a').overrideWith((ref) {
              ownershipStarted = true;
              return owned.future;
            }),
            channelMembersProvider('chat').overrideWith((ref) {
              membershipStarted = true;
              return membership.future;
            }),
          ],
          child: Consumer(
            builder: (context, ref, child) {
              widgetRef = ref;
              return const SizedBox.shrink();
            },
          ),
        ),
      );
      final result = meRecipients(widgetRef, 'chat', 'a', const []);
      expect(ownershipStarted, isTrue);
      expect(membershipStarted, isTrue);
      final assertion = expectLater(
        result,
        scenario == 'valid' ? completion(['a']) : throwsStateError,
      );
      owned.complete(scenario != 'revoked');
      membership.complete(
        scenario == 'removed'
            ? []
            : [
                ChannelMember(
                  pubkey: 'a',
                  role: 'bot',
                  joinedAt: DateTime(2026),
                ),
              ],
      );
      await assertion;
    });
  }
  NostrEvent message(String author, List<String> recipients, int time) =>
      NostrEvent(
        id: '$time',
        pubkey: author,
        kind: EventKind.streamMessage,
        createdAt: time,
        content: 'hello',
        tags: [
          for (final p in recipients) ['p', p],
        ],
        sig: '',
      );
  test('last agent inference uses newest-first equal-second ID ordering', () {
    final older = NostrEvent(
      id: 'z',
      pubkey: 'me',
      kind: EventKind.streamMessage,
      createdAt: 2,
      content: '',
      tags: const [
        ['p', 'a'],
      ],
      sig: '',
    );
    final newer = NostrEvent(
      id: 'a',
      pubkey: 'me',
      kind: EventKind.streamMessage,
      createdAt: 2,
      content: '',
      tags: const [
        ['p', 'b'],
      ],
      sig: '',
    );
    for (final input in [
      [older, newer],
      [newer, older],
    ]) {
      expect(
        resolveMeAssistant(
          selected: null,
          available: {'a', 'b'},
          viewer: 'me',
          messages: input,
        ),
        'b',
      );
    }
  });
  test(
    'inherits the sole agent and does not reuse a removed explicit choice',
    () {
      expect(
        resolveMeAssistant(
          selected: null,
          available: {'a'},
          viewer: 'me',
          messages: [],
        ),
        'a',
      );
      expect(
        resolveMeAssistant(
          selected: 'b',
          available: {'a'},
          viewer: 'me',
          messages: [],
        ),
        isNull,
      );
    },
  );
  test(
    'inherits last desktop recipient but leaves ambiguous recipients unset',
    () {
      expect(
        resolveMeAssistant(
          selected: null,
          available: {'a', 'b'},
          viewer: 'me',
          messages: [
            message('me', ['a'], 1),
            message('other', ['a'], 3),
            message('me', ['b'], 2),
          ],
        ),
        'b',
      );
      expect(
        resolveMeAssistant(
          selected: null,
          available: {'a', 'b'},
          viewer: 'me',
          messages: [
            message('me', ['a', 'b'], 2),
            message('me', ['a'], 1),
          ],
        ),
        isNull,
      );
    },
  );
  testWidgets(
    'header avatar inherits agent, routes plain send, and remembers a change',
    (tester) async {
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
      var members = ['a'];
      var directoryReads = 0;
      var ownershipReads = 0;
      List<String>? sent;
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            meSyncProvider.overrideWithValue(sync),
            meAssistantOwnershipProvider.overrideWith((ref, key) async {
              ownershipReads++;
              return key == 'a' || key == 'b';
            }),
            meAssistantsProvider.overrideWith((ref) async {
              directoryReads++;
              return const [
                AgentDirectoryEntry(pubkey: 'a', displayName: 'Alpha'),
                AgentDirectoryEntry(pubkey: 'b', displayName: 'Beta'),
              ];
            }),
            channelMembersProvider('chat').overrideWith(
              (ref) async => [
                for (final id in members)
                  ChannelMember(
                    pubkey: id,
                    role: 'bot',
                    joinedAt: DateTime(2026),
                  ),
              ],
            ),
            userCacheProvider.overrideWith(_Profiles.new),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: _Harness((value) => sent = value),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.byTooltip('Change agent: Alpha'), findsOneWidget);
      expect(find.byType(AvatarImage), findsOneWidget);
      // Without artwork, the fallback initial follows the owner's label.
      expect(
        find.descendant(of: find.byType(AvatarImage), matching: find.text('A')),
        findsOneWidget,
      );
      expect(find.text('Choose follow-up assistant'), findsNothing);
      final readsBeforeSend = directoryReads;
      await tester.tap(find.text('Send'));
      await tester.pumpAndSettle();
      expect(sent, ['a']);
      expect(directoryReads, readsBeforeSend);
      expect(ownershipReads, 1);
      final readsBeforeOpening = directoryReads;
      await tester.tap(find.byTooltip('Change agent: Alpha'));
      await tester.pumpAndSettle();
      expect(directoryReads, readsBeforeOpening);
      expect(find.text('Agents · 2'), findsOneWidget);
      // Only the sheet heading says Agent; member rows have no role subtitle.
      expect(find.text('Agent'), findsOneWidget);
      final list = tester.widget<ListView>(find.byType(ListView));
      expect(
        list.padding,
        const EdgeInsets.fromLTRB(Grid.gutter, 0, Grid.gutter, Grid.xs),
      );
      members = ['a', 'b'];
      ProviderScope.containerOf(
        tester.element(find.byType(_Harness)),
      ).invalidate(channelMembersProvider('chat'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Beta'));
      await tester.pumpAndSettle();
      expect(find.byTooltip('Change agent: Beta'), findsOneWidget);
      expect(
        prefs.getString('me-assistant:${sync.community}:${sync.pubkey}:chat'),
        'b',
      );
      await tester.tap(find.text('Send'));
      await tester.pumpAndSettle();
      expect(sent, ['b']);
      expect(ownershipReads, 2);
    },
  );
  testWidgets('same-name agents read distinctly in the picker and header', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final sync = MeSync(
      RelaySessionNotifier(),
      RelayConfig(
        baseUrl: 'https://relay.test',
        nsec: nostr.Nip19.encode(
          prefix: nostr.Nip19Prefix.nsec,
          data: '0' * 63 + '1',
        ),
      ),
      prefs: prefs,
    );
    final first = 'a1' * 32;
    final second = 'b2' * 32;
    await prefs.setString(
      'me-assistant:${sync.community}:${sync.pubkey}:chat',
      first,
    );
    var live = [first, second];
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          meSyncProvider.overrideWithValue(sync),
          meAssistantsProvider.overrideWith(
            (ref) async => [
              for (final key in live)
                AgentDirectoryEntry(pubkey: key, displayName: 'Atlas'),
            ],
          ),
          channelMembersProvider('chat').overrideWith(
            (ref) async => [
              for (final key in [first, second])
                ChannelMember(
                  pubkey: key,
                  role: 'bot',
                  joinedAt: DateTime(2026),
                ),
            ],
          ),
          meAssistantOwnershipProvider.overrideWith((ref, key) async => true),
          // An archived agent's profile stays in the client's cache.
          userCacheProvider.overrideWith(
            () => _CachedProfiles({
              first: UserProfile(pubkey: first, displayName: 'Atlas'),
            }),
          ),
        ],
        child: MaterialApp(theme: AppTheme.light(), home: _Harness((_) {})),
      ),
    );
    await tester.pumpAndSettle();
    String headerLabel() => tester
        .widget<IconButton>(find.byKey(const ValueKey('me-assistant-button')))
        .tooltip!;
    List<String> pickerLabels() => [
      for (final row in tester.widgetList<AppListRowRaw>(
        find.byType(AppListRowRaw),
      ))
        (row.title as Text).data!,
    ];

    final firstHeader = headerLabel();
    expect(firstHeader, startsWith('Change agent: Atlas'));
    expect(firstHeader, isNot('Change agent: Atlas'));
    await tester.tap(find.byKey(const ValueKey('me-assistant-button')));
    await tester.pumpAndSettle();
    final labels = pickerLabels();
    expect(labels, hasLength(2));
    expect(labels.toSet(), hasLength(2), reason: 'Namesakes must differ');
    expect('Change agent: ${labels.first}', firstHeader);
    await tester.tap(find.text(labels.last));
    await tester.pumpAndSettle();
    expect(headerLabel(), 'Change agent: ${labels.last}');

    // The first agent is archived: no live namesake remains, so the cached
    // profiles of hidden agents add no qualifier.
    live = [second];
    ProviderScope.containerOf(
      tester.element(find.byType(_Harness)),
    ).invalidate(meAssistantsProvider);
    await tester.pumpAndSettle();
    expect(headerLabel(), 'Change agent: Atlas');
    await tester.tap(find.byKey(const ValueKey('me-assistant-button')));
    await tester.pumpAndSettle();
    expect(pickerLabels(), ['Atlas']);
    await tester.tapAt(Offset.zero);
    await tester.pumpAndSettle();

    // A saved switch to the archived agent still names it apart from its live
    // namesake, where the owner decides whether to finish it.
    await MeAgentChange.create(sync, 'chat', first).save(sync, 'chat');
    ProviderScope.containerOf(
      tester.element(find.byType(_Harness)),
    ).invalidate(meAssistantsProvider);
    await tester.pumpAndSettle();
    final pending = find.textContaining('Finish choosing ');
    final archivedName = tester
        .widget<Text>(pending)
        .data!
        .substring('Finish choosing '.length);
    expect(archivedName, startsWith('Atlas'));
    expect(archivedName, isNot('Atlas'));
    expect(headerLabel(), 'Pending agent change: $archivedName');
    await tester.tap(pending);
    await tester.pumpAndSettle();
    expect(find.textContaining('Your choice of $archivedName'), findsOneWidget);
  });
}
