import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_stars/channel_stars_provider.dart';
import 'package:buzz/features/channels/channel_stars/channel_stars_storage.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/me/me_page.dart';
import 'package:buzz/features/channels/me/me_unread_provider.dart';
import 'package:buzz/features/home/home_page.dart';
import 'package:buzz/features/home/messages_filters.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:buzz/shared/me/me_assistants.dart';
import 'package:buzz/shared/me/me_groups.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/app_list.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'sidebar_sync_fixture.dart';

/// Home renders desktop's unified sidebar once its records arrive: its
/// order, its filings across team and Solo rows, and no edits mobile can't
/// save.
void main() {
  testWidgets('Home follows desktop workspaces and keeps them read-only', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1170, 6000);
    tester.view.devicePixelRatio = 3;
    addTearDown(tester.view.reset);
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final relay = SidebarRelay();
    // The relay this Home's config names; its store holds the heads below.
    relay.session.debugRelayUrl = 'https://relay.example';
    _viewer = relay.pubkey;
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    // Before desktop's unified record, the legacy Team sections apply.
    relay.stored.add(
      relay.event('channel-sections', {
        'version': 1,
        'sections': [
          {'id': 'old', 'name': 'Old', 'order': 0},
        ],
        'assignments': {'design-crit': 'old'},
      }, now - 60),
    );

    await tester.pumpWidget(
      ProviderScope(
        retry: (_, _) => null,
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          relayConfigProvider.overrideWith(() => _Config(relay.keys.nsec)),
          relaySessionProvider.overrideWith(() => relay.session),
          activeCommunityProvider.overrideWith(
            (ref) async => Community(
              id: 'example',
              name: 'Example',
              relayUrl: 'https://relay.example',
              addedAt: DateTime(2026),
            ),
          ),
          channelsProvider.overrideWith(_Channels.new),
          meGroupsProvider.overrideWith(
            (ref) => Stream.value(const MeGroups()),
          ),
          defaultAgentProvider.overrideWith((ref) => Stream.value(null)),
          meAssistantsProvider.overrideWith((ref) async => []),
          channelStarsProvider.overrideWith(_Stars.new),
          meUnreadChannelIdsProvider.overrideWith(_SoloUnread.new),
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

    // Legacy sections stay editable.
    expect(top('design-crit'), greaterThan(top('Old')));
    await tester.tap(find.byTooltip('Old options'));
    await tester.pumpAndSettle();
    expect(find.text('Rename section'), findsOneWidget);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();

    // Desktop saves its unified records; they arrive live.
    for (final head in [
      relay.event('channel-workspaces', {
        'version': 1,
        'sections': [
          {'id': 'launch', 'name': 'Launch', 'icon': '🚀', 'order': 0},
          {'id': 'later', 'name': 'Later', 'order': 1},
        ],
        'assignments': {'design-crit': 'launch', 'solo-plan': 'launch'},
        // Saved with Team above Solo. Later is newer than the reorder, so
        // it follows Launch.
        'sectionOrder': ['starred', 'group:launch', 'channels', 'solo', 'dms'],
        'legacyImportClosed': true,
      }, now),
      relay.event('channel-workspace-sort', {
        'version': 1,
        'groups': {'group:launch': 'recent'},
      }, now),
    ]) {
      relay
        ..stored.add(head)
        ..emit(head);
    }
    await tester.pumpAndSettle();
    expect(inList('Old'), findsNothing);

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
    // Unreads keeps the unread Solo chat its row shows as unread.
    await tester.tap(find.text('Unreads'));
    await tester.tap(find.text('Apply'));
    await tester.pumpAndSettle();
    expect(inList('solo-plan'), findsOneWidget);
    expect(inList('design-crit'), findsNothing);

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
    // Only the filter choice was saved; no layout or sort record desktop
    // would have to reconcile.
    expect(relay.published.map((e) => e.getTagValue('d')), ['channel-filter']);
    await tester.pumpWidget(const SizedBox());
  });
}

late String _viewer;

Channel _channel(String id, {bool solo = false}) => Channel(
  id: id,
  name: id,
  channelType: 'stream',
  visibility: 'private',
  labels: solo ? const ['buzz.solo'] : const [],
  description: '',
  createdBy: solo ? _viewer : 'other',
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

class _Stars extends ChannelStarsNotifier {
  @override
  ChannelStarsState build() => const ChannelStarsState(
    isReady: true,
    store: ChannelStarStore(
      channels: {'pinned': ChannelStarEntry(starred: true, updatedAt: 1)},
    ),
  );
}

class _Config extends RelayConfigNotifier {
  _Config(this._nsec);
  final String _nsec;

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example', nsec: _nsec);
}

class _SoloUnread extends MeUnreadChannelIds {
  @override
  Set<String> build() => const {'solo-plan'};
}
