import 'package:buzz/shared/agents/remote_agents.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/default_agent_launcher.dart';
import 'package:buzz/features/agents/agents_sheet.dart';
import 'package:buzz/features/profile/user_profile_sheet.dart';
import 'package:flutter/material.dart';
import 'package:buzz/features/home/home_page.dart';
import 'package:flutter_test/flutter_test.dart';

import 'agents_harness.dart';

void main() {
  final existing = 'a' * 64;

  testWidgets('agent detail pushes inside one modal and Back restores list', (
    tester,
  ) async {
    final h = await AgentsHarness.create();
    h.registry.listed = [
      {
        'agent_pubkey': existing,
        'agent_id': 'remote-id',
        'agent_name': 'Scout',
      },
    ];
    await h.controller.run(h.controller.refresh);
    await h.open(tester, directory: [existing]);
    if (!supportsRemoteAgents(h.community)) {
      expect(find.text('Sign in to create an agent'), findsNothing);
      for (final button in tester.widgetList<IconButton>(
        find.byWidgetPredicate(
          (widget) => widget is IconButton && widget.tooltip == 'Add agent',
        ),
      )) {
        expect(button.onPressed, isNull);
      }
    }
    final list = tester.element(find.byType(AgentsSheet));
    final container = ProviderScope.containerOf(list);
    expect(
      find.text('Choose or change your Bestie on desktop.'),
      findsOneWidget,
    );
    expect(
      find.descendant(
        of: find.byKey(ValueKey(existing)),
        matching: find.byType(IconButton),
      ),
      findsNothing,
    );
    expect(h.relay.events, isEmpty);

    await tester.tap(find.byKey(ValueKey(existing)));
    await tester.pump();
    for (var frame = 0; frame < 3; frame++) {
      await tester.pump(const Duration(milliseconds: 60));
      final surface = find.byKey(const ValueKey('agent-detail-surface'));
      expect(tester.widget<Material>(surface).color?.a, 1);
      expect(tester.takeException(), isNull);
    }
    await tester.pumpAndSettle();
    expect(find.byType(BottomSheet), findsOneWidget);
    expect(find.byType(UserProfileSheet), findsOneWidget);
    expect(find.text('Delete'), findsNothing);
    expect(find.textContaining('before deleting'), findsNothing);
    expect(list.mounted, isTrue);
    await tester.tap(find.byTooltip('Back'));
    await tester.pumpAndSettle();
    expect(find.byType(UserProfileSheet), findsNothing);
    expect(find.byKey(ValueKey(existing)), findsOneWidget);
    expect(tester.element(find.byType(AgentsSheet)), same(list));
    ScaffoldMessenger.of(list).clearSnackBars();
    await tester.tap(find.byTooltip('Close'));
    await tester.pumpAndSettle();
    expect(find.byType(BottomSheet), findsNothing);
    h.relay.events = [];
    final sync = container.read(meSyncProvider);
    await sync.prefs!.remove('main-agent:${sync.community}:${sync.pubkey}');
    await h.open(
      tester,
      directory: [existing],
      home: HomePage(
        onAgents: showAgents,
        defaultAgentBuilder: (context) =>
            DefaultAgentLauncher(onChooseAgent: () => showAgents(context)),
        hasUnreadInbox: false,
        settingsPageBuilder: (_) => const SizedBox.shrink(),
        mePageBuilder: (_, _, _) =>
            const SliverToBoxAdapter(child: Text('Solo workspace')),
      ),
    );
    await tester.tap(find.byTooltip('Bestie'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(ValueKey(existing)));
    await tester.pumpAndSettle();
    expect(find.byType(UserProfileSheet), findsOneWidget);
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    expect(find.byType(UserProfileSheet), findsNothing);
    await tester.tap(find.byTooltip('Close'));
    await tester.pumpAndSettle();
    expect(find.byTooltip('Search'), findsOneWidget);
  });
}
