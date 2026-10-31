import '../../helpers/two_finger_double_tap.dart';
import 'package:buzz/shared/community/community_grid_option.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/onboarding/hosted_connect_page.dart';
import 'package:buzz/features/onboarding/self_hosted_entry_sheet.dart';
import 'package:buzz/hosted_app_gate.dart';
import 'package:buzz/shared/hosted/hosted_flow_provider.dart';
import 'package:buzz/onboarding_app_entry.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/identity_transfer/nsec_import_sheet.dart';
import 'package:buzz/shared/deeplink/pending_deep_link_provider.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_flow.dart';
import 'package:buzz/shared/hosted/hosted_community_connection.dart';
import 'package:buzz/shared/hosted/self_hosted_entry.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/widgets/buzz_sheet_header.dart';
import 'package:flutter/material.dart';
import 'package:buzz/shared/widgets/bee_loading_mark.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../helpers/hosted_communities_fixture.dart';
import '../../helpers/widget_helpers.dart';

class _NoChannels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [];
}

void main() {
  testWidgets('pushed Add route owns paired progress and self-hosted completion', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(440, 956));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    FlutterSecureStorage.setMockInitialValues({});
    PendingDeepLinkNotifier.debugUriStreamOverride = const Stream<Uri>.empty();
    final f = HostedCommunitiesFixture();
    await f.initialize(widget: true);
    addTearDown(() async {
      PendingDeepLinkNotifier.debugUriStreamOverride = null;
      await f.dispose();
    });
    var probes = 0;
    final inventoryStorage = _EnumerationStorage();
    Completer<void>? pendingProbe;
    await tester.pumpWidget(
      WidgetHelpers.testable(
        disableAnimations: true,
        overrides: [
          hostedFlowProvider.overrideWithValue(f.flow),
          channelsProvider.overrideWith(_NoChannels.new),
          selfHostedEntryProvider.overrideWithValue(
            SelfHostedEntry(
              accountStore: AccountIdentityStore(storage: inventoryStorage),
              probe: (_) async {
                probes++;
                await pendingProbe?.future;
              },
            ),
          ),
        ],
        child: HostedAppGate(
          authenticated: true,
          child: Builder(
            builder: (context) => Center(
              child: FilledButton(
                onPressed: () => Navigator.of(context).push(
                  MaterialPageRoute<void>(
                    builder: (_) =>
                        const OnboardingAppEntry(addingCommunity: true),
                  ),
                ),
                child: const Text('Add community'),
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final container = ProviderScope.containerOf(
      tester.element(find.byType(HostedAppGate)),
    );
    await container.read(authProvider.future);

    Future<void> openEntry() async {
      await tester.tap(find.text('Add community'));
      await tester.pumpAndSettle();
      expect(find.byType(OnboardingAppEntry), findsOneWidget);
    }

    Future<void> pair() async {
      final trigger = find.byKey(const Key('developer-pairing-trigger'));
      await twoFingerDoubleTap(tester, tester.getCenter(trigger));
      await tester.tap(find.text('Use a pairing code').hitTestable());
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byType(TextField).hitTestable(),
        base64Url.encode(
          utf8.encode(
            jsonEncode({
              'relayUrl': 'https://paired.communities.buzz.xyz',
              'nsec': f.keys.nsec,
              'pubkey': f.keys.public,
            }),
          ),
        ),
      );
      await tester.tap(find.text('Connect').hitTestable());
      await tester.pump();
      // Finish the standard page push before checking the loading surface.
      await tester.pump();
      await tester.pump(const Duration(seconds: 1));
    }

    if (HostedEnvironment.authDomains.isNotEmpty) {
      // A root-navigator choice modal must remain actionable above pushed Add.
      f.rows = [f.row('one'), f.row('two')];
      f.beforeAdmission = (name) async {
        if (name == 'two.communities.buzz.xyz') {
          throw const SavedCommunityIdentityConflict();
        }
      };
      await openEntry();
      expect(find.text('Create a community'), findsNothing);
      expect(find.text('Continue with Builderlab'), findsNothing);
      expect(find.text('Use a pairing code'), findsNothing);
      // An independently resumed account restore still owns a root modal
      // above Add; Add itself no longer offers account sign-in or creation.
      await f.flow.signInAccount();
      await tester.pumpAndSettle();
      expect(f.admitted, isEmpty);
      await tester.tap(find.byType(CommunityGridOption).first);
      await tester.pumpAndSettle();
      expect(f.admitted, ['one.communities.buzz.xyz']);
      expect(f.flow.stage, HostedFlowStage.communityConflict);
      expect(
        find.text('Keep saved identity and skip').hitTestable(),
        findsOneWidget,
      );
      expect(find.text('Back to choices').hitTestable(), findsOneWidget);
      await tester.tap(find.text('Back to choices').hitTestable());
      await tester.pumpAndSettle();
      expect(find.byType(CommunityGridOption).hitTestable(), findsNWidgets(2));
      expect(
        tester
            .widget<CommunityGridOption>(
              find.byType(CommunityGridOption).hitTestable().last,
            )
            .name,
        'two',
      );
      f.beforeAdmission = null;
      await tester.tap(find.byType(CommunityGridOption).first);
      await tester.pumpAndSettle();
      expect(f.admitted, [
        'one.communities.buzz.xyz',
        'two.communities.buzz.xyz',
      ]);
      expect(find.byType(OnboardingAppEntry), findsNothing);
      expect(find.text('Add community').hitTestable(), findsOneWidget);
      f.admitted.clear();
      f.rows = [];
      await f.flow.leave();
    }
    if (HostedEnvironment.requiresAuth('paired.communities.buzz.xyz')) {
      await openEntry();
      final admission = Completer<void>();
      f.beforeAdmission = (_) => admission.future;
      await pair();
      expect(f.flow.accountOnly, isFalse);
      expect(f.flow.stage, HostedFlowStage.working);
      expect(find.byTooltip('Back').hitTestable(), findsNothing);
      expect(find.byType(BeeLoadingMark).hitTestable(), findsOneWidget);
      expect(find.byType(BuzzSheetHeader), findsNothing);
      expect(
        find.byType(HostedConnectPage, skipOffstage: false),
        findsOneWidget,
      );
      admission.completeError(AccountFailure.unavailable);
      await tester.pumpAndSettle();
      expect(f.flow.stage, HostedFlowStage.blocked);
      expect(
        find
            .text('We could not finish connecting. You can try again.')
            .hitTestable(),
        findsOneWidget,
      );
      expect(find.text('Try again').hitTestable(), findsOneWidget);
      expect(find.byTooltip('Back').hitTestable(), findsOneWidget);
      expect(find.byType(BuzzSheetHeader), findsNothing);
      f.beforeAdmission = null;
      await tester.tap(find.text('Try again').hitTestable());
      await tester.pumpAndSettle();
      expect(f.admitted, ['paired']);
      expect(f.remembered, contains('wss://paired.communities.buzz.xyz'));
      expect(find.byType(OnboardingAppEntry), findsNothing);
      expect(find.text('Add community').hitTestable(), findsOneWidget);

      // Cancelling a paired sign-in keeps the Add route usable and clears intent.
      await f.flow.leave();
      await openEntry();
      f.expired = true;
      await pair();
      await tester.pumpAndSettle();
      expect(f.flow.stage, HostedFlowStage.signIn);
      // Native browser owns sign-in presentation; the welcome/form stays behind it.
      expect(find.byType(HostedConnectPage), findsNothing);
      await f.flow.leave();
      await tester.pumpAndSettle();
      expect(f.flow.stage, HostedFlowStage.idle);
      await tester.tap(find.byTooltip('Back').hitTestable());
      await tester.pumpAndSettle();
      expect(find.text('Enter a community link').hitTestable(), findsOneWidget);
      expect(container.read(hostedInlineEntryProvider), isTrue);
      f.expired = false;
    } else {
      await openEntry();
    }

    // The selected button becomes Continue without replacing its element.
    final linkAction = find.byKey(const Key('add-community-link-action'));
    final originalAction = tester.element(linkAction);
    await tester.tap(find.text('Enter a community link').hitTestable());
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
    expect(tester.element(linkAction), same(originalAction));
    await tester.pumpAndSettle();
    expect(
      tester
          .widget<Transform>(find.byKey(const Key('add-community-link-rise')))
          .transform
          .storage[13],
      0,
    );
    await tester.tap(find.text('Continue').hitTestable());
    await tester.pumpAndSettle();
    expect(
      find.text('Enter a valid community address or invite link.'),
      findsOneWidget,
    );
    // Drive the real deep-link dispatcher, sheet, service and auth persistence.
    await tester.enterText(
      find.byKey(const Key('community-link-input')).hitTestable(),
      'new-community.example',
    );
    await tester.tap(find.text('Continue').hitTestable());
    await tester.pumpAndSettle();
    expect(find.byType(SelfHostedEntrySheet), findsOneWidget);
    expect(find.text('Saved identity').hitTestable(), findsOneWidget);
    expect(find.text('Import a Nostr key').hitTestable(), findsOneWidget);
    Navigator.of(tester.element(find.byType(SelfHostedEntrySheet))).pop();
    await tester.pumpAndSettle();
    // A failed bulk read must keep explicit import available, but not offer
    // generation on the strength of a falsely empty inventory.
    inventoryStorage.failEnumeration = true;
    await tester.tap(find.text('Continue').hitTestable());
    await tester.pumpAndSettle();
    expect(find.text('Import a Nostr key').hitTestable(), findsOneWidget);
    expect(find.text('Create an identity and join'), findsNothing);
    Navigator.of(tester.element(find.byType(SelfHostedEntrySheet))).pop();
    await tester.pumpAndSettle();
    await const FlutterSecureStorage().write(
      key: 'buzz_nsec',
      value: f.keys.nsec,
    );
    await tester.tap(find.text('Continue').hitTestable());
    await tester.pumpAndSettle();
    expect(find.text('Import a Nostr key').hitTestable(), findsOneWidget);
    final joinRoute = ModalRoute.of(
      tester.element(find.byType(SelfHostedEntrySheet)),
    );
    final joinHeight = tester.getSize(find.byType(SelfHostedEntrySheet)).height;
    await tester.tap(find.text('Import a Nostr key').hitTestable());
    await tester.pumpAndSettle();
    expect(find.byType(BuzzSheetHeader), findsNothing);
    expect(find.text('Import identity'), findsOneWidget);
    expect(
      ModalRoute.of(tester.element(find.byType(NsecImportSheet))),
      isNot(same(joinRoute)),
    );
    expect(find.byType(SelfHostedEntrySheet), findsNothing);
    await tester.tap(find.byTooltip('Back'));
    await tester.pumpAndSettle();
    expect(find.byType(SelfHostedEntrySheet), findsOneWidget);
    expect(find.byType(NsecImportSheet), findsNothing);
    expect(find.text('You already have a saved identity'), findsOneWidget);
    expect(
      tester.getSize(find.byType(SelfHostedEntrySheet)).height,
      joinHeight,
    );
    await tester.tap(find.text('Import a Nostr key').hitTestable());
    await tester.pumpAndSettle();
    await tester.enterText(
      find.descendant(
        of: find.byType(NsecImportSheet),
        matching: find.byType(TextField),
      ),
      f.keys.nsec,
    );
    await tester.tap(find.text('Preview identity'));
    await tester.pumpAndSettle();
    expect(probes, 0);
    pendingProbe = Completer<void>();
    await tester.tap(find.text('Import this identity'));
    await tester.pumpAndSettle();
    expect(probes, 1);
    await tester.tap(find.byTooltip('Back'));
    // Completion races the outgoing route animation.
    pendingProbe.complete();
    await tester.pumpAndSettle();
    expect(find.text('You already have a saved identity'), findsOneWidget);
    expect(await container.read(communityStorageProvider).loadAll(), isEmpty);
    pendingProbe = null;
    await tester.tap(find.text('Import a Nostr key').hitTestable());
    await tester.pumpAndSettle();
    await tester.enterText(
      find.descendant(
        of: find.byType(NsecImportSheet),
        matching: find.byType(TextField),
      ),
      f.keys.nsec,
    );
    await tester.tap(find.text('Preview identity'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Import this identity'));
    await tester.pumpAndSettle();
    expect(probes, 2);
    expect(find.byType(SelfHostedEntrySheet), findsNothing);
    expect(find.byType(OnboardingAppEntry), findsNothing);
    expect(find.text('Add community').hitTestable(), findsOneWidget);
    final saved = await container.read(communityStorageProvider).loadAll();
    expect(saved.single.relayUrl, 'wss://new-community.example');
    expect(saved.single.pubkey, f.keys.public);
    expect(
      await container.read(communityStorageProvider).loadActiveId(),
      saved.single.id,
    );

    // Reopening an already saved URL switches to it and also closes Add.
    await openEntry();
    await tester.tap(find.text('Enter a community link').hitTestable());
    await tester.pumpAndSettle();
    await tester.enterText(
      find.byKey(const Key('community-link-input')).hitTestable(),
      'new-community.example',
    );
    await tester.tap(find.text('Continue').hitTestable());
    await tester.pumpAndSettle();
    expect(find.byType(OnboardingAppEntry), findsNothing);
    expect(find.text('Add community').hitTestable(), findsOneWidget);
    expect(probes, 2);
    expect(
      await container.read(communityStorageProvider).loadActiveId(),
      saved.single.id,
    );

    // Popping Add releases ownership; a later invite uses the app-level UI.
    await openEntry();
    await tester.tap(find.text('Enter a community link').hitTestable());
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Back').hitTestable());
    await tester.pumpAndSettle();
    expect(container.read(hostedInlineEntryProvider), isTrue);
    await tester.pageBack();
    await tester.pumpAndSettle();
    expect(find.byType(OnboardingAppEntry), findsNothing);
    expect(container.read(hostedInlineEntryProvider), isFalse);
    if (HostedEnvironment.requiresAuth('invited.communities.buzz.xyz')) {
      f.expired = true;
      container
          .read(pendingDeepLinkProvider.notifier)
          .open(
            Uri(
              scheme: 'buzz',
              host: 'join',
              queryParameters: {
                'relay': 'wss://invited.communities.buzz.xyz',
                'code': 'invite',
              },
            ),
          );
      await tester.pumpAndSettle();
      expect(f.flow.stage, HostedFlowStage.signIn);
      expect(find.text('Start login again').hitTestable(), findsOneWidget);
      expect(find.text('Leave setup').hitTestable(), findsOneWidget);
      await tester.tap(find.text('Leave setup').hitTestable());
      await tester.pumpAndSettle();
      f.expired = false;
    }

    // Disposing the entire scope while an entry owns presentation is safe.
    await openEntry();
    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pump();
    expect(tester.takeException(), isNull);

    if (HostedEnvironment.authDomains.isEmpty) return;

    // A new coordinator restores the paired target and presents its waiting
    // state on the entry route, without depending on accountOnly discovery.
    await f.flow.stagePairedCommunity(
      Community.create(
        name: 'Restored',
        relayUrl: 'wss://restored.communities.buzz.xyz',
        pubkey: f.keys.public,
        nsec: f.keys.nsec,
      ),
      isCurrent: () => true,
    );
    final restartedBrowser = DiscoveryBrowser();
    final restarted = HostedFlow(
      account: f.account,
      browser: restartedBrowser,
      store: f.store,
      identity: f.flow.identity,
      enterCommunity: (link, subject, pubkey, current) async {},
    );
    addTearDown(() async {
      restarted.dispose();
      await restartedBrowser.dispose();
    });
    await tester.pumpWidget(
      WidgetHelpers.testable(
        disableAnimations: true,
        overrides: [hostedFlowProvider.overrideWithValue(restarted)],
        child: const HostedAppGate(
          authenticated: false,
          child: OnboardingAppEntry(),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(restarted.stage, HostedFlowStage.signIn);
    expect(find.text('Cancel sign-in').hitTestable(), findsOneWidget);
    expect(find.byType(BuzzSheetHeader), findsNothing);
    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pump();
    expect(tester.takeException(), isNull);
  });
}

class _EnumerationStorage extends FlutterSecureStorage {
  bool failEnumeration = false;
  @override
  Future<Map<String, String>> readAll({
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async {
    if (failEnumeration) throw StateError('unrelated entry cannot decrypt');
    return super.readAll();
  }
}
