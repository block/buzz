import 'package:buzz/features/onboarding/hosted_community_selection.dart';
import 'package:buzz/shared/community/community_grid_option.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/hosted_communities_fixture.dart';
import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('picker shortens hosted fallback labels and preserves titles', (
    tester,
  ) async {
    FlutterSecureStorage.setMockInitialValues({});
    final fixture = HostedCommunitiesFixture();
    await fixture.initialize(widget: true);
    addTearDown(fixture.dispose);

    for (final (host, name, expected) in [
      ('mobile.communities.buzz.xyz', 'mobile.communities.buzz.xyz', 'mobile'),
      (
        'mobile.communities.public.builderlabstaging.xyz',
        'mobile.communities.public.builderlabstaging.xyz',
        'mobile',
      ),
      ('mobile.communities.buzz.xyz', 'Mobile Team', 'Mobile Team'),
      ('chat.example.com', 'chat.example.com', 'chat.example.com'),
      (
        'mobile.communities.buzz.xyz.example.com',
        'mobile.communities.buzz.xyz.example.com',
        'mobile.communities.buzz.xyz.example.com',
      ),
      ('communities.buzz.xyz', 'communities.buzz.xyz', 'communities.buzz.xyz'),
    ]) {
      final relay = 'wss://$host';
      fixture.flow.communities = [
        AddCommunityDeepLink(relayUrl: relay, name: name),
      ];
      await tester.pumpWidget(
        WidgetHelpers.testable(
          child: HostedCommunitySelection(flow: fixture.flow, fullScreen: true),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text(expected), findsOneWidget);
      expect(find.bySemanticsLabel(expected), findsOneWidget);
      final tile = tester.widget<CommunityGridOption>(
        find.byType(CommunityGridOption),
      );
      expect(tile.relayUrl, relay);
      expect(fixture.flow.communities.single.name, name);
      if (name != expected) expect(find.text(name), findsNothing);
      await tester.pumpWidget(const SizedBox.shrink());
    }
  }, semanticsEnabled: true);
}
