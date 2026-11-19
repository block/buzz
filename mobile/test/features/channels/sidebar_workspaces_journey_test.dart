import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_sections/channel_sections_provider.dart';
import 'package:buzz/features/channels/channel_sections/workspace_layout.dart';
import 'package:buzz/features/channels/channel_sort/channel_sort_provider.dart';
import 'package:buzz/features/channels/channel_stars/channel_stars_provider.dart';
import 'package:buzz/features/channels/channel_stars/channel_stars_storage.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/me/me_page.dart';
import 'package:buzz/features/home/home_page.dart';
import 'package:buzz/features/home/messages_filters.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:buzz/shared/me/me_assistants.dart';
import 'package:buzz/shared/me/me_groups.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/app_list.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// Home renders desktop's unified sidebar: its order, its filings across
/// team and Solo rows, and no edits mobile can't save.
void main() {
  testWidgets('Home follows desktop workspaces and keeps them read-only', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1170, 6000);
    tester.view.devicePixelRatio = 3;
    addTearDown(tester.view.reset);
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final workspace = WorkspaceRecord.fromJson({
      'version': 1,
      'sections': [
        {'id': 'launch', 'name': 'Launch', 'icon': '🚀', 'order': 0},
        {'id': 'later', 'name': 'Later', 'order': 1},
      ],
      'assignments': {'design-crit': 'launch', 'solo-plan': 'launch'},
      // Saved on desktop with Team above Solo. Later is newer than the
      // reorder, so it follows Launch.
      'sectionOrder': ['starred', 'group:launch', 'channels', 'solo', 'dms'],
    });

    await tester.pumpWidget(
      ProviderScope(
        retry: (_, _) => null,
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          myPubkeyProvider.overrideWithValue('viewer'),
          channelsProvider.overrideWith(_Channels.new),
          meGroupsProvider.overrideWith(
            (ref) => Stream.value(const MeGroups()),
          ),
          defaultAgentProvider.overrideWith((ref) => Stream.value(null)),
          meAssistantsProvider.overrideWith((ref) async => []),
          channelSectionsProvider.overrideWith(() => _Sections(workspace)),
          channelSortProvider.overrideWith(_UnifiedSort.new),
          channelStarsProvider.overrideWith(_Stars.new),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: HomePage(
            settingsPageBuilder: (_) => const SizedBox(),
            onAgents: (_) {},
            defaultAgentBuilder: (_) => const SizedBox(),
            openBestieConversation: (_, _, _) async {},
            mePageBuilder: (_, chat, project) => MePage(
              sliver: true,
              newChatAction: chat,
              newWorkspaceAction: project,
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    Finder inList(String text) => find.descendant(
      of: find.byType(CustomScrollView),
      matching: find.text(text),
    );
    double top(String text) => tester.getTopLeft(inList(text)).dy;

    final order = [
      'Starred',
      'pinned',
      'Launch',
      'Later',
      'Team',
      'general',
      'Solo',
      'solo-scratch',
      'Direct messages',
    ];
    for (final (index, label) in order.indexed.skip(1)) {
      expect(
        top(label),
        greaterThan(top(order[index - 1])),
        reason: '$label follows ${order[index - 1]}',
      );
    }
    expect(inList('🚀'), findsOneWidget);
    // A workspace holds team and Solo rows alike.
    for (final filed in ['design-crit', 'solo-plan']) {
      expect(top(filed), greaterThan(top('Launch')));
      expect(top(filed), lessThan(top('Later')));
    }
    expect(inList('No chats yet'), findsNothing);

    // Desktop owns these workspaces and their sort: only filters remain.
    await tester.tap(find.byTooltip('Launch options'));
    await tester.pumpAndSettle();
    expect(find.text('Show'), findsOneWidget);
    expect(find.text('Rename section'), findsNothing);
    expect(find.text('Delete section'), findsNothing);
    final sortChoice = find.byKey(const ValueKey('section-choice-Sort-Recent'));
    expect(tester.widget<AppListRow>(sortChoice).onTap, isNull);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();

    // Solo lists only Solo rows, still in their workspaces.
    await tester.tap(
      find.descendant(
        of: find.byType(MessagesFilters),
        matching: find.text('Solo'),
      ),
    );
    await tester.pumpAndSettle();
    expect(inList('solo-plan'), findsOneWidget);
    expect(inList('solo-scratch'), findsOneWidget);
    for (final hidden in [
      'design-crit',
      'general',
      'pinned',
      'Later',
      'Team',
    ]) {
      expect(inList(hidden), findsNothing, reason: hidden);
    }
    await tester.pumpWidget(const SizedBox());
  });
}

Channel _channel(String id, {bool solo = false}) => Channel(
  id: id,
  name: id,
  channelType: 'stream',
  visibility: 'private',
  labels: solo ? const ['buzz.solo'] : const [],
  description: '',
  createdBy: solo ? 'viewer' : 'other',
  createdAt: DateTime(2026),
  memberCount: 2,
  isMember: true,
);

class _Channels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [
    _channel('general'),
    _channel('pinned'),
    _channel('design-crit'),
    _channel('solo-plan', solo: true),
    _channel('solo-scratch', solo: true),
  ];
}

class _Sections extends ChannelSectionsNotifier {
  _Sections(this._workspace);
  final WorkspaceRecord _workspace;

  @override
  ChannelSectionsState build() =>
      ChannelSectionsState(isReady: true, workspace: _workspace, version: 1);
}

class _UnifiedSort extends ChannelSortNotifier {
  @override
  ChannelSortState build() =>
      const ChannelSortState(isReady: true, editable: false);
}

class _Stars extends ChannelStarsNotifier {
  @override
  ChannelStarsState build() => const ChannelStarsState(
    isReady: true,
    store: ChannelStarStore(
      channels: {'pinned': ChannelStarEntry(starred: true, updatedAt: 1)},
    ),
  );
}
