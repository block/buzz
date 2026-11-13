import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/hosted/hosted_flow.dart';
import 'package:buzz/shared/utils/string_utils.dart';
import 'package:buzz/shared/deeplink/pending_deep_link_provider.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;

import '../../helpers/hosted_journey_fixture.dart';

void main() {
  void journey(
    String name,
    Future<void> Function(WidgetTester, HostedJourneyFixture) run,
  ) {
    testWidgets(name, (tester) async {
      final f = HostedJourneyFixture();
      await tester.runAsync(
        () => http.runWithClient(() async {
          try {
            await run(tester, f);
          } finally {
            await f.dispose(tester);
          }
        }, f.client),
      );
    }, skip: HostedEnvironment.authDomains.isEmpty);
  }

  // This separate journey covers the first cloud export of an existing local
  // key. The general hosted journey file is already at its size limit.
  testWidgets('self-hosted key requires informed consent before cloud backup', (
    tester,
  ) async {
    final f = HostedJourneyFixture();
    await tester.runAsync(
      () => http.runWithClient(() async {
        try {
          await f.initialize(localIdentity: false);
          f.bound = null;
          f.offerProfile = false;
          final community = Community.create(
            name: 'Self hosted',
            relayUrl: 'wss://self-hosted.example',
            pubkey: f.keys.public,
            nsec: f.keys.nsec,
          );
          await f.storage.save(community);
          await f.storage.save(
            Community.create(
              name: 'Second self hosted',
              relayUrl: 'wss://second.example',
              pubkey: f.secondKeys.public,
              nsec: f.secondKeys.nsec,
            ),
          );
          await f.storage.saveActiveId(community.id);
          await f.mount(tester);
          expect(f.bound, isNull);
          expect(f.sent('/verify'), isEmpty);
          expect(f.sent('/enrollment'), isEmpty);
          expect(f.sent('/nsec-backups'), isEmpty);

          f.accountEmail = 'earlier-account@example.com';
          await f.flow.signInAccount();
          await f.settle(tester);
          await f.tap(tester, fullNpub(f.keys.public)!);
          expect(find.text('Back up your secret key?'), findsOneWidget);
          expect(
            find.textContaining('encrypted copy of your secret key'),
            findsOneWidget,
          );
          expect(
            find.textContaining(HostedEnvironment.origin.host),
            findsOneWidget,
          );
          expect(
            find.textContaining('Anyone signed in to this account'),
            findsOneWidget,
          );
          expect(find.text(fullNpub(f.keys.public)!), findsOneWidget);
          expect(
            find.text('Receiving account: earlier-account@example.com'),
            findsOneWidget,
          );
          expect(
            find.textContaining('cannot change the linked identity later'),
            findsOneWidget,
          );
          expect(await f.store.read(), isNull);
          expect(f.bound, isNull);
          expect(f.sent('/verify'), isEmpty);
          expect(f.sent('/enrollment'), isEmpty);
          expect(f.sent('/nsec-backups'), isEmpty);
          final staleConfirm = tester
              .widget<FilledButton>(
                find.widgetWithText(FilledButton, 'Back up and continue'),
              )
              .onPressed!;
          await f.tap(tester, 'Cancel setup');
          expect((await f.storage.loadAll()).first.nsec, f.keys.nsec);
          expect(await f.storage.loadActiveId(), community.id);
          expect(await f.store.read(), isNull);

          // Re-enter on the same owner: disposal must not be the only fence.
          await f.flow.signInAccount();
          await f.settle(tester);
          expect(f.flow.stage, HostedFlowStage.chooseIdentity);
          await f.tap(tester, fullNpub(f.secondKeys.public)!);
          staleConfirm();
          await f.settle(tester);
          expect(f.flow.stage, HostedFlowStage.confirmBackup);
          expect(f.flow.backupPubkey, f.secondKeys.public);
          expect(await f.store.read(), isNull);
          expect(f.bound, isNull);
          expect(f.sent('/verify'), isEmpty);
          expect(f.sent('/enrollment'), isEmpty);
          expect(f.sent('/nsec-backups'), isEmpty);
          await f.restart(tester);
          expect(f.bound, isNull);
          expect(f.sent('/verify'), isEmpty);
          expect(f.sent('/enrollment'), isEmpty);
          await f.flow.signInAccount();
          await f.settle(tester);
          expect(f.flow.stage, HostedFlowStage.chooseIdentity);
          await f.tap(tester, fullNpub(f.keys.public)!);
          expect(find.text('Back up your secret key?'), findsOneWidget);
          staleConfirm();
          await f.settle(tester);
          expect(f.bound, isNull);
          expect(f.sent('/verify'), isEmpty);
          expect(f.sent('/enrollment'), isEmpty);
          expect(f.sent('/nsec-backups'), isEmpty);
          // Android/system Back also discards the tentative selection.
          await tester.binding.handlePopRoute();
          await f.settle(tester);
          expect(f.flow.stage, HostedFlowStage.idle);
          expect((await f.storage.loadAll()).first.nsec, f.keys.nsec);
          await f.flow.signInAccount();
          await f.settle(tester);
          await f.tap(tester, fullNpub(f.secondKeys.public)!);
          // A restored unintended account is recognizable and can be signed out.
          await f.tap(tester, 'Sign out');
          expect(f.flow.account.account, isNull);
          expect(await f.store.read(), isNull);
          f.accountEmail = null;
          await f.flow.signInAccount();
          await f.settle(tester);
          await f.tap(tester, fullNpub(f.secondKeys.public)!);
          expect(f.flow.confirmBackup, isNull);
          expect(
            find.textContaining('couldn’t identify the receiving account'),
            findsOneWidget,
          );
          await f.tap(tester, 'Cancel setup');
          f.accountEmail = 'owner@example.com';
          await f.flow.signInAccount();
          await f.settle(tester);
          await f.tap(tester, fullNpub(f.secondKeys.public)!);
          expect(
            find.text('Receiving account: owner@example.com'),
            findsOneWidget,
          );

          // The server accepted the upload, but the acknowledgement was lost.
          f.beforeUploadReturn = () async {
            f.cloudKeys.add(f.secondKeys);
            throw http.ClientException('Synthetic lost upload response');
          };
          await f.tap(tester, 'Back up and continue');
          expect(f.bound, f.secondKeys.public);
          expect(f.sent('/enrollment'), hasLength(1));
          expect(f.sent('/nsec-backups'), hasLength(1));
          expect((await f.store.read())!.backup, isNull);
          await f.tap(tester, 'Try again');
          expect(find.textContaining('does not undo'), findsOneWidget);
          expect(find.textContaining('confirmation was lost'), findsOneWidget);
          await f.tap(tester, 'Cancel setup');
          expect(f.bound, f.secondKeys.public);
          expect(f.cloudKeys.single.public, f.secondKeys.public);
          expect(f.sent('/nsec-backups'), hasLength(1));
          await f.restart(tester);
          await f.flow.signInAccount();
          await f.settle(tester);
          expect(find.text('Back up your secret key?'), findsOneWidget);
          expect(find.textContaining('does not undo'), findsOneWidget);
          f.beforeUploadReturn = null;
          await f.tap(tester, 'Back up and continue');
          expect((await f.store.read())!.backup!.pubkey, f.secondKeys.public);
          expect(
            (await f.storage.loadAll()).map((c) => c.nsec),
            containsAll([f.keys.nsec, f.secondKeys.nsec]),
          );
          await f.restart(tester);
          await f.flow.signInAccount();
          await f.settle(tester);
          expect(find.text('Back up your secret key?'), findsNothing);
          expect(f.sent('/nsec-backups'), hasLength(2));
        } finally {
          await f.dispose(tester);
        }
      }, f.client),
    );
  }, skip: HostedEnvironment.authDomains.isEmpty);
  journey(
    'web binding keeps an explicit identity choice without backup support',
    (tester, f) async {
      await f.initialize(localIdentity: false);
      f.bound = null;
      await f.mount(tester);
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        const MethodChannel('buzz/hosted_hpke'),
        (_) async => false,
      );
      final nativeChannels = <MethodChannel>[];
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform_views,
        (call) async {
          if (call.method == 'create') {
            final id = (call.arguments as Map)['id'];
            final channel = MethodChannel('buzz/ios_navigation_bar/$id');
            nativeChannels.add(channel);
            tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
              channel,
              (_) async => null,
            );
          }
          return null;
        },
      );
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        const MethodChannel('plugins.flutter.io/url_launcher'),
        (_) async => true,
      );
      void openBindLink() {
        f.container
            .read(pendingDeepLinkProvider.notifier)
            .open(
              Uri(
                scheme: 'buzz',
                host: 'nostr-bind',
                queryParameters: {
                  'challenge_id': '12345678-1234-1234-1234-123456789012',
                  'nonce': 'a' * 43,
                  'verification_code': '123456',
                  'audience': 'buzz:nostr-identity',
                  'action': 'bind_nostr_identity',
                  'protocol': 'buzz-nostr-identity',
                  'version': '1',
                  'origin': HostedEnvironment.origin.toString(),
                  'expires_at': '2099-01-01T00:00:00Z',
                  'return': 'browser_fragment_v1',
                  'callback_url': HostedEnvironment.origin
                      .resolve('/buzz')
                      .toString(),
                },
              ),
            );
      }

      try {
        openBindLink();
        await f.settle(tester);
        expect(
          find.text('Create my Buzz identity').hitTestable(),
          findsOneWidget,
        );
        expect(
          find.text('Import existing identity').hitTestable(),
          findsOneWidget,
        );
        expect(await f.store.read(), isNull);
        expect(f.bound, isNull);
        expect(f.sent('/verify'), isEmpty);
        await f.tap(tester, 'Import existing identity');
        await tester.enterText(
          find.byType(TextField).hitTestable(),
          f.secondKeys.nsec,
        );
        await f.tap(tester, 'Preview identity');
        await f.tap(tester, 'Import this identity');
        expect(find.text('Link your identity?'), findsOneWidget);
        expect(find.textContaining('No backup is uploaded'), findsOneWidget);
        expect(
          find.text('Receiving account: owner@example.com'),
          findsOneWidget,
        );
        expect(await f.store.read(), isNull);
        expect(f.sent('/verify'), isEmpty);
        expect(f.sent('/enrollment'), isEmpty);
        expect(f.sent('/nsec-backups'), isEmpty);
        await f.tap(tester, 'Cancel setup');
        expect(await f.store.read(), isNull);
        expect(f.sent('/verify'), isEmpty);
        debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
        openBindLink();
        await f.settle(tester);
        await f.tap(tester, 'Create my Buzz identity');
        expect(f.flow.problem, HostedFlowProblem.unsupportedPlatform);
        expect(await f.store.read(), isNull);
        expect(f.bound, isNull);
        // Once the OS supports sealing, creation still requires an explicit tap.
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          const MethodChannel('buzz/hosted_hpke'),
          (_) async => true,
        );
        await f.flow.retry();
        await f.settle(tester);
        expect(await f.store.read(), isNull);
        await f.tap(tester, 'Create my Buzz identity');
        final saved = (await f.store.read())!;
        expect(f.bound, saved.pubkey);
        expect(
          saved.backup,
          isNull,
        ); // Web binding has never promised mobile backup.
        expect(f.flow.stage, HostedFlowStage.browserReturned);

        // A returning account may already have a saved key from an interrupted
        // setup. No adoption occurs, so the bind-only gate must ask for consent.
        await f.flow.signOutAccount();
        f.subject = 'returning-owner';
        f.bound = null;
        await f.store.create(f.secondKeys.nsec);
        await f.flow.account.exchange(
          code: 'synthetic-code',
          verifier: 'v' * 43,
        );
        debugDefaultTargetPlatformOverride = null;
        await f.restart(tester);
        f.requests.clear();
        openBindLink();
        await f.settle(tester);
        expect(find.text('Link your identity?'), findsOneWidget);
        expect(find.textContaining('No backup is uploaded'), findsOneWidget);
        expect(find.text(fullNpub(f.secondKeys.public)!), findsOneWidget);
        expect((await f.store.read())!.backup, isNull);
        expect(f.bound, isNull);
        expect(f.sent('/verify'), isEmpty);
        await f.tap(tester, 'Cancel setup');
        expect((await f.store.read())!.pubkey, f.secondKeys.public);
        expect(f.bound, isNull);
        expect(f.sent('/verify'), isEmpty);

        // Cancellation and restart retain the key, but never retain approval.
        await f.restart(tester);
        openBindLink();
        await f.settle(tester);
        expect(find.text('Link your identity?'), findsOneWidget);
        expect(f.sent('/verify'), isEmpty);
        await f.tap(tester, 'Link and continue');
        expect(f.bound, f.secondKeys.public);
        expect(f.sent('/verify'), hasLength(1));
        expect(f.sent('/enrollment'), isEmpty);
        expect(f.sent('/nsec-backups'), isEmpty);
        expect((await f.store.read())!.backup, isNull);
        expect(f.flow.stage, HostedFlowStage.browserReturned);
      } finally {
        await tester.pumpWidget(const SizedBox.shrink());
        await tester.pump();
        debugDefaultTargetPlatformOverride = null;
        for (final channel in nativeChannels) {
          tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
            channel,
            null,
          );
        }
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          SystemChannels.platform_views,
          null,
        );
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          const MethodChannel('plugins.flutter.io/url_launcher'),
          null,
        );
      }
    },
  );
}
