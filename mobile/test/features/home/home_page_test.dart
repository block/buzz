import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_page.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/mentions/mention_candidates_provider.dart';
import 'package:buzz/features/home/home_page.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  Future<Widget> buildHome({List<Channel>? channels}) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    return ProviderScope(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        agentDirectoryProvider.overrideWith((ref) async => const []),
        if (channels != null)
          channelsProvider.overrideWith(() => _FakeChannelsNotifier(channels)),
      ],
      child: MaterialApp(
        theme: AppTheme.light(),
        home: const HomePage(settingsPageBuilder: _buildSettingsPage),
      ),
    );
  }

  void useSurfaceSize(WidgetTester tester, Size size) {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = size;
    addTearDown(tester.view.resetDevicePixelRatio);
    addTearDown(tester.view.resetPhysicalSize);
  }

  testWidgets('shows icon-only navigation and an aligned quick action', (
    tester,
  ) async {
    useSurfaceSize(tester, const Size(390, 844));
    await tester.pumpWidget(await buildHome());
    await tester.pump();

    expect(find.text('Home'), findsNothing);
    expect(find.text('Activity'), findsNothing);
    expect(find.text('Search'), findsNothing);
    expect(find.bySemanticsLabel('Home'), findsOneWidget);
    expect(find.bySemanticsLabel('Activity'), findsOneWidget);
    expect(find.bySemanticsLabel('Search'), findsOneWidget);

    final quickAction = find.byTooltip('Create or start conversation');
    expect(quickAction, findsOneWidget);
    final launcherSize = tester.getSize(
      find.byType(ChannelQuickActionsLauncher),
    );
    expect(launcherSize.width, 390);
    expect(launcherSize.height, greaterThan(0));
    final motionRect = tester.getRect(
      find.byKey(const Key('channel-quick-actions-motion')),
    );
    expect(motionRect.width, const Size.square(56).width);
    expect(motionRect.left, greaterThanOrEqualTo(0));
    expect(tester.getSize(quickAction), const Size.square(56));
    final quickActionRect = tester.getRect(quickAction);
    expect(quickActionRect.left, greaterThanOrEqualTo(0));
    expect(quickActionRect.top, greaterThanOrEqualTo(0));
    expect(quickActionRect.right, lessThanOrEqualTo(390));
    expect(quickActionRect.bottom, lessThanOrEqualTo(844));
    final homeDestinationRect = tester.getRect(find.bySemanticsLabel('Home'));
    expect(
      quickActionRect.center.dy,
      closeTo(homeDestinationRect.center.dy, 0.01),
    );
  });

  testWidgets('gives selection haptics only when the tab changes', (
    tester,
  ) async {
    useSurfaceSize(tester, const Size(390, 844));
    final hapticCalls = <MethodCall>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, (call) async {
          if (call.method == 'HapticFeedback.vibrate') {
            hapticCalls.add(call);
          }
          return null;
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(SystemChannels.platform, null),
    );

    await tester.pumpWidget(await buildHome());
    await tester.pump();

    await tester.tap(find.byTooltip('Home'));
    await tester.pump();
    expect(hapticCalls, isEmpty);

    await tester.tap(find.byTooltip('Activity'));
    await tester.pump();
    expect(hapticCalls, hasLength(1));
    expect(hapticCalls.single.arguments, 'HapticFeedbackType.selectionClick');

    await tester.tap(find.byTooltip('Activity'));
    await tester.pump();
    expect(hapticCalls, hasLength(1));

    await tester.tap(find.byTooltip('Search'));
    await tester.pump();
    expect(hapticCalls, hasLength(2));
  });

  testWidgets('scales and fades the quick action as tabs change', (
    tester,
  ) async {
    useSurfaceSize(tester, const Size(390, 844));
    await tester.pumpWidget(await buildHome());
    await tester.pump();

    double scale() => tester
        .widget<Transform>(find.byKey(const Key('channel-quick-actions-scale')))
        .transform
        .storage
        .first;
    double opacity() => tester
        .widget<Opacity>(find.byKey(const Key('channel-quick-actions-opacity')))
        .opacity;

    expect(scale(), closeTo(1, 0.001));
    expect(opacity(), closeTo(1, 0.001));

    await tester.tap(find.byTooltip('Activity'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 110));

    expect(scale(), inExclusiveRange(0.8, 1));
    expect(opacity(), inExclusiveRange(0, 1));

    await tester.pumpAndSettle();
    expect(scale(), closeTo(0.8, 0.001));
    expect(opacity(), closeTo(0, 0.001));

    await tester.tap(find.byTooltip('Home'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 110));

    expect(scale(), inExclusiveRange(0.8, 1));
    expect(opacity(), inExclusiveRange(0, 1));

    await tester.pumpAndSettle();
    expect(scale(), closeTo(1, 0.001));
    expect(opacity(), closeTo(1, 0.001));
  });

  testWidgets('keeps compact navigation in a narrow iPad split view', (
    tester,
  ) async {
    useSurfaceSize(tester, const Size(599, 1024));
    await tester.pumpWidget(await buildHome());
    await tester.pump();

    expect(find.byKey(const Key('mobile-navigation-bar')), findsOneWidget);
    expect(find.byKey(const Key('tablet-navigation-rail')), findsNothing);
  });

  testWidgets('uses rail navigation in a medium tablet window', (tester) async {
    useSurfaceSize(tester, const Size(650, 1024));
    await tester.pumpWidget(await buildHome());
    await tester.pump();

    expect(find.byKey(const Key('tablet-navigation-rail')), findsOneWidget);
    expect(find.byKey(const Key('tablet-workspace-sidebar')), findsNothing);
    expect(find.byKey(const Key('mobile-navigation-bar')), findsNothing);
  });

  testWidgets('reserves 420 points before exposing the workspace sidebar', (
    tester,
  ) async {
    useSurfaceSize(tester, const Size(700, 1024));
    await tester.pumpWidget(await buildHome());
    await tester.pump();

    expect(find.byKey(const Key('tablet-navigation-rail')), findsOneWidget);
    expect(find.byKey(const Key('tablet-workspace-sidebar')), findsNothing);

    tester.view.physicalSize = const Size(701, 1024);
    await tester.pump();

    expect(find.byKey(const Key('tablet-navigation-rail')), findsNothing);
    expect(find.byKey(const Key('tablet-workspace-sidebar')), findsOneWidget);
  });

  testWidgets('shows the complete workspace sidebar on iPad', (tester) async {
    useSurfaceSize(tester, const Size(768, 1024));
    await tester.pumpWidget(await buildHome());
    await tester.pump();

    final sidebar = find.byKey(const Key('tablet-workspace-sidebar'));
    expect(sidebar, findsOneWidget);
    expect(find.byKey(const Key('tablet-navigation-rail')), findsNothing);
    expect(find.byKey(const Key('mobile-navigation-bar')), findsNothing);
    for (final label in [
      'Inbox',
      'Agents',
      'Channels',
      'Direct Messages',
      'Community',
      'Your profile',
    ]) {
      expect(
        find.descendant(of: sidebar, matching: find.text(label)),
        findsOneWidget,
      );
    }
    expect(find.text('Home'), findsNothing);
    expect(find.text('Activity'), findsNothing);
    expect(find.byKey(const Key('tablet-profile-footer')), findsOneWidget);

    final labelColumn = tester
        .getTopLeft(find.descendant(of: sidebar, matching: find.text('Inbox')))
        .dx;
    for (final label in [
      'Community',
      'Agents',
      'Channels',
      'Direct Messages',
      'Your profile',
    ]) {
      final labelFinder = find.descendant(
        of: sidebar,
        matching: find.text(label),
      );
      expect(tester.getTopLeft(labelFinder).dx, closeTo(labelColumn, 0.01));
    }

    await tester.tap(
      find.descendant(of: sidebar, matching: find.text('Agents')),
    );
    await tester.pump();

    expect(find.byKey(const Key('agents-page')), findsOneWidget);
  });

  testWidgets('opens a selected channel beside the iPad sidebar', (
    tester,
  ) async {
    useSurfaceSize(tester, const Size(1194, 834));
    final channel = Channel(
      id: 'general',
      name: 'general',
      channelType: 'stream',
      visibility: 'open',
      description: 'General discussion',
      createdBy: 'owner',
      createdAt: DateTime(2026),
      memberCount: 4,
      isMember: true,
    );
    await tester.pumpWidget(await buildHome(channels: [channel]));
    await tester.pump();

    final sidebar = find.byKey(const Key('tablet-workspace-sidebar'));
    await tester.tap(
      find.descendant(of: sidebar, matching: find.text('general')),
    );
    await tester.pump();

    expect(
      find.byKey(const ValueKey('tablet-channel-general')),
      findsOneWidget,
    );
    final selectedTile = tester.widget<Ink>(
      find.byKey(const ValueKey('channel-tile-general')),
    );
    expect((selectedTile.decoration as BoxDecoration).color, isNotNull);
  });

  testWidgets('opens settings from the pinned iPad profile footer', (
    tester,
  ) async {
    useSurfaceSize(tester, const Size(768, 1024));
    await tester.pumpWidget(await buildHome());
    await tester.pump();

    await tester.tap(find.byKey(const Key('tablet-profile-footer')));
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('settings-page')), findsOneWidget);
  });

  testWidgets('keeps content and quick actions inside an iPad landscape pane', (
    tester,
  ) async {
    useSurfaceSize(tester, const Size(1194, 834));
    await tester.pumpWidget(await buildHome());
    await tester.pump();

    expect(find.byKey(const Key('tablet-workspace-sidebar')), findsOneWidget);
    expect(find.byKey(const Key('tablet-navigation-rail')), findsNothing);

    final contentRect = tester.getRect(
      find.byKey(const Key('tablet-page-content')),
    );
    expect(contentRect.width, 840);
    expect(contentRect.left, greaterThan(200));
    expect(contentRect.right, lessThan(1194));

    final quickAction = find.byTooltip('Create or start conversation');
    final closedRect = tester.getRect(quickAction);
    expect(contentRect.contains(closedRect.topLeft), isTrue);
    expect(contentRect.contains(closedRect.bottomRight), isTrue);

    await tester.tap(quickAction);
    await tester.pumpAndSettle();

    final openRect = tester.getRect(
      find.byKey(const Key('quick-actions-surface')),
    );
    expect(openRect.left, greaterThan(contentRect.left));
    expect(openRect.right, lessThan(contentRect.right));
  });
}

Widget _buildSettingsPage(BuildContext context) =>
    const SizedBox(key: Key('settings-page'));

class _FakeChannelsNotifier extends ChannelsNotifier {
  _FakeChannelsNotifier(this.channels);

  final List<Channel> channels;

  @override
  Future<List<Channel>> build() async => channels;
}
