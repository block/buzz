import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/hosted/hosted_flow.dart';
import 'package:buzz/shared/utils/string_utils.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;

import '../../helpers/hosted_journey_fixture.dart';

void main() {
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
          await f.storage.saveActiveId(community.id);
          await f.mount(tester);
          expect(f.bound, isNull);
          expect(f.sent('/verify'), isEmpty);
          expect(f.sent('/enrollment'), isEmpty);
          expect(f.sent('/nsec-backups'), isEmpty);

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
            find.textContaining('Anyone signed in to your Builderlab account'),
            findsOneWidget,
          );
          expect(find.text(fullNpub(f.keys.public)!), findsOneWidget);
          expect(f.bound, isNull);
          expect(f.sent('/verify'), isEmpty);
          expect(f.sent('/enrollment'), isEmpty);
          expect(f.sent('/nsec-backups'), isEmpty);
          final staleConfirm = tester
              .widget<FilledButton>(
                find.widgetWithText(FilledButton, 'Back up and continue'),
              )
              .onPressed!;
          await f.tap(tester, 'Keep key on this device');
          expect((await f.storage.loadAll()).single.nsec, f.keys.nsec);
          expect(await f.storage.loadActiveId(), community.id);
          expect((await f.store.read())!.backup, isNull);

          // Re-enter on the same owner: disposal must not be the only fence.
          await f.flow.signInAccount();
          await f.settle(tester);
          staleConfirm();
          await f.settle(tester);
          expect(f.flow.stage, HostedFlowStage.confirmBackup);
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
          expect(find.text('Back up your secret key?'), findsOneWidget);
          staleConfirm();
          await f.settle(tester);
          expect(f.bound, isNull);
          expect(f.sent('/verify'), isEmpty);
          expect(f.sent('/enrollment'), isEmpty);
          expect(f.sent('/nsec-backups'), isEmpty);
          // Android/system Back cancels just as the explicit local-only action does.
          await tester.binding.handlePopRoute();
          await f.settle(tester);
          expect(f.flow.stage, HostedFlowStage.idle);
          expect((await f.storage.loadAll()).single.nsec, f.keys.nsec);
          await f.flow.signInAccount();
          await f.settle(tester);
          await f.tap(tester, 'Back up and continue');
          expect(f.sent('/enrollment'), hasLength(1));
          expect(f.sent('/nsec-backups'), hasLength(1));
          expect((await f.store.read())!.backup!.pubkey, f.keys.public);
          expect((await f.storage.loadAll()).single.nsec, f.keys.nsec);
          await f.restart(tester);
          await f.flow.signInAccount();
          await f.settle(tester);
          expect(find.text('Back up your secret key?'), findsNothing);
          expect(f.sent('/nsec-backups'), hasLength(1));
        } finally {
          await f.dispose(tester);
        }
      }, f.client),
    );
  }, skip: HostedEnvironment.authDomains.isEmpty);
}
