import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/features/pairing/pairing_provider.dart';
import 'package:buzz/shared/deeplink/pending_deep_link_provider.dart';
import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/profile/profile_setup_page.dart';
import 'package:buzz/features/profile/profile_setup_avatar_editor.dart';
import 'package:buzz/features/profile/profile_avatar_draft.dart';
import 'package:buzz/shared/emoji/emoji_avatar.dart';
import 'package:buzz/shared/widgets/playing_avatar_image.dart';
import 'package:buzz/features/pairing/pairing_page.dart';
import 'package:buzz/onboarding_app_entry.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/relay/relay_session.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:buzz/shared/relay/media_image.dart';
import 'package:buzz/shared/utils/string_utils.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;

import '../../helpers/hosted_journey_fixture.dart';

void main() {
  void journey(
    String name,
    Future<void> Function(WidgetTester, HostedJourneyFixture) run, {
    bool requiresAuthDomain = false,
    bool requiresBuilderlab = true,
  }) {
    testWidgets(
      name,
      (tester) async {
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
      },
      skip:
          (requiresBuilderlab && HostedEnvironment.authDomains.isEmpty) ||
          (requiresAuthDomain &&
              !HostedEnvironment.requiresAuth('one.communities.buzz.xyz')),
    );
  }

  for (final importing in [false, true]) {
    journey(
      'new account ${importing ? 'imports' : 'creates'} and keeps its identity on retry',
      (tester, f) async {
        await f.initialize(localIdentity: false);
        f.bound = null;
        f.offerProfile = false;
        await f.mount(tester);
        await f.tap(tester, 'Continue with Builderlab');
        expect(find.text('Create my Buzz identity'), findsOneWidget);
        expect(find.text('Import existing identity'), findsOneWidget);
        expect(await f.store.read(), isNull);
        if (importing) {
          await f.tap(tester, 'Import existing identity');
          await tester.enterText(
            find.byType(TextField).hitTestable(),
            f.keys.nsec,
          );
          await f.tap(tester, 'Preview identity');
          final started = Completer<void>();
          final release = Completer<void>();
          f.beforeStatusReturn = () async {
            if (!started.isCompleted) started.complete();
            await release.future;
          };
          await f.tap(tester, 'Import this identity');
          await started.future;
          // Dismiss while the real import is awaiting its external status response.
          Navigator.of(
            tester.element(find.byType(TextField).hitTestable()),
          ).pop();
          await f.settle(tester);
          release.complete();
          f.beforeStatusReturn = null;
          await f.settle(tester);
          expect(f.bound, f.keys.public);
        } else {
          await f.tap(tester, 'Create my Buzz identity');
        }
        final saved = await f.store.read();
        expect(saved, isNotNull);
        expect(f.bound, saved!.pubkey);
        expect(saved.backup, isNotNull);
        await f.flow.retry();
        await f.settle(tester);
        expect((await f.store.read())!.pubkey, saved.pubkey);
      },
    );
  }

  journey('linked account without backup imports its key without replacing it', (
    tester,
    f,
  ) async {
    await f.initialize(localIdentity: false);
    f.offerProfile = false;
    await f.mount(tester);
    await f.tap(tester, 'Continue with Builderlab');
    expect(find.text('Create my Buzz identity'), findsNothing);
    expect(find.text('Import existing identity'), findsOneWidget);
    await f.tap(tester, 'Import existing identity');
    await tester.enterText(
      find.byType(TextField).hitTestable(),
      f.secondKeys.nsec,
    );
    await f.tap(tester, 'Preview identity');
    await f.tap(tester, 'Import this identity');
    expect(
      find.text(
        'This key doesn’t match the identity linked to your Builderlab account.',
      ),
      findsOneWidget,
    );
    expect(await f.store.read(), isNull);
    expect(f.bound, f.keys.public);
    await tester.enterText(find.byType(TextField).hitTestable(), f.keys.nsec);
    await f.tap(tester, 'Preview identity');
    await f.tap(tester, 'Import this identity');
    expect((await f.store.read())!.pubkey, f.keys.public);
    expect((await f.store.read())!.backup, isNotNull);
    expect(f.requests.where((r) => r.url.path.endsWith('/recovery')), isEmpty);
  });

  journey('system Back leaves community creation', (tester, f) async {
    await f.initialize();
    await f.mount(tester);
    await f.tap(tester, 'Create a community');
    expect(
      tester.widget<PairingPage>(find.byType(PairingPage)).creatingCommunity,
      isTrue,
    );
    await tester.binding.handlePopRoute();
    await f.settle(tester);
    expect(
      tester.widget<PairingPage>(find.byType(PairingPage)).creatingCommunity,
      isFalse,
    );
    expect(find.text('Create a community').hitTestable(), findsOneWidget);
  });

  journey('retiring profile setup removes both profile and avatar routes', (
    tester,
    f,
  ) async {
    await f.initialize();
    f.rows.add(f.row('one'));
    await f.mount(tester);
    await f.tap(tester, 'Continue with Builderlab');
    await tester.tap(find.byTooltip('Add profile photo'));
    await f.settle(tester);
    expect(find.byType(ProfileSetupAvatarEditor), findsOneWidget);
    await f.link(tester, 'two');
    expect(
      find.byType(ProfileSetupAvatarEditor, skipOffstage: false),
      findsNothing,
    );
    expect(find.text('Set up your profile', skipOffstage: false), findsNothing);
    expect(f.profileWrites, isEmpty);
  });

  journey('fallback profile previews the accepted draft before saving', (
    tester,
    f,
  ) async {
    await f.initialize();
    f.rows.add(f.row('one'));
    await f.mount(tester);
    await f.tap(tester, 'Continue with Builderlab');
    await tester.tap(find.byTooltip('Add profile photo'));
    await f.settle(tester);
    final url = emojiAvatarDataUrl('🐝', 0xFFFFE75C);
    Navigator.of(
      tester.element(find.byType(ProfileSetupAvatarEditor)),
    ).pop(ProfileUrlAvatarDraft(url));
    await f.settle(tester);
    expect(
      tester
          .widget<PlayingAvatarImage>(find.byType(PlayingAvatarImage))
          .imageUrl,
      url,
    );
    await tester.enterText(
      find.byType(TextField).hitTestable(),
      'Draft Person',
    );
    await f.tap(tester, 'Continue');
    expect(f.profileWrites, ['Draft Person']);
    expect(f.profileAvatars, [url]);
  });

  journey(
    'QR credentials enter without an account when domain is not required',
    (tester, f) async {
      if (HostedEnvironment.requiresAuth('paired.communities.buzz.xyz')) return;
      // Seed a previous account session: an opt-out build must not restore it.
      await f.initialize(localIdentity: false);
      f.requests.clear();
      await f.mount(
        tester,
        pairingValidator: ({required relayUrl, required nsec}) async {},
      );
      if (HostedEnvironment.authDomains.isEmpty) {
        expect(f.activeFlow, isNull);
        expect(f.requests, isEmpty);
        expect(find.text('Connect to Builderlab'), findsNothing);
        expect(find.text('Continue with Builderlab'), findsNothing);
        expect(find.text('Create a community'), findsNothing);
        expect(find.text('Enter a community link'), findsOneWidget);
        expect(find.text('Scan a QR code'), findsOneWidget);
        expect(find.text('Use pairing code'), findsNothing);
        await f.tap(tester, 'Enter a community link');
        expect(
          find.byKey(const Key('community-link-input')).hitTestable(),
          findsOneWidget,
        );
        expect(f.activeFlow, isNull);
        await f.tap(tester, 'Back');
        expect(f.requests, isEmpty);
      }
      final notifier = f.container.read(pairingProvider.notifier);
      await notifier.pair(
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
      await f.settle(tester);
      final saved = (await f.storage.loadAll()).single;
      expect(saved.nsec, f.keys.nsec);
      expect(saved.pubkey, f.keys.public);
      expect(f.sent('/exchange'), isEmpty);
      expect(f.sent('/current'), isEmpty);
      expect(f.sent('/nsec-backups/status'), isEmpty);
      expect(find.text('Sign in to Builderlab to reconnect'), findsNothing);
    },
    requiresBuilderlab: false,
  );

  journey(
    'saved community signs in again without changing identity or onboarding',
    (tester, f) async {
      f.relayAuthEnabled = true;
      await f.initialize(signedIn: false);
      final community = Community.create(
        name: 'One',
        relayUrl: 'wss://one.communities.buzz.xyz',
        pubkey: f.keys.public,
        nsec: f.keys.nsec,
      );
      await f.storage.save(community);
      await f.storage.saveActiveId(community.id);
      await f.mount(tester, openEntry: false);
      // The real signed-in app watches its session; the fixture's home is inert.
      final sessionSubscription = f.container.listen(
        relaySessionProvider,
        (_, _) {},
      );
      addTearDown(sessionSubscription.close);
      final session = f.container.read(relaySessionProvider.notifier);
      await f.settle(tester);
      if (!HostedEnvironment.requiresAuth(Uri.parse(community.relayUrl).host)) {
        expect(find.text('Sign in to Builderlab to reconnect'), findsNothing);
        expect(f.socketHeaders, isNotEmpty);
        expect(f.socketHeaders, everyElement(isEmpty));
        expect(f.sent('/assertions'), isEmpty);
        expect(f.container.read(relayConfigProvider).nsec, community.nsec);
        return;
      }
      expect(find.text('Sign in to Builderlab to reconnect'), findsOneWidget);
      expect(f.socketHeaders, isEmpty);

      final browser = Completer<void>();
      f.beforeBrowserReturn = () => browser.future;
      await f.tap(tester, 'Sign in');
      await f.tap(tester, 'Cancel sign-in');
      browser.complete();
      await f.settle(tester);
      expect(find.text('Sign in to Builderlab to reconnect'), findsOneWidget);
      expect(f.socketHeaders, isEmpty);

      f.beforeBrowserReturn = null;
      await f.tap(tester, 'Sign in');
      await f.settle(tester);
      expect(f.socketHeaders, isNotEmpty);
      expect(find.text('Sign in to Builderlab to reconnect'), findsNothing);
      await session.queryRelay(const []);
      expect(f.sent('/query'), hasLength(1));
      final proofRequest = f.sent('/identity/assertions').single;
      expect(
        proofRequest.headers['X-BB-Session-Credential'],
        'synthetic-session',
      );
      final proof =
          jsonDecode(
                utf8.decode(
                  base64Decode(
                    proofRequest.headers['Authorization']!.substring(6),
                  ),
                ),
              )
              as Map;
      expect(proof['pubkey'], f.keys.public);
      expect(
        proof['tags'],
        contains(equals(['u', proofRequest.url.toString()])),
      );

      // Production media transport attaches only to its captured community,
      // refuses redirects, and renews an expired assertion without browser UI.
      final media = f.container.read(mediaHttpClientProvider);
      for (final origin in [
        'https://one.communities.buzz.xyz',
        'https://outside.example',
      ]) {
        await media.get(Uri.parse('$origin/auth-transport'));
        final request = f.requests.last;
        expect(
          request.headers['Nostr-Federated-Identity'],
          origin.contains('communities.buzz.xyz')
              ? 'Bearer synthetic-assertion'
              : isNull,
        );
        if (origin.contains('communities.buzz.xyz')) {
          expect(request.followRedirects, isFalse);
        }
      }
      final logins = f.browserStarts;
      final issued = f.sent('/identity/assertions').length;
      f.assertionNow = f.assertionNow.add(const Duration(minutes: 6));
      await session.queryRelay(const []);
      expect(f.sent('/identity/assertions'), hasLength(issued + 1));
      expect(f.browserStarts, logins);

      f.expiredSession = true;
      f.assertionNow = f.assertionNow.add(const Duration(minutes: 6));
      await expectLater(
        session.queryRelay(const []),
        throwsA(AccountFailure.signedOut),
      );
      await f.settle(tester);
      expect(find.text('Sign in to Builderlab to reconnect'), findsOneWidget);
      final beforeWrongAccount = f.sent('/identity/assertions').length;
      f.bound = f.secondKeys.public;
      f.subject = 'wrong-owner';
      await f.tap(tester, 'Sign in');
      await f.settle(tester);
      expect(
        find.text(
          'This account cannot reconnect to this community with its saved key.',
        ),
        findsOneWidget,
      );
      expect(f.sent('/identity/assertions'), hasLength(beforeWrongAccount));
      // An empty different account cannot claim the saved key through reconnect.
      f.bound = null;
      await f.tap(tester, 'Use a different account');
      await f.settle(tester);
      expect(
        find.text(
          'This account cannot reconnect to this community with its saved key.',
        ),
        findsOneWidget,
      );
      expect(f.sent('/identity/assertions'), hasLength(beforeWrongAccount));
      expect(f.bound, isNull);
      expect(await f.store.read(), isNull);
      expect((await f.storage.loadAll()).single.nsec, community.nsec);
      f.subject = 'journey-owner';
      f.bound = f.keys.public;
      await f.tap(tester, 'Use a different account');
      await f.settle(tester);
      expect(
        f.container.read(builderlabAccountProvider).account!.subject,
        'journey-owner',
      );
      expect(find.text('Use a different account'), findsNothing);
      expect((await f.storage.loadAll()).single.nsec, community.nsec);
      // A legacy account can link the existing saved key on first issuance.
      f.bound = null;
      f.assertionNow = f.assertionNow.add(const Duration(minutes: 6));
      await session.reconnect();
      await f.settle(tester);
      expect(f.bound, f.keys.public);
      expect(
        find.text(
          'This account cannot reconnect to this community with its saved key.',
        ),
        findsNothing,
      );
      expect((await f.storage.loadAll()).single.nsec, community.nsec);
      expect(await f.storage.loadActiveId(), community.id);
      expect((await f.store.read())!.pubkey, f.keys.public);
      for (final path in [
        '/status',
        '/enrollment',
        '/known-communities/list',
      ]) {
        expect(
          f.sent(path),
          isEmpty,
          reason: 'Reconnect must not run onboarding',
        );
      }
      // The issuer can succeed while the relay refuses the upgrade. One
      // renewal is allowed, then stop; a user retry can recover later.
      f.refuseUpgrade = true;
      final attempts = f.socketHeaders.length;
      await session.reconnect();
      await f.settle(tester);
      expect(f.socketHeaders.length, attempts + 2);
      expect(
        f.container.read(relaySessionProvider).status,
        SessionStatus.disconnected,
      );
      expect(find.text('Use a different account'), findsOneWidget);
      f.refuseUpgrade = false;
      await session.reconnect();
      await f.settle(tester);

      // A timed-out query cannot dispatch later when a slow issuer completes.
      final stalled = Completer<void>();
      f.beforeAssertionReturn = () => stalled.future;
      f.assertionNow = f.assertionNow.add(const Duration(minutes: 6));
      final queries = f.sent('/query').length;
      await expectLater(
        session.queryRelay(const [], timeout: const Duration(milliseconds: 20)),
        throwsA(isA<TimeoutException>()),
      );
      stalled.complete();
      await f.settle(tester);
      expect(f.sent('/query'), hasLength(queries));
      f.beforeAssertionReturn = null;
      expect(f.container.read(relayConfigProvider).nsec, community.nsec);
    },
    requiresBuilderlab: false,
  );

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
      await f.settle(tester);
      await tester.tap(find.text('Create my Buzz identity'));
      await enrollmentReached.future.timeout(const Duration(seconds: 5));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 400));
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
      expect(
        find.byType(ProfileSetupPage, skipOffstage: false),
        findsOneWidget,
      );
      expect(
        find.ancestor(
          of: find.byType(TextField).hitTestable(),
          matching: find.byType(BottomSheet),
        ),
        findsOneWidget,
      );
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
    requiresAuthDomain: true,
  );

  for (final boundAccount in [false, true]) {
    journey(
      'switch accounts preserves both keys and requires unbound reuse consent ($boundAccount)',
      (tester, f) async {
        await f.initialize();
        final original = f.store;
        await f.mount(tester);
        await f.container.read(builderlabAccountProvider).clearLocalSession();
        f.subject = 'new-owner';
        f.bound = boundAccount ? f.secondKeys.public : null;
        if (boundAccount) f.cloudKeys.add(f.secondKeys);
        await f.tap(tester, 'Continue with Builderlab');
        if (!boundAccount) {
          expect(find.text('Saved identity'), findsOneWidget);
          expect(f.sent('/verify'), isEmpty);
          expect(f.sent('/nsec-backups'), isEmpty);
          expect(await f.store.read(), isNull);
          await f.tap(tester, fullNpub(f.keys.public)!);
        }
        final saved = (await f.store.read())!;
        expect(
          saved.pubkey,
          boundAccount ? f.secondKeys.public : f.keys.public,
        );
        expect(saved.backup!.subject, 'new-owner');
        expect((await original.read())!.pubkey, f.keys.public);
        expect((await original.read())!.backup!.subject, 'journey-owner');
      },
    );
  }

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
      expect(find.text('Use pairing code'), findsNothing);
      f.rows.addAll([f.row('one'), f.row('two')]);
      await f.tap(tester, 'Continue with Builderlab');
      // Joined communities are added automatically after choosing the identity.
      expect(find.byType(CheckboxListTile).hitTestable(), findsNothing);
      expect(
        find.byType(ProfileSetupPage, skipOffstage: false),
        findsOneWidget,
      );
      final firstSaved = (await f.storage.loadAll()).firstWhere(
        (c) => c.name == 'one.communities.buzz.xyz',
      );
      Navigator.of(
        tester.element(find.byType(ProfileSetupPage, skipOffstage: false)),
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
      expect(
        saved.singleWhere((c) => c.name == 'one.communities.buzz.xyz').id,
        firstSaved.id,
      );
      expect(await f.storage.loadActiveId(), firstSaved.id);
      expect(
        f.probes.where((c) => c.name == 'one.communities.buzz.xyz'),
        hasLength(1),
      );
      expect(
        f.probes.where((c) => c.name == 'two.communities.buzz.xyz'),
        hasLength(2),
      );
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
        expect(
          find.byType(ProfileSetupPage, skipOffstage: false),
          findsNothing,
        );
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
      f.requests.clear();
      await f.mount(tester);
      await f.tap(tester, 'Enter a community link');
      await tester.enterText(
        find.byKey(const Key('community-link-input')).hitTestable(),
        'my-community.example',
      );
      await f.tap(tester, 'Continue');
      // Choices loaded successfully; enumeration breaks before selection.
      // The account-scoped key remains directly readable throughout admission.
      f.identityStorage.failEnumeration = true;
      await f.tap(tester, 'Saved identity');
      final saved = (await f.storage.loadAll()).single;
      expect(saved.relayUrl, 'wss://my-community.example');
      expect(saved.nsec, f.keys.nsec);
      expect(await f.storage.loadActiveId(), saved.id);
      expect(f.probes, hasLength(1));
      expect(f.sent('/claim'), isEmpty);
      f.identityStorage.failEnumeration = false;
      expect(
        (await const FlutterSecureStorage().readAll()).keys.where(
          (key) => key.startsWith('buzz_pending_self_hosted_'),
        ),
        isEmpty,
      );
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
      await f.container
          .read(communityListProvider.notifier)
          .removeCommunity(saved.id);
      await AccountIdentityStore(
        subject: 'other-owner',
      ).create(f.secondKeys.nsec);
      await f.restart(tester);
      await f.tap(tester, 'Community home');
      await f.tap(tester, 'Enter a community link');
      await tester.enterText(
        find.byKey(const Key('community-link-input')).hitTestable(),
        'my-community.example',
      );
      await f.tap(tester, 'Continue');
      // A refused key must not pin the URL against an explicit allowed choice.
      f.refusedSelfHostedPubkey = f.keys.public;
      await f.tap(tester, fullNpub(f.keys.public)!);
      expect(await f.storage.loadAll(), isEmpty);
      expect(find.textContaining('Could not join.'), findsOneWidget);
      await f.tap(tester, fullNpub(f.secondKeys.public)!);
      expect((await f.storage.loadAll()).single.nsec, f.secondKeys.nsec);
      expect(
        (await AccountIdentityStore().forPairing(f.keys.public).read())!.pubkey,
        f.keys.public,
      );
      if (HostedEnvironment.authDomains.isEmpty) {
        expect(f.requests, isEmpty);
      }
    },
    requiresBuilderlab: false,
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
    requiresAuthDomain: true,
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
      expect(
        find.byType(ProfileSetupPage, skipOffstage: false),
        findsOneWidget,
      );
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

  for (final (sentinel, guidance, retryable) in [
    (
      'invite_restricted',
      'This account, or the account that owns it, is restricted in this community. Contact a community administrator. If the restriction is temporary, try again after it ends.',
      true,
    ),
    (
      'invite_owner_mismatch',
      'This account does not match its recorded owner. Use the correct account or owner identity to join this community.',
      false,
    ),
  ]) {
    journey('$sentinel is rendered without discarding the hosted invite', (
      tester,
      f,
    ) async {
      await f.initialize();
      f.offerProfile = false;
      f.claimError = sentinel;
      // Even recorded membership must not turn a current explicit denial
      // into successful admission through lost-response reconciliation.
      f.rows.add(f.row('one'));
      await f.mount(tester);
      await f.link(tester, 'one');
      await tester.tap(find.byType(CheckboxListTile));
      await f.settle(tester);
      await f.tap(tester, 'Accept and join');
      expect(find.text(guidance), findsOneWidget);
      expect(await f.storage.loadAll(), isEmpty);
      expect(f.probes, isEmpty);
      expect(f.sent('/claim'), hasLength(1));
      expect(f.sent('/communities/joined'), isEmpty);
      expect(
        find.text('Try again').hitTestable(),
        retryable ? findsOneWidget : findsNothing,
      );
      await f.settle(tester);
      expect(f.sent('/claim'), hasLength(1));
      expect((await f.flow.browser.retainedIntent())!['code'], 'invite-one');
      f.claimError = null;
      if (retryable) {
        await f.tap(tester, 'Try again');
      } else {
        // Owner mismatch requires the correct identity, not an automatic
        // retry. Explicitly abandon setup, then reopen the same link after
        // the server-side owner conflict has been corrected.
        await f.tap(tester, 'Leave setup');
        expect((await f.store.read())!.pubkey, f.keys.public);
        await f.link(tester, 'one');
      }
      await tester.tap(find.byType(CheckboxListTile));
      await f.settle(tester);
      await f.tap(tester, 'Accept and join');
      expect(f.sent('/claim'), hasLength(2));
      for (final request in f.sent('/claim')) {
        expect(jsonDecode(request.body)['code'], 'invite-one');
      }
      expect((await f.storage.loadAll()).single.pubkey, f.keys.public);
      expect(find.text('Community home').hitTestable(), findsOneWidget);
    }, requiresAuthDomain: true);
  }

  journey(
    'second invitation replaces pending consent and only joins the newer community',
    (tester, f) async {
      await f.initialize();
      f.offerProfile = false;
      f.saveFailures = 1;
      // The first dispatched attempt ends; retry owns the pending consent.
      // A new incoming link must now invalidate that consent, not wait on it.
      f.policyResponses.addAll([
        http.Response('<html>Unavailable</html>', 502),
        http.Response('', 503),
        http.Response('{"error":"proxy_busy"}', 503),
        http.Response('', 429),
      ]);
      await f.mount(tester);
      await f.link(tester, 'one');
      while (f.policyResponses.isNotEmpty) {
        expect(find.text('Try again').hitTestable(), findsOneWidget);
        expect(await f.storage.loadAll(), isEmpty);
        await f.tap(tester, 'Try again');
      }
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
      // The remote add was accepted but its acknowledgement was lost. The
      // saved community and pending intent survive; retry never reclaims it.
      expect(find.text('Try again').hitTestable(), findsOneWidget);
      expect(f.knownSaved, {'wss://two.communities.buzz.xyz'});
      await f.restart(tester);
      await f.tap(tester, 'Start login again');
      expect((await f.storage.loadAll()).single.id, saved.id);
      expect(f.sent('/claim'), hasLength(1));
      expect(f.sent('/known-communities/add'), hasLength(2));
      expect(find.byType(OnboardingAppEntry), findsNothing);
    },
    requiresAuthDomain: true,
  );
}
