import 'package:buzz/shared/deeplink/pending_deep_link_provider.dart';
import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/onboarding/hosted_profile_setup.dart';
import 'package:buzz/onboarding_app_entry.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/utils/string_utils.dart';
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
      final fixture = HostedJourneyFixture();
      // Keep asynchronous storage/HTTP owners in one real zone across tests;
      // their serialized tails otherwise outlive the widget fake clock. The
      // fixture pumps bounded frames with event-loop yields, not pumpAndSettle.
      await tester.runAsync(
        () => http.runWithClient(() async {
          try {
            await run(tester, fixture);
          } finally {
            await fixture.dispose(tester);
          }
        }, fixture.client),
      );
    });
  }

  journey(
    'fresh account signs in, backs up, accepts invite and retries starters before home',
    (tester, f) async {
      await f.initialize(
        localIdentity: false,
        cloudChoice: true,
        signedIn: false,
      );
      f.starterFailures = 2;
      final browserReturn = Completer<void>();
      f.beforeBrowserReturn = () => browserReturn.future;
      final enrollmentReached = Completer<void>();
      final enrollmentReturn = Completer<void>();
      f.beforeEnrollmentReturn = () {
        enrollmentReached.complete();
        return enrollmentReturn.future;
      };
      await f.mount(tester);
      await f.link(tester, 'one');
      try {
        expect(
          find.text('Enter a community link').hitTestable(),
          findsOneWidget,
        );
        expect(find.text('Scan a QR code').hitTestable(), findsOneWidget);
        expect(find.text('Start login again'), findsNothing);
        expect(find.text('Complete sign-in in your browser.'), findsNothing);
      } finally {
        browserReturn.complete();
      }
      await enrollmentReached.future.timeout(const Duration(seconds: 5));
      await tester.pump();
      try {
        await tester.tap(find.text('Leave setup').hitTestable());
        await tester.pump();
      } finally {
        enrollmentReturn.complete();
      }
      await f.settle(tester);
      expect(f.sent('/nsec-backups'), isEmpty);
      final retainedKey = (await f.store.read())!.pubkey;
      f.beforeEnrollmentReturn = null;
      // A dispatched upload can succeed after cancellation. Its receipt must
      // still be durable; resuming must not replay an acknowledged backup.
      f.beforeUploadReturn = () => f.flow.leave();
      await f.link(tester, 'one');
      expect((await f.store.read())!.backup, isNotNull);
      f.beforeUploadReturn = null;
      await f.link(tester, 'one');
      expect((await f.store.read())!.pubkey, retainedKey);
      expect(f.browserStarts, 1);
      final generated = (await f.store.read())!;
      expect(generated.pubkey, f.bound);
      expect(generated.backup, isNotNull);
      expect(f.sent('/verify'), hasLength(1));
      expect(f.sent('/nsec-backups'), hasLength(1));
      final accept = find.widgetWithText(FilledButton, 'Accept and join');
      expect(tester.widget<FilledButton>(accept).onPressed, isNull);
      expect(f.sent('/claim'), isEmpty);
      expect(await f.storage.loadAll(), isEmpty);
      await f.tap(tester, 'Terms of service');
      expect(find.text('Be kind to this community.'), findsOneWidget);
      await tester.tap(find.byType(CheckboxListTile));
      await f.settle(tester);
      await f.tap(tester, 'Accept and join');
      expect(find.text('Try again').hitTestable(), findsOneWidget);
      final pending = (await f.storage.loadAll()).single;
      expect(pending.starterSetupIncomplete, isTrue);
      expect(pending.pubkey, generated.pubkey);
      expect(f.sent('/claim'), hasLength(1));
      expect(
        jsonDecode(f.sent('/claim').single.body)['policy_receipt'],
        'synthetic-policy-receipt',
      );
      expect(
        jsonDecode(f.sent('/accept-policy').single.body)['age_confirmed'],
        isTrue,
      );
      // Reopen the saved invite after its claim was consumed. Starter recovery
      // must retain the durable marker and must not claim it again.
      await f.link(tester, 'one');
      expect((await f.storage.loadAll()).single.starterSetupIncomplete, isTrue);
      expect(find.text('Try again').hitTestable(), findsOneWidget);
      await f.tap(tester, 'Try again');
      expect(find.byType(HostedProfileSetup), findsOneWidget);
      expect(f.sent('/claim'), hasLength(1));
      expect(f.recoveryScopes, hasLength(3));
      expect(
        f.recoveryScopes.every((scope) => scope.nsec == pending.nsec),
        isTrue,
      );
      await tester.enterText(
        find.byType(TextField).hitTestable(),
        'Journey Person',
      );
      await f.tap(tester, 'Continue');
      expect(f.profileWrites, ['Journey Person']);
      final saved = (await f.storage.loadAll()).single;
      expect(saved.id, pending.id);
      expect(saved.starterSetupIncomplete, isFalse);
      expect(await f.storage.loadActiveId(), saved.id);
      expect(find.byType(OnboardingAppEntry), findsNothing);
      expect(find.text('Community home').hitTestable(), findsOneWidget);
    },
  );

  journey(
    'choose second cloud identity, dismiss profile, retry remaining batch without losing keys',
    (tester, f) async {
      await f.initialize(localIdentity: false, cloudChoice: true);
      f.cloudKeys.addAll([f.keys, f.secondKeys]);
      f.rejectList = true;
      f.secondRelayFailures = 1;
      final legacy = Community.create(
        name: 'Saved local',
        relayUrl: 'wss://saved.example',
        pubkey: f.keys.public,
        nsec: f.keys.nsec,
      );
      await f.storage.save(legacy);
      await f.storage.saveActiveId(legacy.id);
      await f.mount(tester);
      await f.tap(tester, 'Continue with Builderlab');
      expect(find.text('Use Builderlab identity'), findsNWidgets(2));
      await f.tap(tester, fullNpub(f.secondKeys.public)!);
      expect((await f.store.read())!.pubkey, f.secondKeys.public);
      expect(
        find.text('We could not finish connecting. You can try again.'),
        findsOneWidget,
      );
      expect(
        find.text(
          'No communities available to add. Scan a QR code or use an invite link.',
        ),
        findsNothing,
      );
      f.rejectList = false;
      await f.tap(tester, 'Continue with Builderlab');
      expect(
        find.text(
          'No communities available to add. Scan a QR code or use an invite link.',
        ),
        findsOneWidget,
      );
      expect(find.text('Scan a QR code'), findsOneWidget);
      expect(find.text('Use pairing code'), findsOneWidget);
      f.rows.addAll([f.row('one'), f.row('two')]);
      await f.tap(tester, 'Continue with Builderlab');
      // Joined communities are added automatically after choosing the identity.
      expect(find.byType(CheckboxListTile).hitTestable(), findsNothing);
      expect(find.byType(HostedProfileSetup), findsOneWidget);
      final firstSaved = (await f.storage.loadAll()).firstWhere(
        (c) => c.name == 'one',
      );
      Navigator.of(
        tester.element(find.byType(HostedProfileSetup)),
        rootNavigator: true,
      ).pop();
      await f.settle(tester);
      expect(f.profileWrites, isEmpty);
      expect(find.text('Try again').hitTestable(), findsOneWidget);
      expect(
        (await f.storage.loadAll()).map((c) => c.id),
        containsAll([legacy.id, firstSaved.id]),
      );
      await f.tap(tester, 'Try again');
      final saved = await f.storage.loadAll();
      expect(saved, hasLength(3));
      expect(saved.singleWhere((c) => c.id == legacy.id).nsec, f.keys.nsec);
      expect(
        saved
            .where((c) => c.id != legacy.id)
            .every((c) => c.nsec == f.secondKeys.nsec),
        isTrue,
      );
      expect(saved.singleWhere((c) => c.name == 'one').id, firstSaved.id);
      expect(await f.storage.loadActiveId(), firstSaved.id);
      expect(f.probes.where((c) => c.name == 'one'), hasLength(1));
      expect(f.probes.where((c) => c.name == 'two'), hasLength(2));
      expect(find.byType(OnboardingAppEntry), findsNothing);
    },
  );

  for (final unreadableProfile in [false, true]) {
    journey(
      'new device recovers and preserves existing/unreadable profile ($unreadableProfile)',
      (tester, f) async {
        await f.initialize(localIdentity: false);
        f.cloudKeys.addAll([f.secondKeys, f.keys]);
        f.rows.add(f.row('one'));
        f.offerProfile = false;
        f.profileUnavailable = unreadableProfile;
        await f.mount(tester);
        await f.tap(tester, 'Continue with Builderlab');
        // One discovered community is admitted automatically.
        expect(f.profileWrites, isEmpty);
        expect(find.byType(HostedProfileSetup), findsNothing);
        final saved = (await f.storage.loadAll()).single;
        expect(saved.nsec, f.keys.nsec);
        expect(saved.pubkey, f.keys.public);
        expect((await f.store.read())!.pubkey, f.keys.public);
        expect(f.sent('/recovery'), hasLength(1));
        expect(f.sent('/challenge'), isEmpty);
        expect(await f.storage.loadActiveId(), saved.id);
        expect(find.byType(OnboardingAppEntry), findsNothing);
      },
    );
  }

  journey(
    'self-hosted link explicitly reuses saved account key and persists admission',
    (tester, f) async {
      await f.initialize();
      await f.mount(tester);
      await f.tap(tester, 'Enter a community link');
      await tester.enterText(
        find.byKey(const Key('community-link-input')).hitTestable(),
        'my-community.example',
      );
      await f.tap(tester, 'Continue');
      await f.tap(tester, 'Saved account identity');
      final saved = (await f.storage.loadAll()).single;
      expect(saved.relayUrl, 'wss://my-community.example');
      expect(saved.nsec, f.keys.nsec);
      expect(await f.storage.loadActiveId(), saved.id);
      expect(f.probes, hasLength(1));
      expect(f.sent('/claim'), isEmpty);
      // Reopening an interrupted self-hosted invite follows legacy recovery.
      await f.storage.save(saved.copyWith(starterSetupIncomplete: true));
      f.container.invalidate(communityListProvider);
      f.container
          .read(pendingDeepLinkProvider.notifier)
          .open(
            Uri(
              scheme: 'buzz',
              host: 'join',
              queryParameters: {
                'relay': saved.relayUrl,
                'code': 'consumed-invite',
              },
            ),
          );
      await f.settle(tester);
      expect(f.recoveryScopes, hasLength(1));
      expect(
        (await f.storage.loadAll()).single.starterSetupIncomplete,
        isFalse,
      );
      expect(f.sent('/claim'), isEmpty);

      expect(find.byType(OnboardingAppEntry), findsNothing);
    },
  );

  journey(
    'signed-in invite survives restart and leaving consent abandons only setup',
    (tester, f) async {
      await f.initialize();
      await f.mount(tester);
      await f.link(tester, 'one');
      expect(find.text('Accept and join').hitTestable(), findsOneWidget);
      await f.restart(tester);
      await f.tap(tester, 'Start login again');
      expect(find.text('Accept and join').hitTestable(), findsOneWidget);
      expect(f.browserStarts, 0);
      expect(
        f.sent('/api/join-policy').last.url.host,
        'one.communities.buzz.xyz',
      );
      Navigator.of(
        tester.element(find.text('Accept and join')),
        rootNavigator: true,
      ).pop();
      await f.settle(tester);
      await f.restart(tester);
      expect(find.text('Start login again'), findsNothing);
      expect(find.text('Community home').hitTestable(), findsOneWidget);
      expect(f.sent('/accept-policy'), isEmpty);
      expect(f.sent('/claim'), isEmpty);
      expect(f.probes, isEmpty);
      expect(await f.storage.loadAll(), isEmpty);
      expect(await f.storage.loadActiveId(), isNull);
      expect((await f.store.read())!.pubkey, f.keys.public);

      // An open relay's successful AUTH must not convert a failed invitation
      // into a completed claim for a non-member.
      f.offerProfile = false;
      f.inviteExpired = true;
      f.missingList = true;
      await f.link(tester, 'one');
      await tester.tap(find.byType(CheckboxListTile));
      await f.settle(tester);
      await f.tap(tester, 'Accept and join');
      expect(await f.storage.loadAll(), isEmpty);
      expect(f.probes, isEmpty);

      expect(
        find.text('This invite has expired. Ask for a new invite link.'),
        findsOneWidget,
      );
      f.missingList = false;
      await f.link(tester, 'one');
      await tester.tap(find.byType(CheckboxListTile));
      await f.settle(tester);
      await f.tap(tester, 'Accept and join');
      expect(await f.storage.loadAll(), isEmpty);
      expect(f.probes, isEmpty);

      // The server admits this identity but its reply is lost. The retained
      // intent resumes after restart even once the code has expired.
      f.inviteExpired = false;
      f.missingList = false;
      f.loseClaimResponse = true;
      await f.link(tester, 'one');
      await tester.tap(find.byType(CheckboxListTile));
      await f.settle(tester);
      await f.tap(tester, 'Accept and join');
      expect(await f.storage.loadAll(), isEmpty);
      expect(f.rows, hasLength(1));
      f.inviteExpired = true;
      f.rejectList = true;
      await f.restart(tester);
      await f.tap(tester, 'Start login again');
      await tester.tap(find.byType(CheckboxListTile));
      await f.settle(tester);
      await f.tap(tester, 'Accept and join');
      expect(await f.storage.loadAll(), isEmpty);
      expect(f.probes, isEmpty);
      f.rejectList = false;
      await f.tap(tester, 'Try again');
      await tester.tap(find.byType(CheckboxListTile));
      await f.settle(tester);
      await f.tap(tester, 'Accept and join');
      final saved = (await f.storage.loadAll()).single;
      expect(saved.relayUrl, 'wss://one.communities.buzz.xyz');
      expect(saved.pubkey, f.keys.public);
      expect(await f.storage.loadActiveId(), saved.id);
      expect(find.text('Community home').hitTestable(), findsOneWidget);
    },
  );

  journey(
    'create survives a lost response and restart, then joins the same owned community',
    (tester, f) async {
      await f.initialize();
      f.loseCreateResponse = true;
      await f.mount(tester);
      await f.tap(tester, 'Create a community');
      await tester.enterText(find.byType(TextField).hitTestable(), 'new-home');
      await f.tap(tester, 'Continue');
      expect(find.text('Try again').hitTestable(), findsOneWidget);
      expect(f.owned, hasLength(1));
      expect(await f.storage.loadAll(), isEmpty);
      await f.restart(tester);
      await f.tap(tester, 'Start login again');
      expect(find.byType(HostedProfileSetup), findsOneWidget);
      await tester.enterText(find.byType(TextField).hitTestable(), 'Creator');
      await f.tap(tester, 'Continue');
      final saved = (await f.storage.loadAll()).single;
      expect(saved.relayUrl, 'wss://new-home.communities.buzz.xyz');
      expect(saved.nsec, f.keys.nsec);
      expect(await f.storage.loadActiveId(), saved.id);
      expect(f.sent('/v1/buzz/communities'), hasLength(2));
      expect(f.owned, hasLength(1));
      expect(f.sent('/status'), isEmpty);
      expect(f.browserStarts, 0);
      expect(f.profileWrites, ['Creator']);
      expect(find.text('Community home').hitTestable(), findsOneWidget);
    },
  );

  journey(
    'second invitation replaces pending consent and only joins the newer community',
    (tester, f) async {
      await f.initialize();
      f.offerProfile = false;
      // The first dispatched attempt ends; retry owns the pending consent.
      // A new incoming link must now invalidate that consent, not wait on it.
      f.policyFailures = 1;
      await f.mount(tester);
      await f.link(tester, 'one');
      await f.tap(tester, 'Try again');
      expect(find.text('Accept and join').hitTestable(), findsOneWidget);
      await f.link(tester, 'two');
      await tester.tap(find.byType(CheckboxListTile));
      await f.settle(tester);
      await f.tap(tester, 'Accept and join');
      final saved = (await f.storage.loadAll()).single;
      expect(saved.relayUrl, 'wss://two.communities.buzz.xyz');
      expect(saved.starterSetupIncomplete, isFalse);
      expect(f.sent('/claim'), hasLength(1));
      expect(f.sent('/claim').single.url.host, 'two.communities.buzz.xyz');
      expect(f.sent('/accept-policy'), hasLength(1));
      expect(
        f.sent('/accept-policy').single.url.host,
        'two.communities.buzz.xyz',
      );
      expect(find.byType(OnboardingAppEntry), findsNothing);
    },
  );
}
