import 'package:buzz/features/channels/default_agent_launcher.dart';
import 'package:buzz/features/agents/agents_page.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/me/me_page.dart';
import 'package:buzz/features/channels/me/me_unread_provider.dart';
import 'package:buzz/features/channels/unread_badge/observed_unread_event.dart';
import 'package:buzz/features/home/home_page.dart';
import 'package:buzz/features/home/messages_filters.dart';
import 'package:buzz/shared/me/me_assistants.dart';
import 'package:buzz/shared/me/me_groups.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:buzz/shared/theme/theme.dart';

Channel chat(String id, {bool member = true, bool archived = false}) => Channel(
  id: id,
  name: id,
  channelType: 'stream',
  visibility: 'private',
  description: '',
  createdBy: 'owner',
  createdAt: DateTime(2026),
  lastMessageAt: DateTime.fromMillisecondsSinceEpoch(20000),
  memberCount: 2,
  isMember: member,
  archivedAt: archived ? DateTime(2026) : null,
);

class _Channels extends ChannelsNotifier {
  var all = [
    chat('solo'),
    chat('ordinary'),
    chat('archived', archived: true),
    chat('nonmember', member: false),
  ];
  int timestamp = 20;
  @override
  Future<List<Channel>> build() async => all;
  @override
  Map<String, int> get latestObservedByChannel => {
    for (final c in all) c.id: timestamp,
  };
  @override
  Map<String, Map<String, ObservedUnreadEvent>>
  get observedUnreadEventsByChannel => {
    for (final c in all)
      c.id: {
        'reply': ObservedUnreadEvent(
          id: 'reply',
          createdAt: timestamp,
          rootId: 'root',
          highPriority: false,
          countsTowardBadge: true,
          countsTowardAppBadge: false,
        ),
      },
  };
  void reply() {
    timestamp++;
    all = [
      for (final c in all)
        c.copyWith(
          lastMessageAt: DateTime.fromMillisecondsSinceEpoch(timestamp * 1000),
        ),
    ];
    state = AsyncData([...all]);
  }
}

class _Reads extends ReadStateNotifier {
  final bool empty;
  final bool ready;
  _Reads({this.empty = false, this.ready = true});
  @override
  void seedContextsRead(Map<String, int> contexts) {
    for (final entry in contexts.entries) {
      seedContextRead(entry.key, entry.value);
    }
  }

  @override
  void seedContextRead(String key, int timestamp) => read(key, timestamp);
  @override
  ReadStateState build() => ReadStateState(
    isReady: ready,
    pubkey: 'owner',
    contexts: empty ? {} : {'solo': 10},
    version: 0,
  );
  void finishLoading() => state = ReadStateState(
    isReady: true,
    pubkey: state.pubkey,
    contexts: state.contexts,
    version: state.version + 1,
  );
  void resetIdentity() => state = const ReadStateState(
    isReady: true,
    pubkey: 'other-owner',
    contexts: {},
    version: 0,
  );
  void read(String key, int timestamp) =>
      state = state.copyWithContext(key, timestamp);
  void force() => state = ReadStateState(
    isReady: true,
    pubkey: 'owner',
    contexts: state.contexts,
    version: state.version + 1,
    forcedUnreadContexts: const {'msg:reply': 'solo'},
  );
}

const organization = MeGroups(
  groups: [
    {'id': 'project', 'name': 'Project', 'defaultTemplateId': ''},
  ],
  assignments: {'solo': 'project'},
  channels: ['solo', 'archived', 'nonmember'],
);

