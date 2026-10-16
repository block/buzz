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
      f.starterFailures = 1;
      await f.mount(tester);
      await f.link(tester, 'one');
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
      await f.tap(tester, 'Try again');
      expect(find.byType(HostedProfileSetup), findsOneWidget);
      expect(f.sent('/claim'), hasLength(1));
      expect(f.recoveryScopes, hasLength(2));
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
      f.rows.addAll([f.row('one'), f.row('two')]);
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
      expect(find.byType(CheckboxListTile).hitTestable(), findsNWidgets(2));
      await f.tap(tester, 'Continue');
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

  journey(
    'new device recovers bound cloud key and joins without replacing identity',
    (tester, f) async {
      await f.initialize(localIdentity: false);
      f.cloudKeys.addAll([f.secondKeys, f.keys]);
      f.rows.add(f.row('one'));
      f.offerProfile = false;
      await f.mount(tester);
      await f.tap(tester, 'Continue with Builderlab');
      // One discovered community is admitted automatically.
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
      expect(find.byType(OnboardingAppEntry), findsNothing);
    },
  );

  journey(
    'leaving consent performs no policy acceptance, claim or durable admission',
    (tester, f) async {
      await f.initialize();
      await f.mount(tester);
      await f.link(tester, 'one');
      expect(find.text('Accept and join').hitTestable(), findsOneWidget);
      await f.tap(tester, 'Leave setup');
      expect(find.text('Enter a community link').hitTestable(), findsOneWidget);
      expect(f.sent('/accept-policy'), isEmpty);
      expect(f.sent('/claim'), isEmpty);
      expect(f.probes, isEmpty);
      expect(await f.storage.loadAll(), isEmpty);
      expect(await f.storage.loadActiveId(), isNull);
      expect((await f.store.read())!.pubkey, f.keys.public);
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
