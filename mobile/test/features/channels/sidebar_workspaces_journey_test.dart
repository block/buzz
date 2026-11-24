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
          channelPinsProvider.overrideWith(() => _Stars(id: 'pinned')),
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

    for (final label in ['Old', 'Solo']) {
      final header = find
          .ancestor(
            of: inList(label),
            matching: find.byType(label == 'Solo' ? InkWell : GestureDetector),
          )
          .first;
      final expandedTop = top(label);
      await tester.tap(inList(label));
      await tester.pumpAndSettle();
      expect(top(label), expandedTop);
      expect(
        tester.getBottomLeft(header).dy -
            tester.getBottomLeft(inList(label)).dy,
        closeTo(Grid.inventoryRowInset, 0.01),
      );
      expect(
        top(label) - tester.getTopLeft(header).dy,
        Grid.inventorySectionTopInset,
      );
      double chevronCenter(IconData icon) => tester
          .getCenter(find.descendant(of: header, matching: find.byIcon(icon)))
          .dy;
      expect(
        chevronCenter(BuzzIcons.chevronRight),
        tester.getCenter(inList(label)).dy,
      );
      await tester.tap(inList(label));
      await tester.pumpAndSettle();
      expect(
        chevronCenter(BuzzIcons.chevronDown),
        tester.getCenter(inList(label)).dy,
      );
      expect(top(label), expandedTop);
    }

    Future<void> openEdit() async {
      await tester.tap(find.byTooltip('Messages options'));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('Edit Sections'));
      await tester.tap(find.text('Edit Sections'));
      await tester.pump();
      expect(find.byType(ReorderableListView), findsNothing);
      await tester.pumpAndSettle();
    }

    await tester.tap(find.byTooltip('Messages options'));
    await tester.pumpAndSettle();
    expect(find.byType(BottomSheet), findsOneWidget);
    expect(find.byType(ReorderableListView), findsNothing);
    final view = find.byKey(const ValueKey('messages-display-View'));
    await tester.tap(view);
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(MenuItemButton, 'Unreads'));
    await tester.pumpAndSettle();
    expect(find.text('Unreads'), findsOneWidget);
    await tester.tap(find.byTooltip('Discard changes'));
    await tester.pumpAndSettle();
    Future<void> openDisplay() async {
      await tester.tap(find.byTooltip('Messages options'));
      await tester.pumpAndSettle();
    }

    await openDisplay();
    expect(find.text('Unreads'), findsNothing);
    expect(find.text('Sort'), findsOneWidget);
    Future<void> choose(Finder control, String label) async {
      await tester.tap(control);
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(MenuItemButton, label));
      await tester.pumpAndSettle();
    }

    final sort = find.byKey(const ValueKey('messages-display-Sort'));
    await choose(sort, 'Recent');
    await choose(view, 'Unreads');
    expect(find.text('Section containers'), findsNothing);
    await tester.tap(find.byTooltip('Apply changes'));
    await tester.pumpAndSettle();
    expect(inList('general'), findsNothing);
    expect(inList('design-crit'), findsNothing);
    expect(inList('solo-scratch'), findsNothing);
    await openDisplay();
    expect(find.text('Recent'), findsOneWidget);
    expect(find.text('Unreads'), findsOneWidget);
    await choose(sort, 'Recent');
    await choose(view, 'All chats');
    await tester.tap(find.byTooltip('Apply changes'));
    await tester.pumpAndSettle();

    // Legacy sections stay editable, and their moves follow display order.
    expect(top('design-crit'), greaterThan(top('Old')));
    expect(top('Old'), greaterThan(top('Newer')));
    await openEdit();

    await tester.tap(find.byTooltip('Old actions'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Rename'));
    await tester.pumpAndSettle();
    expect(find.text('Rename Section'), findsOneWidget);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Old actions'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Remove'));
    await tester.pumpAndSettle();
    expect(find.text('Delete "Old"?'), findsOneWidget);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Discard order'));
    await tester.pumpAndSettle();
    expect(find.byTooltip('Discard changes'), findsNothing);
    // Team actions follow the Team section, not the Solo project sharing its
    // ID.
    for (final (header, team) in [('Newer', true), ('Focus', false)]) {
      await openEdit();
      final menu = find.byTooltip('$header actions');
      expect(menu, team ? findsOneWidget : findsNothing);
      expect(
        find.byTooltip('Move $header with the up and down arrow keys'),
        findsOneWidget,
      );
      await tester.tap(find.byTooltip('Discard order'));
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

    expect(find.byTooltip('pinned'), findsOneWidget);
    expect(inList('Pins'), findsNothing);
    expect(
      tester.getCenter(find.byTooltip('pinned')).dy,
      lessThan(top('Launch')),
    );
    final order = [
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
    expect(find.byTooltip('Messages options'), findsOneWidget);
    for (final label in ['Solo', 'Direct messages', 'Team', 'Launch']) {
      expect(find.byTooltip('$label options'), findsNothing);
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
    expect(inList('Starred'), findsOneWidget);
    expect(inList('solo-star'), findsOneWidget);
    expect(find.byTooltip('solo-star'), findsNothing);
    expect(
      tester.getTopLeft(inList('Later')).dx,
      tester.getTopLeft(inList('Team')).dx,
    );
    expect(emoji.center.dx, channelIcon.center.dx);
    expect(emoji.center.dy, tester.getCenter(inList('Launch')).dy);
    expect(emoji.size, channelIcon.size);
    expect(tester.getTopLeft(inList('Launch')).dx - emoji.right, 8);
    expect(tester.getTopLeft(inList('general')).dx - channelIcon.right, 8);
    // A workspace holds team and Solo rows alike, in desktop's sort.
    for (final filed in ['design-crit', 'solo-plan']) {
      expect(top(filed), greaterThan(top('Launch')));
      expect(top(filed), lessThan(top('Later')));
    }
    expect(top('solo-plan'), lessThan(top('design-crit')));
    expect(inList('No chats yet'), findsNothing);

    await tester.tap(inList('Launch'));
    await tester.pumpAndSettle();
    expect(inList('design-crit'), findsNothing);
    await tester.tap(inList('Launch'));
    await tester.pumpAndSettle();
    expect(inList('design-crit'), findsOneWidget);

    // Apply the Home filter across desktop-owned sections and Solo.
    await openDisplay();
    await choose(view, 'Unreads');
    await tester.tap(find.byTooltip('Apply changes'));
    await tester.pumpAndSettle();
    expect(inList('solo-plan'), findsOneWidget);
    expect(inList('design-crit'), findsNothing);
    expect(inList('solo-scratch'), findsNothing);
    expect(relay.published, isEmpty);
    await openDisplay();
    await choose(view, 'Active');
    await choose(
      find.byKey(const ValueKey('messages-display-Active in the past')),
      '7 days',
    );
    await tester.tap(find.byTooltip('Apply changes'));
    await tester.pumpAndSettle();
    expect(inList('solo-scratch'), findsOneWidget);
    await tester.tap(find.text('Retry').last);
    await tester.pumpAndSettle();
    expect(inList('solo-scratch'), findsNothing);

    await openDisplay();
    await choose(view, 'All chats');
    await tester.tap(find.byTooltip('Apply changes'));
    await tester.pumpAndSettle();
    await openEdit();
    await tester.pumpAndSettle();
    Finder handle(String id) => find.descendant(
      of: find.byKey(ValueKey(id)).last,
      matching: find.byIcon(BuzzIcons.menu2),
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

    // Reopen options from Messages after saving the order.
    await openEdit();
    await tester.pumpAndSettle();
    expect(sheetOrder(sectionIds), firstOrder);
    await tester.tap(find.byTooltip('Discard order'));
    await tester.pumpAndSettle();

    final stars = container.read(channelStarsProvider.notifier) as _Stars;
    stars.setStarred(false);
    await tester.pumpAndSettle();
    await openEdit();
    await tester.pumpAndSettle();
    expect(find.text('Starred'), findsOneWidget);

    final dmFocus = Focus.of(tester.element(handle('dms')));
    for (
      var n = 0;
      n < 2 * sectionIds.length + 4 && !dmFocus.hasPrimaryFocus;
      n++
    ) {
      await tester.sendKeyEvent(LogicalKeyboardKey.tab);
      await tester.pump();
    }
    expect(dmFocus.hasPrimaryFocus, isTrue);
    final visibleOrder = firstOrder.toList();
    final from = visibleOrder.indexOf('dms');
    visibleOrder.insert(from - 1, visibleOrder.removeAt(from));
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowUp);
    await tester.pumpAndSettle();
    expect(sheetOrder(visibleOrder), visibleOrder);
    await tester.tap(find.byTooltip('Save order'));
    await tester.pumpAndSettle();
    expect(find.byTooltip('Discard changes'), findsNothing);
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
    _channel('solo-star', solo: true),
    _channel('design-crit'),
    // Newest in Launch, so Recent puts it above design-crit.
    _channel('solo-plan', solo: true, day: 9),
    _channel('solo-scratch', solo: true),
  ];
}

class _Stars extends ChannelStarsNotifier {
  _Stars({this.id = 'solo-star'});
  final String id;

  @override
  ChannelStarsState build() => ChannelStarsState(
    isReady: true,
    store: ChannelStarStore(
      channels: {id: const ChannelStarEntry(starred: true, updatedAt: 1)},
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
