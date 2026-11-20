import 'dart:async';

import 'package:buzz/features/channels/bestie_conversation.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/default_agent_launcher.dart';
import 'package:buzz/features/channels/first_agent_callout.dart';
import 'package:buzz/shared/agents/remote_agents.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:buzz/shared/me/default_agent_writer.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/adaptive_conversation_pane.dart';
import 'package:buzz/shared/widgets/agent_navigation_overlay.dart';
import 'package:buzz/shared/widgets/agent_rail_link.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

import '../../shared/me/me_sync_test.dart' show FakeRelay;

void main() {
  // A Duo rail control stands in for the Flutter button and must follow the
  // same guards, so the journey runs once through each.
  final railVariant = ValueVariant<bool>({false, true});
  testWidgets(
    'community switch cannot launch a retained default while loading',
    (tester) async {
      final onRail = railVariant.currentValue!;
      const secret =
          '0000000000000000000000000000000000000000000000000000000000000001';
      final nsec = nostr.Nip19.encode(
        prefix: nostr.Nip19Prefix.nsec,
        data: secret,
      );
      final relayA = FakeRelay();
      final relayB = FakeRelay();
      final a = MeSync(
        relayA,
        RelayConfig(baseUrl: 'https://a.test', nsec: nsec),
      );
      final b = MeSync(
        relayB,
        RelayConfig(baseUrl: 'https://b.test', nsec: nsec),
      );
      await saveDefaultAgent(a, 'a' * 64);
      final pending = Completer<List<NostrEvent>>();
      relayB.pendingRead = pending.future;
      var choices = 0;
      final rail = AgentRailLink();
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      Widget app(MeSync scope) => ProviderScope(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          meSyncProvider.overrideWithValue(scope),
          relaySessionProvider.overrideWith(() => relayA),
          remoteAgentsEnabledProvider.overrideWithValue(true),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: AgentNavigationOverlay(
              active: ValueNotifier(false),
              railLink: onRail ? rail : null,
              child: DefaultAgentLauncher(onChooseAgent: () => choices++),
            ),
          ),
        ),
      );
      await tester.pumpWidget(app(a));
      await tester.pumpAndSettle();
      final button = find.byType(IconButton);
      VoidCallback? current() => onRail
          ? rail.action.value?.onPressed
          : tester.widget<IconButton>(button).onPressed;
      // Hidden behind the native control, the Flutter button takes no touch.
      Future<void> press() async {
        if (onRail) {
          current()!();
        } else {
          await tester.tap(button);
        }
      }

      final staleAction = current()!;
      if (onRail) {
        expect(rail.action.value!.label, 'Chat with your Bestie');
        expect(rail.action.value!.onLongPress, isNotNull);
      }
      await tester.pumpWidget(app(b));
      await tester.pumpAndSettle();
      final container = ProviderScope.containerOf(tester.element(button));
      expect(container.read(defaultAgentProvider).isLoading, isTrue);
      // Riverpod can keep A's previous data while B's first read is pending.
      expect(current(), isNull);
      if (onRail) expect(rail.action.value!.onLongPress, isNull);
      staleAction();
      await tester.pumpAndSettle();
      expect(choices, 0);
      expect(relayB.writes, 0);
      // A failed read can keep A's Bestie as B's value. That must not retire
      // the first-agent card in B, which has no Bestie.
      pending.completeError(StateError('offline'));
      await tester.pumpAndSettle();
      expect(container.read(defaultAgentProvider).hasError, isTrue);
      expect(prefs.getBool(firstAgentCalloutKey(b)), isNot(true));
      relayB.pendingRead = null;
      await press();
      await tester.pumpAndSettle();
      expect(choices, 0);
      expect(prefs.getBool(firstAgentCalloutKey(b)), isNot(true));
      await press();
      expect(choices, 1);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
    },
    variant: railVariant,
  );

  final paneVariant = ValueVariant<bool>({false, true});
  testWidgets('a slow Bestie lookup does not open over a newer screen', (
    tester,
  ) async {
    final nsec = nostr.Nip19.encode(
      prefix: nostr.Nip19Prefix.nsec,
      data: '0' * 63 + '1',
    );
    final relay = FakeRelay();
    final scope = MeSync(
      relay,
      RelayConfig(baseUrl: 'https://a.test', nsec: nsec),
    );
    await saveDefaultAgent(scope, 'a' * 64);
    final actions = _SlowDmActions();
    final pane = ConversationPaneController();
    addTearDown(pane.dispose);
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          meSyncProvider.overrideWithValue(scope),
          relaySessionProvider.overrideWith(() => relay),
          remoteAgentsEnabledProvider.overrideWithValue(true),
          channelsProvider.overrideWith(_NoChannels.new),
          channelActionsProvider.overrideWithValue(actions),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: paneVariant.currentValue!
                ? ConversationPaneScope(
                    controller: pane,
                    child: DefaultAgentLauncher(onChooseAgent: () {}),
                  )
                : DefaultAgentLauncher(onChooseAgent: () {}),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byType(IconButton));
    await tester.pump();
    // The app shortcut asking for the same conversation meanwhile waits on
    // the same lookup instead of starting a second one.
    final launcher = tester.element(find.byType(DefaultAgentLauncher));
    unawaited(
      openBestieConversation(launcher, launcher as WidgetRef, 'a' * 64),
    );
    await tester.pump();
    expect(actions.calls, 1);
    Navigator.of(
      tester.element(find.byType(DefaultAgentLauncher)),
    ).push(MaterialPageRoute<void>(builder: (_) => const Text('Elsewhere')));
    await tester.pumpAndSettle();
    actions.dm.complete(
      Channel(
        id: 'dm',
        name: 'Bestie',
        description: '',
        createdBy: 'self',
        createdAt: DateTime(2026),
        memberCount: 2,
        visibility: 'private',
        channelType: 'dm',
        isMember: true,
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Elsewhere'), findsOneWidget);
    expect(find.byType(ChannelDetailPage), findsNothing);

    // The next open starts a fresh lookup.
    Navigator.of(launcher).pop();
    await tester.pumpAndSettle();
    await tester.tap(find.byType(IconButton));
    await tester.pumpAndSettle();
    expect(actions.calls, 2);
    if (paneVariant.currentValue!) {
      expect(pane.selectedId, 'dm');
      expect(find.byType(ChannelDetailPage), findsNothing);
      expect(find.byType(DefaultAgentLauncher), findsOneWidget);
    } else {
      expect(find.byType(ChannelDetailPage), findsOneWidget);
    }
    await tester.pumpWidget(const SizedBox());
  }, variant: paneVariant);
}

class _NoChannels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => const [];
}

class _SlowDmActions implements ChannelActions {
  final dm = Completer<Channel>();
  var calls = 0;

  @override
  Future<Channel> openDm({
    required List<String> pubkeys,
    bool waitForChannelList = true,
  }) {
    calls++;
    return dm.future;
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
