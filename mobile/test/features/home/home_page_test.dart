import 'package:buzz/features/channels/default_agent_launcher.dart';
import 'package:buzz/features/agents/agents_page.dart';
import 'package:buzz/features/channels/me/me_section.dart';
import 'package:buzz/features/activity/activity_page.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:buzz/features/home/home_page.dart';
import 'package:buzz/features/home/messages_filters.dart';
import 'package:buzz/features/home/messages_navigation_bar.dart';
import 'package:buzz/shared/widgets/ios_message_create_button.dart';
import 'package:buzz/features/search/search_page.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  Future<Widget> buildHome({
    int unreadInboxCount = 0,
    bool longSolo = false,
    bool disableAnimations = false,
    Gradient? topSectionGradient,
    ValueChanged<ValueNotifier<VoidCallback?>>? onNewChatAction,
  }) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    return ProviderScope(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        defaultAgentProvider.overrideWith((ref) => Stream.value(null)),
        channelsProvider.overrideWith(_Channels.new),
      ],
      child: MaterialApp(
        theme: AppTheme.light(topSectionGradient: topSectionGradient),
        builder: (context, child) => MediaQuery(
          data: MediaQuery.of(
            context,
          ).copyWith(disableAnimations: disableAnimations),
          child: child!,
        ),
        home: HomePage(
          onAgents: showAgents,
          defaultAgentBuilder: (context) =>
              DefaultAgentLauncher(onChooseAgent: () => showAgents(context)),
          mePageBuilder: (_, action, workspace) {
            onNewChatAction?.call(action);
            return SliverToBoxAdapter(
              child: MeSection(
                label: 'Chats',
                children: [
                  const Text('Personal workspace'),
                  if (longSolo) ...[
                    const SizedBox(height: 1200),
                    const SizedBox(height: 44, child: Text('Last solo chat')),
                  ],
                ],
              ),
            );
          },
          settingsPageBuilder: _buildSettingsPage,
          hasUnreadInbox: unreadInboxCount > 0,
        ),
      ),
    );
  }

  testWidgets(
    'Messages filters its inventory and opens Search and Settings above it',
    (tester) async {
      await tester.pumpWidget(await buildHome());
      await tester.pumpAndSettle();
      expect(find.byTooltip('Messages'), findsNothing);
      expect(find.byTooltip('Solo'), findsNothing);
      expect(find.text('Personal workspace'), findsOneWidget);
      expect(find.text('General'), findsOneWidget);
      final header = tester.widget<FrostedAppBar>(find.byType(FrostedAppBar));
      expect(header.useNativeNavigation, isFalse);
      expect(header.nativeLargeTitle, isFalse);
      expect(header.alwaysFrosted, isTrue);
      expect(header.titleStyle?.fontSize, 22);
      expect(header.centerTitle, isFalse);

      await tester.tap(
        find.descendant(
          of: find.byType(MessagesFilters),
          matching: find.text('Solo'),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('Personal workspace'), findsOneWidget);
      expect(find.text('General'), findsNothing);
      await tester.tap(
        find.descendant(
          of: find.byType(MessagesFilters),
          matching: find.text('Team'),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('Personal workspace'), findsNothing);
      expect(find.text('General'), findsOneWidget);
      await tester.tap(find.text('All'));
      await tester.pumpAndSettle();
      expect(find.text('Personal workspace'), findsOneWidget);
      await tester.tap(find.text('Chats'));
      await tester.tap(find.text('Channels'));
      await tester.pumpAndSettle();
      for (final filter in ['Solo', 'Team', 'All']) {
        await tester.tap(
          find.descendant(
            of: find.byType(MessagesFilters),
            matching: find.text(filter),
          ),
        );
        await tester.pumpAndSettle();
        expect(find.text('Personal workspace').hitTestable(), findsNothing);
        expect(find.text('General').hitTestable(), findsNothing);
      }
      await tester.tap(find.text('Chats'));
      await tester.tap(find.text('Channels'));
      await tester.pumpAndSettle();
      expect(find.text('Personal workspace').hitTestable(), findsOneWidget);
      expect(find.text('General').hitTestable(), findsOneWidget);
      await tester.tap(find.text('Inbox'));
      await tester.pumpAndSettle();
      expect(find.byType(ActivityPage), findsOneWidget);
      Navigator.of(tester.element(find.byType(ActivityPage))).pop();
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Search'));
      await tester.pumpAndSettle();
      expect(find.byType(SearchPage), findsOneWidget);
      expect(find.byKey(const Key('search-close')), findsOneWidget);
      await tester.tap(find.byKey(const Key('search-close')));
      await tester.pumpAndSettle();
      expect(find.text('General'), findsOneWidget);
      expect(find.byType(MessagesNavigationBar), findsOneWidget);
      await tester.tap(find.byTooltip('Community menu'));
      await tester.pumpAndSettle();
      expect(find.text('Switch workspace'), findsOneWidget);
      expect(find.text('Agents'), findsOneWidget);
      expect(find.text('Settings'), findsOneWidget);
      await tester.tap(find.text('Settings'));
      await tester.pumpAndSettle();
      expect(find.text('Your profile destination'), findsOneWidget);
      Navigator.of(tester.element(find.text('Your profile destination'))).pop();
      await tester.pumpAndSettle();
      expect(find.text('General'), findsOneWidget);
      await tester.pumpWidget(await buildHome(longSolo: true));
      await tester.pumpAndSettle();
      await tester.tap(
        find.descendant(
          of: find.byType(MessagesFilters),
          matching: find.text('Solo'),
        ),
      );
      await tester.pumpAndSettle();
      await tester.drag(
        find.byType(CustomScrollView).first,
        const Offset(0, -2000),
      );
      await tester.pumpAndSettle();
      expect(
        tester.getBottomLeft(find.text('Last solo chat')).dy,
        lessThanOrEqualTo(
          tester.getTopLeft(find.byType(MessagesNavigationBar)).dy,
        ),
      );
    },
  );

  for (final reduced in [false, true]) {
    testWidgets('New opens creation actions with reduced motion $reduced', (
      tester,
    ) async {
      ValueNotifier<VoidCallback?>? chat;
      var created = 0;
      await tester.pumpWidget(
        await buildHome(
          disableAnimations: reduced,
          onNewChatAction: (value) => chat = value,
        ),
      );
      await tester.pumpAndSettle();
      chat!.value = () => created++;
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('New'));
      await tester.pumpAndSettle();
      expect(find.text('New workspace'), findsOneWidget);
      expect(find.text('New DM'), findsOneWidget);
      expect(find.text('New channel'), findsOneWidget);
      await tester.tap(find.text('New chat'));
      await tester.pumpAndSettle();
      expect(created, 1);
    });
  }

  testWidgets(
    'iOS shows Messages above filters with the native creation menu',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      await tester.pumpWidget(await buildHome());
      await tester.pumpAndSettle();
      final bar = tester.widget<FrostedAppBar>(find.byType(FrostedAppBar));
      expect(bar.nativeTitle, '');
      expect(find.text('Messages'), findsOneWidget);
      expect(bar.nativeActions, isEmpty);
      expect(bar.leading, isNull);
      expect(find.byType(MessagesFilters), findsOneWidget);
      expect(bar.bottomHeight, greaterThan(0));
      expect(
        tester
            .widget<IosMessageCreateButton>(
              find.byWidgetPredicate(
                (w) => w is IosMessageCreateButton && w.label == 'New',
              ),
            )
            .actions
            .map((a) => a.label),
        ['New chat', 'New workspace', 'New DM', 'New channel'],
      );
      final communityMenu = tester.widget<IosMessageCreateButton>(
        find.byWidgetPredicate(
          (w) =>
              w is IosMessageCreateButton &&
              w.accessibilityLabel == 'Community menu',
        ),
      );
      communityMenu.actions.last.onPressed!();
      await tester.pump();
      expect(
        find.descendant(
          of: find.byType(IosMessageCreateButton, skipOffstage: false),
          matching: find.byType(UiKitView, skipOffstage: false),
        ),
        findsNothing,
      );
      await tester.pumpAndSettle();
      Navigator.of(tester.element(find.text('Your profile destination'))).pop();
      await tester.pump();
      expect(
        find.descendant(
          of: find.byType(IosMessageCreateButton, skipOffstage: false),
          matching: find.byType(UiKitView, skipOffstage: false),
        ),
        findsNothing,
      );
      await tester.pumpAndSettle();
      expect(
        find.descendant(
          of: find.byType(IosMessageCreateButton),
          matching: find.byType(UiKitView),
        ),
        findsNWidgets(2),
      );
      communityMenu.actions.first.onPressed!();
      await tester.pump();
      // Finish the route entrance while the community inventory is loading.
      await tester.pump(const Duration(milliseconds: 150));
      expect(find.text('Switch Community'), findsOneWidget);
      final homeOpacity = find.byKey(
        const Key('home-community-surface-visibility'),
      );
      expect(tester.widget<Opacity>(homeOpacity).opacity, 0);
      Navigator.of(tester.element(find.text('Switch Community'))).pop();
      await tester.pumpAndSettle();
      expect(tester.widget<Opacity>(homeOpacity).opacity, 1);
      expect(find.text('Messages'), findsOneWidget);
      await tester.pumpWidget(const SizedBox.shrink());
      debugDefaultTargetPlatformOverride = null;
    },
  );
}

Widget _buildSettingsPage(BuildContext context) =>
    const Scaffold(body: Text('Your profile destination'));

class _Channels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [
    Channel(
      id: 'channel',
      name: 'General',
      description: '',
      createdBy: 'owner',
      createdAt: DateTime(2026),
      memberCount: 1,
      visibility: 'open',
      channelType: 'stream',
      isMember: true,
    ),
  ];
}
