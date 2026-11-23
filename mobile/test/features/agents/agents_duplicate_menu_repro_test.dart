// Investigation test for duplicate Community menu -> Agents routes. Before the
// guard fix in agents_page.dart this test failed with three stacked Agents
// pages; with the fix it passes with one. Delete or rewrite once the upstream
// regression test exists.
import 'package:buzz/features/agents/agents_page.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/home/home_page.dart';
import 'package:buzz/features/activity/inbox_unread_provider.dart';
import 'package:buzz/shared/relay/app_lifecycle_provider.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'agents_harness.dart';

void main() {
  testWidgets('queued native Agents selections open one Agents page', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(390, 844);
    addTearDown(tester.view.reset);
    final messenger = tester.binding.defaultBinaryMessenger;
    final nativeChannels = <MethodChannel>[];
    MethodChannel? communityMenu;
    int? agentsIndex;
    messenger.setMockMethodCallHandler(SystemChannels.platform_views, (
      call,
    ) async {
      if (call.method != 'create') return null;
      final args = call.arguments as Map;
      final params = const StandardMessageCodec().decodeMessage(
        ByteData.sublistView(args['params'] as Uint8List),
      );
      if (params is! Map) return null;
      final channel = MethodChannel('${args['viewType']}/${args['id']}');
      nativeChannels.add(channel);
      messenger.setMockMethodCallHandler(channel, (_) async => null);
      if (params['accessibilityLabel'] == 'Community menu') {
        communityMenu = channel;
        final menu = params['menu'] as List;
        agentsIndex =
            (menu.singleWhere((item) => (item as Map)['label'] == 'Agents')
                    as Map)['id']
                as int;
      }
      return null;
    });
    addTearDown(() {
      messenger.setMockMethodCallHandler(SystemChannels.platform_views, null);
      for (final channel in nativeChannels) {
        messenger.setMockMethodCallHandler(channel, null);
      }
    });

    final h = await AgentsHarness.create();
    final prefs = await SharedPreferences.getInstance();
    await h.open(
      tester,
      directory: [],
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        channelsProvider.overrideWith(_EmptyChannels.new),
        appLifecycleProvider.overrideWith(_Foreground.new),
        inboxHasUnreadProvider.overrideWithValue(false),
      ],
      home: HomePage(
        onAgents: showAgents,
        openBestieConversation: (_, _, _) async {},
        defaultAgentBuilder: (_) => const SizedBox.shrink(),
        settingsPageBuilder: (_) => const SizedBox.shrink(),
        mePageBuilder: (_, _, _) =>
            const SliverToBoxAdapter(child: Text('Investigation home')),
      ),
    );
    expect(communityMenu, isNotNull);
    expect(agentsIndex, isNotNull);
    expect(find.byType(AgentsPage, skipOffstage: false), findsNothing);

    // Model independently delivered native menu selections before Flutter's
    // next frame removes the covered Home control. No sleeps or fake navigator.
    final source = communityMenu!;
    for (var i = 0; i < 3; i++) {
      await messenger.handlePlatformMessage(
        source.name,
        const StandardMethodCodec().encodeMethodCall(
          MethodCall('menuAction', agentsIndex),
        ),
        (_) {},
      );
    }
    await tester.pumpAndSettle();
    expect(find.byType(AgentsPage, skipOffstage: false), findsOneWidget);
    // Closing returns Home, and a later selection opens Agents again.
    tester.widget<AgentsPage>(find.byType(AgentsPage)).onClose();
    await tester.pumpAndSettle();
    expect(find.byType(AgentsPage, skipOffstage: false), findsNothing);
    expect(find.byType(HomePage), findsOneWidget);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox.shrink());
    debugDefaultTargetPlatformOverride = null;
  });
}

// Unrelated channel data and OS lifecycle are isolated from this navigation
// investigation. Agents controller, native bridge, Home callbacks and routes
// are the production implementations supplied by the existing test harness.
class _EmptyChannels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [];
}

class _Foreground extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}
