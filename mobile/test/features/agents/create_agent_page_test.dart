import 'package:buzz/shared/widgets/preset_agent_avatar.dart';
import 'package:buzz/shared/agents/agent_avatar_catalog.dart';
import 'package:buzz/features/agents/agent_avatar_editor_page.dart';
import 'dart:async';

import 'package:buzz/shared/agents/remote_agents.dart';

import 'package:buzz/features/agents/agents_page.dart';
import 'package:buzz/features/agents/create_agent_page.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

import 'agents_harness.dart';

void main() {
  final existing = existingDesktopKeys.public;

  testWidgets(
    'creation overlays the retained list, then failure and resume both land',
    (tester) async {
      final h = await AgentsHarness.create();
      final controller = h.controller;
      final added = nostr.Keys('0' * 63 + '2').public;
      await h.open(tester, directory: [existing]);
      final list = tester.element(find.byType(AgentsPage));
      if (!supportsRemoteAgents(h.community)) {
        expect(find.byTooltip('Add agent'), findsNothing);
        expect(find.text('Sign in to create an agent'), findsNothing);
        expect(find.byKey(ValueKey(existing)), findsOneWidget);
        return;
      }
      await tester.tap(find.byTooltip('Add agent'));
      await tester.pumpAndSettle();
      expect(find.byType(CreateAgentPage), findsOneWidget);
      final createContext = tester.element(find.byType(CreateAgentPage));
      expect(Navigator.of(list), same(Navigator.of(createContext)));

      await tester.ensureVisible(find.text('Model'));
      await tester.tap(find.text('Model'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Test model').last);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Reasoning effort'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('high').last);
      await tester.pumpAndSettle();
      // Switching models must clear an effort unsupported by the new choice.
      await tester.tap(find.text('Test model'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Other model').last);
      await tester.pumpAndSettle();
      expect(
        tester
            .widget<IconButton>(find.widgetWithIcon(IconButton, Icons.check))
            .onPressed,
        isNull,
      );
      await tester.tap(find.text('Other model'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Test model').last);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Reasoning effort'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('high').last);
      await tester.pumpAndSettle();
      controller.gate = Completer<void>();
      await tester.enterText(
        find.descendant(
          of: find.byWidgetPredicate(
            (widget) =>
                widget is Semantics && widget.properties.label == 'Name',
          ),
          matching: find.byType(TextField),
        ),
        'Scout',
      );
      await tester.enterText(
        find.descendant(
          of: find.byWidgetPredicate(
            (widget) =>
                widget is Semantics &&
                widget.properties.label == 'Instructions',
          ),
          matching: find.byType(TextField),
        ),
        'Reply concisely.',
      );
      await tester.pump();
      final originalAvatar = tester
          .widget<PresetAgentAvatar>(find.byType(PresetAgentAvatar))
          .imageUrl;
      final targetIndex = originalAvatar == agentAvatarCatalog.first.url
          ? 1
          : 0;
      await tester.ensureVisible(find.text('Choose avatar'));
      await tester.tap(find.text('Choose avatar'));
      await tester.pumpAndSettle();
      expect(find.byType(AgentAvatarEditorPage), findsOneWidget);
      expect(find.text('Instructions'), findsNothing);
      await tester.tap(find.byKey(ValueKey('agent-avatar-$targetIndex')));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Cancel'));
      await tester.pumpAndSettle();
      expect(
        tester
            .widget<PresetAgentAvatar>(find.byType(PresetAgentAvatar))
            .imageUrl,
        originalAvatar,
      );
      await tester.tap(find.text('Choose avatar'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(ValueKey('agent-avatar-$targetIndex')));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Done'));
      await tester.pumpAndSettle();
      expect(
        tester
            .widget<PresetAgentAvatar>(find.byType(PresetAgentAvatar))
            .imageUrl,
        isNot(originalAvatar),
      );
      expect(find.text('Scout'), findsOneWidget);
      expect(find.text('Reply concisely.'), findsOneWidget);
      await tester.tap(find.byTooltip('Create'));
      await tester.pump(const Duration(milliseconds: 100));
      expect(controller.busy, isTrue);
      expect(
        find.descendant(
          of: find.byType(CreateAgentPage),
          matching: find.byType(LinearProgressIndicator),
        ),
        findsOneWidget,
      );
      expect(
        tester
            .widget<IconButton>(
              find.ancestor(
                of: find.byTooltip('Create'),
                matching: find.byType(IconButton),
              ),
            )
            .onPressed,
        isNull,
      );
      int catalogLoads() =>
          h.registry.calls.where((call) => call == 'get-model-catalog').length;
      final loadsBeforeFailure = catalogLoads();
      controller.gate!.complete();
      await tester.pumpAndSettle();
      expect(find.byType(CreateAgentPage), findsOneWidget);
      // A rejected choice may be stale, so the catalog reloads.
      expect(catalogLoads(), loadsBeforeFailure + 1);
      expect(find.byType(CreateAgentPage), findsOneWidget);
      expect(find.text('Creation failed. Retry.'), findsOneWidget);
      expect(controller.submittedName, 'Scout');
      expect(controller.submittedInstructions, 'Reply concisely.');

      // The real registry flow then succeeds and returns to the same list.
      controller.gate = null;
      await tester.tap(find.byTooltip('Create'));
      await tester.pumpAndSettle();
      expect(find.byType(BottomSheet), findsNothing);
      expect(tester.element(find.byType(AgentsPage)), same(list));
      expect(find.byKey(ValueKey(added)), findsOneWidget);
      expect(h.registry.calls, contains('publish-policy'));
    },
  );
}
