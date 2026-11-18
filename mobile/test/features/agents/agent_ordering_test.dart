import 'package:buzz/features/agents/agent_card.dart';
import 'package:buzz/features/agents/agents_page.dart';
import 'package:buzz/shared/identity_archives/identity_archives.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'agents_harness.dart';
import '../../helpers/remote_agents_fixture.dart';

void main() {
  testWidgets('directory sorts names ignoring case and reorders after rename', (
    tester,
  ) async {
    final h = await AgentsHarness.create();
    final existing = existingDesktopKeys.public;
    await h.open(tester, directory: [existing, fakeAgent]);
    final container = ProviderScope.containerOf(
      tester.element(find.byType(AgentsPage)),
    );
    final profiles = container.read(userCacheProvider.notifier);
    profiles.put(UserProfile(pubkey: existing, displayName: 'Zulu'));
    profiles.put(UserProfile(pubkey: fakeAgent, displayName: 'alpha'));
    await tester.pumpAndSettle();
    expect(
      tester
          .widgetList<AgentCard>(find.byType(AgentCard))
          .map((card) => card.name),
      ['alpha', 'Zulu'],
    );
    profiles.put(UserProfile(pubkey: existing, displayName: 'aardvark'));
    await tester.pumpAndSettle();
    expect(
      tester
          .widgetList<AgentCard>(find.byType(AgentCard))
          .map((card) => card.name),
      ['aardvark', 'alpha'],
    );
  });

  testWidgets('leaves archived agents out of the directory until restored', (
    tester,
  ) async {
    final h = await AgentsHarness.create();
    final existing = existingDesktopKeys.public;
    var archived = {fakeAgent};
    await h.open(
      tester,
      directory: [existing, fakeAgent],
      overrides: [
        identityArchivesProvider.overrideWith((ref) async => archived),
      ],
    );
    final container = ProviderScope.containerOf(
      tester.element(find.byType(AgentsPage)),
    );
    final profiles = container.read(userCacheProvider.notifier);
    profiles.put(UserProfile(pubkey: existing, displayName: 'Zulu'));
    profiles.put(UserProfile(pubkey: fakeAgent, displayName: 'alpha'));
    await tester.pumpAndSettle();
    expect(
      tester
          .widgetList<AgentCard>(find.byType(AgentCard))
          .map((card) => card.name),
      ['Zulu'],
    );
    // Restored elsewhere: pulling to refresh rereads the archive list.
    archived = {};
    await tester.fling(
      find.byType(CustomScrollView).first,
      const Offset(0, 400),
      1000,
    );
    await tester.pumpAndSettle();
    expect(
      tester
          .widgetList<AgentCard>(find.byType(AgentCard))
          .map((card) => card.name),
      ['alpha', 'Zulu'],
    );
  });
}
