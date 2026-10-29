import 'package:buzz/shared/agents/remote_agents.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/default_agent_launcher.dart';
import 'package:buzz/features/agents/agents_page.dart';
import 'package:buzz/features/profile/user_profile_sheet.dart';
import 'package:flutter/material.dart';
import 'package:buzz/features/home/home_page.dart';
import 'package:flutter_test/flutter_test.dart';

import 'agents_harness.dart';
import '../../helpers/remote_agents_fixture.dart';

void main() {
  final existing = existingDesktopKeys.public;

  testWidgets('agent profile sheet closes back to the full-page directory', (
    tester,
  ) async {
    final h = await AgentsHarness.create();
    await h.open(
      tester,
      directory: [existing, otherAgentKeys.public],
      botDirectory: ['b' * 64],
    );
    expect(find.byKey(ValueKey(otherAgentKeys.public)), findsNothing);
    expect(find.byKey(ValueKey('b' * 64)), findsNothing);
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
    await h.controller.run(() async {
      throw StateError('Resume unfinished setup');
    });
    await tester.pumpAndSettle();
    expect(find.textContaining('Resume unfinished setup'), findsNothing);
    expect(find.byType(LinearProgressIndicator), findsNothing);
    final list = tester.element(find.byType(AgentsPage));
    final container = ProviderScope.containerOf(list);
    expect(find.text('Set as default'), findsNothing);
    expect(find.text('Model not shared'), findsNothing);
    expect(find.byIcon(Icons.star), findsNothing);
    expect(find.text('Manage on desktop'), findsOneWidget);
    expect(find.byIcon(Icons.keyboard_arrow_down), findsNothing);

    await tester.tap(find.byKey(ValueKey(existing)));
    await tester.pump();
    await tester.pumpAndSettle();
    expect(find.byType(BottomSheet), findsOneWidget);
    expect(find.byType(UserProfileSheet), findsOneWidget);
    expect(find.text('Model not shared'), findsNothing);
    expect(list.mounted, isTrue);
    expect(find.byTooltip('Delete agent'), findsNothing);
    h.relay.loseAcknowledgment = true;
    await tester.ensureVisible(find.text('Make Bestie'));
    await tester.tap(find.text('Make Bestie'));
    await tester.pumpAndSettle();
    expect(find.textContaining('Could not save Bestie.'), findsOneWidget);
    h.relay.loseAcknowledgment = false;
    h.relay.events = []; // The unacknowledged write was not retained.
    await tester.ensureVisible(find.text('Make Bestie'));
    await tester.tap(find.text('Make Bestie'));
    await tester.pumpAndSettle();
    expect(find.text('Your Bestie'), findsOneWidget);
    final savedSync = container.read(meSyncProvider);
    expect(decodeDefaultAgent(h.relay.events.single, savedSync), existing);
    expect(
      savedSync.prefs!.containsKey(
        'main-agent:${savedSync.community}:${savedSync.pubkey}',
      ),
      isTrue,
    );

    await tester.tap(find.byTooltip('Close sheet'));
    await tester.pumpAndSettle();
    expect(find.byType(UserProfileSheet), findsNothing);
    expect(find.byKey(ValueKey(existing)), findsOneWidget);
    expect(tester.element(find.byType(AgentsPage)), same(list));
    if (supportsRemoteAgents(h.community)) {
      h.registry.listed = [
        {
          'agent_pubkey': fakeAgent,
          'agent_id': 'remote-id',
          'agent_name': 'Scout',
        },
      ];
      await h.controller.run(h.controller.refresh);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(ValueKey(fakeAgent)));
      await tester.pumpAndSettle();
      expect(find.byTooltip('Delete agent'), findsOneWidget);
      expect(
        tester.getCenter(find.byTooltip('Delete agent')).dx,
        lessThan(tester.getCenter(find.byTooltip('Close sheet')).dx),
      );
      await tester.tap(find.byTooltip('Delete agent'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      h.relay.events = [];
      await tester.ensureVisible(find.text('Make Bestie'));
      await tester.tap(find.text('Make Bestie'));
      await tester.pumpAndSettle();
      expect(find.text('Your Bestie'), findsOneWidget);
      expect(decodeDefaultAgent(h.relay.events.single, savedSync), fakeAgent);
      await tester.tap(find.byTooltip('Close sheet'));
      await tester.pumpAndSettle();
    }
    ScaffoldMessenger.of(list).clearSnackBars();
    await tester.tap(find.byTooltip('Close agents'));
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
    expect(find.text('Model not shared'), findsNothing);
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    expect(find.byType(UserProfileSheet), findsNothing);
    await tester.tap(find.byTooltip('Close agents'));
    await tester.pumpAndSettle();
    expect(find.byTooltip('Search'), findsOneWidget);
  });
}
