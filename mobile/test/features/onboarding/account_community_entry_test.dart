import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:buzz/shared/hosted/browser_auth.dart';
import 'package:buzz/shared/hosted/hosted_flow.dart';
import 'package:buzz/shared/hosted/hosted_identity_service.dart';
import 'package:buzz/features/onboarding/self_hosted_entry_sheet.dart';
import 'package:buzz/hosted_app_gate.dart';
import 'package:buzz/onboarding_app_entry.dart';
import 'package:buzz/shared/community/community_icon_provider.dart';
import 'package:buzz/shared/deeplink/pending_deep_link_provider.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/hosted/hosted_flow_provider.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/hosted_communities_fixture.dart';
import '../../helpers/widget_helpers.dart';

void main() {
  for (final discover in [true, false]) {
    for (final callback in [true, false]) {
      testWidgets(
        'browser restart retains discovery=$discover callback=$callback',
        (tester) async {
          if (!HostedEnvironment.enabled) return;
          FlutterSecureStorage.setMockInitialValues({});
          debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
          tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
            const MethodChannel('buzz/hosted_hpke'),
            (_) async => false,
          );
          addTearDown(() {
            debugDefaultTargetPlatformOverride = null;
            tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
              const MethodChannel('buzz/hosted_hpke'),
              null,
            );
          });
          final fixture = HostedCommunitiesFixture();
          await fixture.initialize(withIdentity: false);
          fixture.rows = [
            {'community_url': 'wss://external.example:8443'},
          ];
          await fixture.account.clearLocalSession();
          await fixture.flow.initialize();
          await fixture.flow.signInAccount(discoverOnly: discover);
          final intent = fixture.browser.startedIntent!;
          fixture.flow.dispose();
          await fixture.browser.dispose();
          final browser = DiscoveryBrowser();
          final flow = HostedFlow(
            account: fixture.account,
            browser: browser,
            store: fixture.store,
            identity: HostedIdentityService(
              account: fixture.account,
              store: fixture.store,
            ),
            openWeb: (_) async => true,
            enterCommunity: (_, _, _, _) async {},
          );
          await flow.initialize();
          if (callback) {
            browser.callbacks.add(BrowserAuthCode('return', 'a' * 43, intent));
            await tester.pumpAndSettle();
          } else {
            // Restored intent also survives a retry before the browser returns.
            await flow.retry();
            browser.callbacks.add(
              BrowserAuthCode('return', 'a' * 43, browser.startedIntent!),
            );
            await tester.pumpAndSettle();
          }
          if (discover) {
            expect(flow.stage, HostedFlowStage.chooseCommunities);
            expect(flow.communities.map((c) => c.relayUrl), [
              'wss://external.example:8443',
            ]);
            expect(
              fixture.requests.where((p) => p.contains('nostr-identities')),
              isEmpty,
            );
          } else {
            expect(flow.stage, HostedFlowStage.blocked);
            expect(flow.problem, HostedFlowProblem.unsupportedPlatform);
            expect(
              fixture.requests.where((p) => p.endsWith('/current')),
              isNotEmpty,
            );
          }
          debugDefaultTargetPlatformOverride = null;
          flow.dispose();
          await browser.dispose();
          fixture.account.dispose();
        },
      );
    }
  }

  for (final scenario in ['external-only', 'mixed', 'hosted-selection']) {
    final mixed = scenario != 'external-only';
    testWidgets(
      'fresh $scenario account discovers destinations before admission',
      (tester) async {
        if (!HostedEnvironment.enabled) return;
        FlutterSecureStorage.setMockInitialValues({});
        PendingDeepLinkNotifier.debugUriStreamOverride =
            const Stream<Uri>.empty();
        addTearDown(
          () => PendingDeepLinkNotifier.debugUriStreamOverride = null,
        );
        final fixture = HostedCommunitiesFixture();
        await fixture.initialize(widget: true, withIdentity: mixed);
        addTearDown(fixture.dispose);
        fixture.rows = [
          {'community_url': 'wss://team.example:8443'},
          {'community_url': 'wss://team.example:9443'},
          {'community_url': 'wss://unsupported.communities.buzz.xyz:8443'},
          if (mixed) fixture.row('hosted'),
        ];
        await tester.pumpWidget(
          WidgetHelpers.testable(
            overrides: [
              hostedFlowProvider.overrideWithValue(fixture.flow),
              builderlabAccountProvider.overrideWithValue(fixture.account),
              communityIconPresentationProvider.overrideWith(
                (ref, relay) => null,
              ),
            ],
            child: const HostedAppGate(
              authenticated: false,
              child: OnboardingAppEntry(),
            ),
          ),
        );
        await tester.pumpAndSettle();
        expect(find.text('Find my communities'), findsOneWidget);
        await tester.tap(find.text('Find my communities'));
        await tester.pumpAndSettle();
        expect(find.text('team.example:8443'), findsOneWidget);
        expect(find.text('team.example:9443'), findsOneWidget);
        expect(
          find.text('unsupported.communities.buzz.xyz:8443'),
          findsNothing,
        );
        expect(find.text('hosted'), mixed ? findsOneWidget : findsNothing);
        expect(
          fixture.requests.where((p) => p.contains('nostr-identities')),
          isEmpty,
        );
        if (scenario == 'hosted-selection') {
          await tester.tap(find.text('hosted'));
          await tester.pumpAndSettle();
          expect(fixture.admitted, ['hosted.communities.buzz.xyz']);
          expect(
            fixture.requests.where((p) => p.endsWith('/current')),
            isNotEmpty,
          );
          expect(find.byType(SelfHostedEntrySheet), findsNothing);
          await tester.pumpWidget(const SizedBox.shrink());
          await tester.pumpAndSettle();
          return;
        }
        final port = mixed ? 9443 : 8443;
        final row = find.byKey(
          Key('community-switcher-row-wss://team.example:$port'),
        );
        final semantics = tester.ensureSemantics();
        expect(tester.getSemantics(row).label, contains('team.example:$port'));
        semantics.dispose();
        await tester.tap(find.text('team.example:$port'));
        await tester.pumpAndSettle();
        expect(find.byType(SelfHostedEntrySheet), findsOneWidget);
        expect(
          tester
              .widget<SelfHostedEntrySheet>(find.byType(SelfHostedEntrySheet))
              .relay,
          'wss://team.example:$port',
        );
        expect(fixture.admitted, isEmpty);
        if (!mixed) {
          expect(
            fixture.requests.where((p) => p.contains('nostr-identities')),
            isEmpty,
            reason:
                'external discovery must not resolve, bind, or back up a hosted key',
          );
          expect(await fixture.store.read(), isNull);
        }
        expect(tester.takeException(), isNull);
        await tester.pumpWidget(const SizedBox.shrink());
        await tester.pumpAndSettle();
      },
    );
  }
}
