import 'package:buzz/features/channels/agent_activity/agent_activity_sheet.dart';
import 'package:buzz/features/channels/agent_activity/observer_subscription.dart';
import 'package:buzz/features/channels/channel_identity_names_provider.dart';
import 'package:buzz/shared/agents/remote_agents.dart';
import 'package:buzz/shared/identity_names/identity_names.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const _agent =
    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

class _Cache extends UserCacheNotifier {
  @override
  Map<String, UserProfile> build() => {};
  @override
  Future<bool> preload(List<String> pubkeys) async => true;
}

void main() {
  testWidgets('the header says Remote only for a confirmed remote agent', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    Future<void> pump({required bool remote}) async {
      // A fresh scope, since overrides are fixed for a scope's lifetime.
      await tester.pumpWidget(const SizedBox());
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            observerSubscriptionProvider.overrideWith(
              (ref, key) => const ObserverState.initial(),
            ),
            channelIdentityNamesProvider('chat').overrideWithValue(
              IdentityNameSources.empty.scope(
                [_agent],
                fallbackNames: {_agent: 'Cloud Bot'},
              ),
            ),
            userCacheProvider.overrideWith(_Cache.new),
            availableRemoteAgentsProvider.overrideWith(
              (ref) async => remote ? {_agent} : <String>{},
            ),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: const Scaffold(
              body: AgentActivitySheet(channelId: 'chat', agentPubkey: _agent),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
    }

    await pump(remote: false);
    expect(find.text('Cloud Bot'), findsOneWidget);
    expect(find.text(' · Remote'), findsNothing);

    await pump(remote: true);
    final label = find.text(' · Remote');
    expect(label, findsOneWidget);
    // VoiceOver hears the word, not the separator.
    expect(find.bySemanticsLabel('Remote'), findsOneWidget);
    await tester.longPress(label);
    await tester.pumpAndSettle();
    expect(find.text('Runs remotely on Builderlab'), findsOneWidget);
    expect(tester.takeException(), isNull);
    semantics.dispose();
  });
}
