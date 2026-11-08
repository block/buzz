import 'package:buzz/shared/deeplink/pending_deep_link_provider.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'dart:async';

import 'package:buzz/features/channels/channels_page.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/community/community_icon_provider.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/hosted/known_communities_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/hosted/hosted_flow_provider.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import '../../helpers/hosted_communities_fixture.dart';

import '../../helpers/widget_helpers.dart';

final _saved = Community(
  id: 'saved',
  name: 'Saved community',
  relayUrl: 'wss://saved.communities.buzz.xyz',
  addedAt: DateTime(2025),
);
const _discovered = AddCommunityDeepLink(
  name: 'Account community',
  relayUrl: 'wss://other.communities.buzz.xyz',
);

class _SavedCommunities extends CommunityListNotifier {
  @override
  Future<List<Community>> build() async => [_saved];
}

Widget _page(Future<List<AddCommunityDeepLink>> Function() load) =>
    WidgetHelpers.testable(
      overrides: [
        communityListProvider.overrideWith(_SavedCommunities.new),
        activeCommunityProvider.overrideWith((ref) async => _saved),
        knownCommunitiesProvider.overrideWith((ref) => load()),
        communityIconPresentationProvider.overrideWith((ref, relay) => null),
      ],
      child: CommunitySwitcherPage(
        destination: null,
        prepareLanding: () async => null,
        onFlightChanged: (_) {},
        onTransitionProgress: (_) {},
      ),
    );

void main() {
  testWidgets('shows account destinations alongside saved communities once', (
    tester,
  ) async {
    final discovery = Completer<List<AddCommunityDeepLink>>();
    await tester.pumpWidget(_page(() => discovery.future));
    await tester.pump();
    expect(find.text('Saved community'), findsOneWidget);
    expect(find.byKey(const Key('community-switcher-add')), findsOneWidget);
    discovery.complete([
      AddCommunityDeepLink(name: 'Duplicate', relayUrl: '${_saved.relayUrl}/'),
      _discovered,
    ]);
    await tester.pumpAndSettle();
    expect(find.text('Saved community'), findsOneWidget);
    expect(find.text('Account community'), findsOneWidget);
    expect(find.text('Duplicate'), findsNothing);
    expect(find.text('Find my communities'), findsNothing);
    await tester.tap(find.text('Edit'));
    await tester.pumpAndSettle();
    // Edit exposes removal for both local and account-saved destinations.
    expect(find.byTooltip('Remove Saved community'), findsOneWidget);
    expect(find.byTooltip('Remove Account community'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  for (final adding in [false, true]) {
    testWidgets(
      adding
          ? 'switcher Add omits redundant account discovery'
          : 'account selection enters only the chosen community through admission',
      (tester) async {
        FlutterSecureStorage.setMockInitialValues({});
        final fixture = HostedCommunitiesFixture();
        await fixture.initialize(widget: true);
        fixture.rows = [fixture.row('other')];
        addTearDown(fixture.dispose);
        await tester.pumpWidget(
          WidgetHelpers.testable(
            overrides: [
              communityListProvider.overrideWith(_SavedCommunities.new),
              activeCommunityProvider.overrideWith((ref) async => _saved),
              knownCommunitiesProvider.overrideWith(
                (ref) async => [_discovered],
              ),
              communityIconPresentationProvider.overrideWith(
                (ref, relay) => null,
              ),
              hostedFlowProvider.overrideWithValue(fixture.flow),
            ],
            child: Builder(
              builder: (context) => TextButton(
                onPressed: () => Navigator.of(context).push(
                  MaterialPageRoute<void>(
                    builder: (_) => CommunitySwitcherPage(
                      destination: null,
                      prepareLanding: () async => null,
                      onFlightChanged: (_) {},
                      onTransitionProgress: (_) {},
                    ),
                  ),
                ),
                child: const Text('Open switcher'),
              ),
            ),
          ),
        );
        await tester.tap(find.text('Open switcher'));
        await tester.pumpAndSettle();
        expect(fixture.admitted, isEmpty);
        if (adding) {
          await tester.tap(find.byKey(const Key('community-switcher-add')));
          await tester.pumpAndSettle();
          expect(find.text('Enter a community link'), findsOneWidget);
          expect(find.text('Find my communities'), findsNothing);
          expect(fixture.admitted, isEmpty);
        } else {
          await tester.tap(find.text('Account community'));
          await tester.pumpAndSettle();
          expect(fixture.admitted, ['other.communities.buzz.xyz']);
        }
        expect(find.byKey(const Key('community-switcher-page')), findsNothing);
        expect(tester.takeException(), isNull);
      },
    );
  }

  testWidgets(
    'external account destination uses the existing self-hosted entry dispatcher',
    (tester) async {
      PendingDeepLinkNotifier.debugUriStreamOverride =
          const Stream<Uri>.empty();
      addTearDown(() => PendingDeepLinkNotifier.debugUriStreamOverride = null);
      const external = AddCommunityDeepLink(
        name: 'External',
        relayUrl: 'wss://team.example:8443',
      );
      await tester.pumpWidget(_page(() async => [external]));
      await tester.pumpAndSettle();
      final container = ProviderScope.containerOf(
        tester.element(find.byType(CommunitySwitcherPage)),
      );
      final subscription = container.listen(pendingDeepLinkProvider, (_, _) {});
      addTearDown(subscription.close);
      await tester.tap(find.text('External'));
      await tester.pumpAndSettle();
      expect(
        container.read(pendingDeepLinkProvider),
        isA<AddCommunityDeepLink>().having(
          (c) => c.relayUrl,
          'relay',
          external.relayUrl,
        ),
      );
    },
  );

  testWidgets('failed discovery keeps saved choices and Add, and can retry', (
    tester,
  ) async {
    var attempts = 0;
    await tester.pumpWidget(
      _page(() async {
        if (++attempts == 1) throw StateError('offline');
        return [_discovered];
      }),
    );
    await tester.pumpAndSettle();
    expect(find.text('Saved community'), findsOneWidget);
    expect(find.byKey(const Key('community-switcher-add')), findsOneWidget);
    expect(
      find.text('Could not load your account communities.'),
      findsOneWidget,
    );
    await tester.tap(find.text('Try again'));
    await tester.pumpAndSettle();
    expect(find.text('Account community'), findsOneWidget);
    expect(find.text('Could not load your account communities.'), findsNothing);
    expect(attempts, 2);
  });
}
