import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_filter/channel_filter_activity.dart';
import 'package:buzz/features/channels/channel_sections/section_reorder_sheet.dart';
import 'package:buzz/features/channels/channel_stars/channel_stars_provider.dart';
import 'package:buzz/features/channels/channel_stars/channel_stars_storage.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/me/me_page.dart';
import 'package:buzz/features/channels/me/me_unread_provider.dart';
import 'package:buzz/features/home/home_page.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:buzz/shared/me/me_assistants.dart';
import 'package:buzz/shared/me/me_groups.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/app_list.dart';
import 'package:buzz/shared/widgets/tabler_star_icon.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:shared_preferences_platform_interface/shared_preferences_platform_interface.dart';

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
    final orderStore = _OrderStore();
    final previousStore = SharedPreferencesStorePlatform.instance;
    SharedPreferencesStorePlatform.instance = orderStore;
    addTearDown(() => SharedPreferencesStorePlatform.instance = previousStore);
    final prefs = await SharedPreferences.getInstance();
    final relay = SidebarRelay();
    // The relay this Home's config names; its store holds the heads below.
    relay.session.debugRelayUrl = 'https://relay.example';
    _viewer = relay.pubkey;
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    var soloActivityReads = 0;
    // Before desktop's unified record, the legacy Team sections apply.
    relay.stored.add(
      relay.event('channel-sections', {
        'version': 1,
        // Stored out of display order, as a Move up leaves them.
        'sections': [
          {'id': 'old', 'name': 'Old', 'order': 1},
          {'id': 'newer', 'name': 'Newer', 'order': 0},
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
          channelFilterActivityProvider('solo-scratch').overrideWith((_) async {
            if (soloActivityReads++ == 0) {
              throw StateError('Activity unavailable');
            }
            return now - 8 * 86400;
          }),
          meGroupsProvider.overrideWith(
            (ref) => Stream.value(
              const MeGroups(
                groups: [
                  // Shares Newer's ID, so Newer projects under another.
                  {'id': 'newer', 'name': 'Focus', 'defaultTemplateId': ''},
                ],
              ),
            ),
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

    // Legacy sections stay editable, and their moves follow display order.
    expect(top('design-crit'), greaterThan(top('Old')));
    expect(top('Old'), greaterThan(top('Newer')));
    await tester.tap(find.byTooltip('Old options'));
    await tester.pumpAndSettle();
    expect(find.text('Rename section'), findsOneWidget);
    await tester.tap(find.text('Reorder sections'));
    await tester.pumpAndSettle();
    final list = tester.widget<ReorderableListView>(
      find.byType(ReorderableListView),
    );
    list.onReorderItem!(0, 1);
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Discard order'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Discard changes'));
    await tester.pumpAndSettle();
    // Team actions follow the Team section, not the Solo project sharing its
    // ID.
    for (final (header, team) in [('Newer', true), ('Focus', false)]) {
      await tester.tap(find.byTooltip('$header options'));
      await tester.pumpAndSettle();
      expect(find.text('Rename section'), team ? findsOneWidget : findsNothing);
      await tester.tap(find.byTooltip('Discard changes'));
      await tester.pumpAndSettle();
    }

    // A Solo project header tells a screen reader whether it is expanded.
    final semantics = tester.ensureSemantics();
    final focus = find.bySemanticsLabel('Focus');
    expect(
      tester.getSemantics(focus),
      isSemantics(isButton: true, hasExpandedState: true, isExpanded: true),
    );
    await tester.tap(inList('Focus'));
    await tester.pumpAndSettle();
    expect(
      tester.getSemantics(focus),
      isSemantics(isButton: true, hasExpandedState: true, isExpanded: false),
    );
    semantics.dispose();

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
        'groups': {'section:launch': 'recent'},
      }, now),
    ]) {
      relay
        ..stored.add(head)
        ..emit(head);
    }
    await tester.pumpAndSettle();
    expect(inList('Old'), findsNothing);
    expect(inList('Focus'), findsNothing);

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
    Finder menuIcon(String label) => find.descendant(
      of: find.byTooltip('$label options'),
      matching: find.byIcon(BuzzIcons.ellipsis),
    );
    final teamIcon = menuIcon('Team');
    for (final label in ['Solo', 'Direct messages', 'Launch']) {
      final icon = menuIcon(label);
      expect(tester.getCenter(icon).dx, tester.getCenter(teamIcon).dx);
      expect(tester.getSize(icon), tester.getSize(teamIcon));
      expect(
        tester.widget<Icon>(icon).color,
        tester.widget<Icon>(teamIcon).color,
      );
    }
    expect(
      tester.getTopLeft(find.byIcon(BuzzIcons.messageCircle).first).dx,
      tester.getTopLeft(find.byKey(const ValueKey('channel-icon-general'))).dx,
    );
    final emoji = tester.getRect(
      find.ancestor(of: inList('🚀'), matching: find.byType(FittedBox)).first,
    );
    final channelIcon = tester.getRect(
      find.byKey(const ValueKey('channel-icon-general')),
    );
    final star = find.byType(TablerStarIcon);
    expect(tester.widget<TablerStarIcon>(star).filled, isTrue);
    expect(tester.getCenter(star).dx, channelIcon.center.dx);
    expect(tester.getCenter(star).dy, tester.getCenter(inList('Starred')).dy);
    expect(
      tester.getTopLeft(inList('Starred')).dx,
      tester.getTopLeft(inList('Launch')).dx,
    );
    expect(emoji.center.dx, channelIcon.center.dx);
    expect(emoji.center.dy, tester.getCenter(inList('Launch')).dy);
    expect(
      emoji.center.dy,
      tester.getCenter(find.byTooltip('Launch options')).dy,
    );
    expect(tester.getTopLeft(inList('Launch')).dx - emoji.right, 8);
    // A workspace holds team and Solo rows alike, in desktop's sort.
    for (final filed in ['design-crit', 'solo-plan']) {
      expect(top(filed), greaterThan(top('Launch')));
      expect(top(filed), lessThan(top('Later')));
    }
    expect(top('solo-plan'), lessThan(top('design-crit')));
    expect(inList('No chats yet'), findsNothing);

    // Desktop owns these workspaces and their sort: only filters remain.
    final launchOptions = tester.getRect(find.byTooltip('Launch options'));
    expect(launchOptions.size, const Size(64, 48));
    await tester.tapAt(launchOptions.bottomRight - const Offset(2, 2));
    await tester.pumpAndSettle();
    expect(find.text('Show'), findsOneWidget);
    expect(find.text('Rename section'), findsNothing);
    expect(find.text('Delete section'), findsNothing);
    final sortChoice = find.byKey(const ValueKey('section-menu-Sort'));
    expect(tester.widget<AppListRow>(sortChoice).onTap, isNull);
    // Unreads keeps the unread Solo chat its row shows as unread.
    await tester.tap(find.byKey(const ValueKey('section-menu-Show')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Unreads'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Apply changes'));
    await tester.pumpAndSettle();
    expect(inList('solo-plan'), findsOneWidget);
    expect(inList('design-crit'), findsNothing);

    // Only the filter choice was saved; no layout or sort record desktop
    // would have to reconcile.
    expect(relay.published.map((e) => e.getTagValue('d')), ['channel-filter']);
    final soloSection = find.byKey(const ValueKey('me-ungrouped-chats'));
    Finder soloDisclosure(String label) =>
        find.descendant(of: soloSection, matching: find.text(label));
    await tester.tap(find.byTooltip('Solo options'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('section-menu-Show')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Unreads').last);
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Apply changes'));
    await tester.pumpAndSettle();
    expect(inList('solo-scratch'), findsNothing);
    relay.session.setStatus(SessionStatus.disconnected);
    await tester.pumpAndSettle();
    await tester.tap(soloDisclosure('Show more'));
    await tester.pumpAndSettle();
    expect(inList('solo-scratch'), findsOneWidget);
    await tester.tap(soloDisclosure('Show fewer'));
    await tester.pumpAndSettle();
    expect(inList('solo-scratch'), findsNothing);
    expect(relay.published.map((e) => e.getTagValue('d')), [
      'channel-filter',
      'channel-filter',
    ]);
    relay.session.setStatus(SessionStatus.connected);
    await tester.pumpAndSettle();

    await tester.tap(find.byTooltip('Solo options'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('section-menu-Show')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Active'));
    await tester.pumpAndSettle();
    await tester.tap(
      find.byKey(const ValueKey('section-menu-Active in the past')),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('7 days'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Apply changes'));
    await tester.pumpAndSettle();
    expect(inList('solo-scratch'), findsOneWidget);
    expect(find.text('Some activity couldn’t load'), findsOneWidget);
    await tester.tap(find.text('Retry'));
    await tester.pumpAndSettle();
    expect(find.text('Some activity couldn’t load'), findsNothing);
    expect(inList('solo-scratch'), findsNothing);
    expect(inList('solo-plan'), findsOneWidget);
    await tester.tap(find.byTooltip('Solo options'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Reorder sections'));
    await tester.pumpAndSettle();
    Finder handle(String id) => find.descendant(
      of: find.byKey(ValueKey(id)).last,
      matching: find.byIcon(Icons.drag_handle),
    );
    final haptics = <Object?>[];
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        if (call.method == 'HapticFeedback.vibrate') {
          haptics.add(call.arguments);
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
    final start = tester.getCenter(handle('solo'));
    final gesture = await tester.startGesture(start);
    await tester.pump();
    for (var step = 1; step <= 18; step++) {
      await gesture.moveTo(start - Offset(0, step * 10));
      await tester.pump(const Duration(milliseconds: 24));
    }
    await tester.pumpAndSettle();
    expect(haptics.first, 'HapticFeedbackType.mediumImpact');
    expect(haptics.skip(1), isNotEmpty);
    expect(haptics.skip(1), everyElement('HapticFeedbackType.selectionClick'));
    await gesture.up();
    await tester.pumpAndSettle();
    expect(
      tester.getCenter(handle('solo')).dy,
      lessThan(tester.getCenter(handle('channels')).dy),
    );

    const sectionIds = [
      'starred',
      'group:launch',
      'group:later',
      'channels',
      'solo',
      'dms',
    ];
    List<String> sheetOrder(Iterable<String> ids) => ids.toList()
      ..sort(
        (a, b) => tester
            .getCenter(handle(a))
            .dy
            .compareTo(tester.getCenter(handle(b)).dy),
      );
    final firstOrder = sheetOrder(sectionIds);
    // The signing identity owns the device-local order before a profile loads.
    final key = 'section-order:https://relay.example:${relay.pubkey}';
    final container = ProviderScope.containerOf(tester.element(handle('solo')));
    final orders = container.read(mobileSectionOrderProvider(key).notifier);
    final gate = Completer<void>();
    orderStore.gate = gate;
    await tester.tap(find.byTooltip('Save order'));
    await tester.pumpAndSettle();
    // Flutter's screen-reader reorder actions call this even with drag disabled.
    tester
        .widget<ReorderableListView>(find.byType(ReorderableListView))
        .onReorderItem!(0, 1);
    await tester.pump();
    expect(sheetOrder(sectionIds), firstOrder);
    expect(await orders.save(const [], firstOrder.reversed.toList()), isFalse);
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    expect(find.byType(ReorderableListView), findsOneWidget);
    gate.complete();
    await tester.pumpAndSettle();
    expect(prefs.getStringList(key), firstOrder);
    expect(await orders.save(const [], firstOrder.reversed.toList()), isFalse);

    // Reopening from the same options sheet uses the order just saved.
    await tester.tap(find.text('Reorder sections'));
    await tester.pumpAndSettle();
    expect(sheetOrder(sectionIds), firstOrder);
    await tester.tap(find.byTooltip('Discard order'));
    await tester.pumpAndSettle();

    final stars = container.read(channelStarsProvider.notifier) as _Stars;
    stars.setStarred(false);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Reorder sections'));
    await tester.pumpAndSettle();
    expect(find.text('Starred'), findsNothing);

    final dmFocus = Focus.of(tester.element(handle('dms')));
    for (var n = 0; n < 10 && !dmFocus.hasPrimaryFocus; n++) {
      await tester.sendKeyEvent(LogicalKeyboardKey.tab);
      await tester.pump();
    }
    expect(dmFocus.hasPrimaryFocus, isTrue);
    final visibleOrder = firstOrder.where((id) => id != 'starred').toList();
    final from = visibleOrder.indexOf('dms');
    visibleOrder.insert(from - 1, visibleOrder.removeAt(from));
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowUp);
    await tester.pumpAndSettle();
    expect(sheetOrder(visibleOrder), visibleOrder);
    await tester.tap(find.byTooltip('Save order'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Discard changes'));
    await tester.pumpAndSettle();
    final starredIndex = firstOrder.indexOf('starred');
    visibleOrder.insert(starredIndex, 'starred');
    expect(prefs.getStringList(key), visibleOrder);
    expect(
      prefs.getStringList('section-order:https://relay.example:null'),
      isNull,
    );
    stars.setStarred(true);
    await tester.pumpAndSettle();
    final labels = {
      'starred': 'Starred',
      'group:launch': 'Launch',
      'group:later': 'Later',
      'channels': 'Team',
      'solo': 'Solo',
      'dms': 'Direct messages',
    };
    final rendered = labels.keys.toList()
      ..sort((a, b) => top(labels[a]!).compareTo(top(labels[b]!)));
    expect(rendered, visibleOrder);
    await tester.pumpWidget(const SizedBox());
  });
}

late String _viewer;

Channel _channel(String id, {bool solo = false, int day = 1}) => Channel(
  id: id,
  name: id,
  channelType: 'stream',
  visibility: 'private',
  labels: solo ? const ['buzz.solo'] : const [],
  description: '',
  createdBy: solo ? _viewer : 'other',
  createdAt: DateTime(2026),
  lastMessageAt: DateTime(2026, 1, day),
  memberCount: 2,
  isMember: true,
);

class _Channels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [
    _channel('general'),
    _channel('pinned'),
    _channel('design-crit'),
    // Newest in Launch, so Recent puts it above design-crit.
    _channel('solo-plan', solo: true, day: 9),
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

  void setStarred(bool starred) =>
      state = starred ? build() : const ChannelStarsState(isReady: true);
}

class _OrderStore extends InMemorySharedPreferencesStore {
  _OrderStore() : super.empty();

  Completer<void>? gate;

  @override
  Future<bool> setValue(String type, String key, Object value) async {
    if (key.startsWith('flutter.section-order:')) await gate?.future;
    return super.setValue(type, key, value);
  }
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
