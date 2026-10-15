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
  memberCount: 2,
  isMember: member,
  archivedAt: archived ? DateTime(2026) : null,
);

class _Channels extends ChannelsNotifier {
  final all = [
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
    state = AsyncData([...all]);
  }
}

class _Reads extends ReadStateNotifier {
  @override
  ReadStateState build() => const ReadStateState(
    isReady: true,
    pubkey: 'owner',
    contexts: {'solo': 10},
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
  testWidgets(
    'Solo retains live unread row, collapsed section, and tab indication',
    (tester) async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final channels = _Channels();
      final reads = _Reads();
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            channelsProvider.overrideWith(() => channels),
            readStateProvider.overrideWith(() => reads),
            meGroupsProvider.overrideWith((ref) => Stream.value(organization)),
            meAssistantsProvider.overrideWith((ref) async => []),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Consumer(
              builder: (context, ref, _) => HomePage(
                settingsPageBuilder: (_) => const SizedBox(),
                mePageBuilder: (_) => const MePage(),
                hasUnreadInbox: false,
                hasUnreadSolo: ref.watch(meUnreadChannelIdsProvider).isNotEmpty,
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      final container = ProviderScope.containerOf(
        tester.element(find.byType(HomePage)),
      );
      expect(container.read(meUnreadChannelIdsProvider), {'solo'});
      expect(find.bySemanticsLabel('Solo, unread'), findsOneWidget);
      expect(find.text('solo'), findsNothing);
      await tester.tap(find.byTooltip('Solo'));
      await tester.pumpAndSettle();
      expect(
        tester.widget<Text>(find.text('solo')).style?.fontWeight,
        FontWeight.w700,
      );
      expect(find.bySemanticsLabel('solo, unread'), findsOneWidget);
      await tester.tap(find.text('Project'));
      await tester.pumpAndSettle();
      expect(find.text('solo'), findsNothing);
      expect(
        find.byKey(const ValueKey('solo-section-unread-Project')),
        findsOneWidget,
      );
      reads.read('msg:reply', 20);
      await tester.pumpAndSettle();
      expect(
        find.byKey(const ValueKey('solo-section-unread-Project')),
        findsNothing,
      );
      await tester.tap(find.byTooltip('Home'));
      await tester.pumpAndSettle();
      expect(find.bySemanticsLabel('Solo, unread'), findsNothing);
      channels.reply();
      await tester.pumpAndSettle();
      expect(find.bySemanticsLabel('Solo, unread'), findsOneWidget);
      reads.read('thread:root', 21);
      await tester.pumpAndSettle();
      expect(find.bySemanticsLabel('Solo, unread'), findsNothing);
      reads.force();
      await tester.pumpAndSettle();
      expect(find.bySemanticsLabel('Solo, unread'), findsOneWidget);
    },
  );
}