void main() {
  for (final duringSeed in [false, true]) {
    test(
      'fresh Solo history is gated; new reply during seeding=$duringSeed badges',
      () async {
        SharedPreferences.setMockInitialValues({});
        final prefs = await SharedPreferences.getInstance();
        final channels = _Channels();
        final reads = _Reads(empty: true);
        final container = ProviderContainer(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            channelsProvider.overrideWith(() => channels),
            readStateProvider.overrideWith(() => reads),
            meGroupsProvider.overrideWith(
              (ref) =>
                  Stream.value(const MeGroups(channels: ['solo', 'ordinary'])),
            ),
          ],
        );
        addTearDown(container.dispose);
        container.listen(channelsProvider, (_, _) {});
        container.listen(meGroupsProvider, (_, _) {});
        await container.read(channelsProvider.future);
        await container.read(meGroupsProvider.future);
        final observed = <Set<String>>[];
        container.listen(
          meUnreadChannelIdsProvider,
          (_, next) => observed.add(next),
          fireImmediately: true,
        );
        expect(container.read(meUnreadChannelIdsProvider), isEmpty);
        if (duringSeed) {
          channels.reply();
          expect(container.read(meUnreadChannelIdsProvider), isEmpty);
        }
        await container.pump();
        expect(
          container.read(readStateProvider).effectiveTimestamp('solo'),
          20,
        );
        if (!duringSeed) {
          expect(observed.every((ids) => ids.isEmpty), isTrue);
          channels.reply();
        }
        await container.pump();
        expect(container.read(meUnreadChannelIdsProvider), {
          'solo',
          'ordinary',
        });
        expect(
          container.read(readStateProvider).effectiveTimestamp('solo'),
          20,
        );
        reads.resetIdentity();
        expect(container.read(meUnreadChannelIdsProvider), isEmpty);
        await container.pump();
        expect(
          container.read(readStateProvider).effectiveTimestamp('solo'),
          21,
        );
        expect(container.read(meUnreadChannelIdsProvider), isEmpty);
        channels.reply();
        await container.pump();
        expect(container.read(meUnreadChannelIdsProvider), {
          'solo',
          'ordinary',
        });
      },
    );
  }
  for (final loading in [false, true]) {
    testWidgets(
      'Solo retains row, section, and tab unread after loading=$loading',
      (tester) async {
        SharedPreferences.setMockInitialValues({});
        final prefs = await SharedPreferences.getInstance();
        final channels = _Channels();
        final reads = _Reads(empty: loading, ready: !loading);
        await tester.pumpWidget(
          ProviderScope(
            overrides: [
              savedPrefsProvider.overrideWithValue(prefs),
              channelsProvider.overrideWith(() => channels),
              readStateProvider.overrideWith(() => reads),
              meGroupsProvider.overrideWith(
                (ref) => Stream.value(organization),
              ),
              meAssistantsProvider.overrideWith((ref) async => []),
            ],
            child: MaterialApp(
              theme: AppTheme.light(),
              home: Consumer(
                builder: (context, ref, _) => HomePage(
                  onAgents: showAgents,
                  defaultAgentBuilder: (context) => DefaultAgentLauncher(
                    onChooseAgent: () => showAgents(context),
                  ),
                  settingsPageBuilder: (_) => const SizedBox(),
                  mePageBuilder: (_, action, workspace) => MePage(
                    sliver: true,
                    newChatAction: action,
                    newWorkspaceAction: workspace,
                  ),
                  hasUnreadInbox: false,
                  hasUnreadSolo: ref
                      .watch(meUnreadChannelIdsProvider)
                      .isNotEmpty,
                ),
              ),
            ),
          ),
        );
        await tester.pumpAndSettle();
        final container = ProviderScope.containerOf(
          tester.element(find.byType(HomePage)),
        );
        await tester.tap(
          find.descendant(
            of: find.byType(MessagesFilters),
            matching: find.text('Team'),
          ),
        );
        await tester.pumpAndSettle();
        if (loading) {
          expect(container.read(meUnreadChannelIdsProvider), isEmpty);
          expect(find.bySemanticsLabel('Solo, unread'), findsNothing);
          expect(
            container.read(readStateProvider).effectiveTimestamp('solo'),
            isNull,
          );
          channels.reply();
          await tester.pumpAndSettle();
          expect(container.read(meUnreadChannelIdsProvider), isEmpty);
          expect(
            container.read(readStateProvider).effectiveTimestamp('solo'),
            isNull,
          );
          reads.finishLoading();
          await tester.pumpAndSettle();
          expect(
            container.read(readStateProvider).effectiveTimestamp('solo'),
            20,
          );
        }
        expect(container.read(meUnreadChannelIdsProvider), {'solo'});
        expect(find.bySemanticsLabel('Solo, unread'), findsOneWidget);
        expect(find.text('solo').hitTestable(), findsNothing);
        await tester.tap(find.text('Solo'));
        await tester.pumpAndSettle();
        expect(
          tester.widget<Text>(find.text('solo')).style?.fontWeight,
          FontWeight.w700,
        );
        expect(find.bySemanticsLabel('solo, unread'), findsOneWidget);
        await tester.tap(find.text('Project'));
        await tester.pumpAndSettle();
        expect(find.text('solo').hitTestable(), findsNothing);
        expect(
          find.byKey(const ValueKey('solo-section-unread-Project')),
          findsOneWidget,
        );
        reads.read('msg:reply', channels.timestamp);
        await tester.pumpAndSettle();
        expect(
          find.byKey(const ValueKey('solo-section-unread-Project')),
          findsNothing,
        );
        await tester.tap(
          find.descendant(
            of: find.byType(MessagesFilters),
            matching: find.text('Team'),
          ),
        );
        await tester.pumpAndSettle();
        expect(find.bySemanticsLabel('Solo, unread'), findsNothing);
        channels.reply();
        await tester.pumpAndSettle();
        expect(find.bySemanticsLabel('Solo, unread'), findsOneWidget);
        reads.read('thread:root', channels.timestamp);
        await tester.pumpAndSettle();
        expect(find.bySemanticsLabel('Solo, unread'), findsNothing);
        reads.force();
        await tester.pumpAndSettle();
        expect(find.bySemanticsLabel('Solo, unread'), findsOneWidget);
      },
    );
  }
}
