import 'dart:async';
import 'dart:convert';

import 'package:buzz/app.dart';
import 'package:buzz/features/age_gate/age_restriction_page.dart';
import 'package:buzz/features/age_gate/age_signal_push_bootstrap.dart';
import 'package:buzz/features/age_gate/age_signal_provider.dart';
import 'package:buzz/features/channels/unread_badge/unread_badge_provider.dart';
import 'package:buzz/features/home/home_page.dart';
import 'package:buzz/features/pairing/pairing_provider.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/huddle/huddle.dart';
import 'package:nostr/nostr.dart' as nostr;
import '../../shared/community/community_storage_test.dart'
    show FakeSecureStorage;
import 'package:buzz/shared/push/push_bootstrap.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  const appBadgeChannel = MethodChannel('app_badge_plus');

  tearDown(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(ageSignalChannel, null);
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(appBadgeChannel, null);
  });

  testWidgets(
    'an in-flight credential commit cannot reopen restricted access',
    (tester) async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final storage = _PausedCommunityStorage();
      final age = _MutableAgeSignalNotifier();
      var connections = 0;
      final relay = RelaySessionNotifier(
        socketFactory:
            ({
              required wsUrl,
              required nsec,
              required onMessage,
              required onConnected,
              required onDisconnected,
            }) {
              connections++;
              throw StateError(
                'Restricted authentication must not open a socket',
              );
            },
      );
      final container = ProviderContainer(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          communityStorageProvider.overrideWithValue(storage),
          communitySnapshotWriterProvider.overrideWithValue((_) async {}),
          ageSignalProvider.overrideWith(() => age),
          relaySessionProvider.overrideWith(() => relay),
          pairingProvider.overrideWith(
            () => PairingNotifier(
              credentialValidator:
                  ({required relayUrl, required nsec}) async {},
            ),
          ),
        ],
      );
      addTearDown(container.dispose);
      await container.read(authProvider.future);
      final listener = container.listen(relaySessionProvider, (_, _) {});
      addTearDown(listener.close);
      await tester.pumpWidget(
        UncontrolledProviderScope(container: container, child: const App()),
      );
      await tester.pump();
      final code = base64Url.encode(
        utf8.encode(
          jsonEncode({
            'relayUrl': 'https://relay.example',
            'nsec': nostr.Keys.generate().nsec,
          }),
        ),
      );
      final pairing = container.read(pairingProvider.notifier).pair(code);
      await tester.pump();
      expect(storage.started.isCompleted, isTrue);
      age.setState(AgeSignalState.restricted);
      await tester.pump();
      expect(find.byType(AgeRestrictionPage), findsOneWidget);
      storage.release.complete();
      await pairing;
      await tester.pump();
      expect((await storage.loadAll()), hasLength(1));
      expect(
        (await container.read(authProvider.future)).status,
        AuthStatus.authenticated,
      );
      expect(container.read(pairingProvider).status, PairingStatus.idle);
      expect(find.byType(AgeRestrictionPage), findsOneWidget);
      expect(
        container.read(relaySessionProvider).status,
        SessionStatus.disconnected,
      );
      expect(connections, 0);
      expect(
        container.read(huddleSessionProvider).phase,
        HuddleSessionPhase.idle,
      );
      await tester.pumpWidget(const SizedBox.shrink());
      listener.close();
      container.dispose();
      await tester.pump(const Duration(milliseconds: 1));
    },
  );

  test('backs off repeated snapshot transition failures', () {
    expect(ageSignalPushSnapshotRetryDelay(0), const Duration(seconds: 5));
    expect(ageSignalPushSnapshotRetryDelay(1), const Duration(seconds: 10));
    expect(ageSignalPushSnapshotRetryDelay(5), const Duration(seconds: 160));
    expect(ageSignalPushSnapshotRetryDelay(6), const Duration(minutes: 5));
    expect(ageSignalPushSnapshotRetryDelay(100), const Duration(minutes: 5));
  });

  for (final restrict in [false, true]) {
    testWidgets('pending legacy pairing follows restriction=$restrict', (
      tester,
    ) async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final age = _MutableAgeSignalNotifier();
      final auth = _RecordingPairAuthNotifier();
      final validation = Completer<void>();
      final pairing = PairingNotifier(
        credentialValidator: ({required relayUrl, required nsec}) =>
            validation.future,
      );
      final container = ProviderContainer(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          authProvider.overrideWith(() => auth),
          ageSignalProvider.overrideWith(() => age),
          pairingProvider.overrideWith(() => pairing),
        ],
      );
      addTearDown(container.dispose);
      await tester.pumpWidget(
        UncontrolledProviderScope(container: container, child: const App()),
      );
      await tester.pump();
      final code = base64Url.encode(
        utf8.encode(
          jsonEncode({
            'relayUrl': 'https://relay.example',
            'nsec': 'pending-key',
          }),
        ),
      );
      final pending = container.read(pairingProvider.notifier).pair(code);
      expect(container.read(pairingProvider).status, PairingStatus.connecting);
      if (restrict) age.setState(AgeSignalState.restricted);
      await tester.pump();
      validation.complete();
      await pending;
      expect(auth.imports, restrict ? 0 : 1);
      expect(
        container.read(pairingProvider).status,
        restrict ? PairingStatus.idle : PairingStatus.success,
      );
      await tester.pumpWidget(const SizedBox.shrink());
    });
  }

  testWidgets('blocks authenticated app content', (tester) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          authProvider.overrideWith(() => _AuthenticatedAuthNotifier()),
          ageSignalProvider.overrideWith(() => _BlockingAgeSignalNotifier()),
          savedPrefsProvider.overrideWithValue(prefs),
        ],
        child: const AgeSignalPushBootstrap(child: App()),
      ),
    );
    await tester.pump();

    expect(find.byType(AgeRestrictionPage), findsOneWidget);
    expect(find.byType(HomePage), findsNothing);
  });

  testWidgets('clears the app badge only after a confirmed restriction', (
    tester,
  ) async {
    final badgeCounts = <int>[];
    final ageSignal = _MutableAgeSignalNotifier();
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(appBadgeChannel, (call) async {
          if (call.method == 'updateBadge') {
            badgeCounts.add(
              (call.arguments as Map<Object?, Object?>)['count']! as int,
            );
          }
          return null;
        });
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          authProvider.overrideWith(() => _UnauthenticatedAuthNotifier()),
          ageSignalProvider.overrideWith(() => ageSignal),
          unreadBadgeProvider.overrideWithValue(
            const UnreadBadgeState(highPriorityCount: 7),
          ),
          savedPrefsProvider.overrideWithValue(prefs),
        ],
        child: const App(),
      ),
    );
    await tester.pump();

    expect(badgeCounts, isNotEmpty);
    expect(badgeCounts.last, 7);

    ageSignal.setState(AgeSignalState.allowed);
    await tester.pump();
    expect(badgeCounts.last, 7);

    ageSignal.setState(AgeSignalState.restricted);
    await tester.pump();
    expect(badgeCounts.last, 0);
  });

  testWidgets(
    'opens app and restores push snapshots without a native age check',
    (tester) async {
      final relaySession = _CountingRelaySessionNotifier();
      var requests = 0;
      var snapshotRestorations = 0;
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(ageSignalChannel, (call) {
            requests += 1;
            throw PlatformException(code: 'unavailable');
          });
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();

      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            authProvider.overrideWith(() => _AuthenticatedAuthNotifier()),
            relaySessionProvider.overrideWith(() => relaySession),
            ageAllowedNotificationRestorerProvider.overrideWithValue(() async {
              snapshotRestorations += 1;
              if (snapshotRestorations == 1) {
                throw StateError('injected restoration failure');
              }
            }),
            ageSignalPushSnapshotRetryWaitProvider.overrideWithValue(
              (_) async {},
            ),
            savedPrefsProvider.overrideWithValue(prefs),
          ],
          child: const AgeSignalPushBootstrap(child: App()),
        ),
      );

      await tester.pump();
      await tester.pump();

      expect(find.bySemanticsLabel('Checking age eligibility'), findsNothing);
      expect(find.byType(HomePage), findsOneWidget);
      expect(find.byType(Navigator), findsOneWidget);
      expect(relaySession.builds, 1);
      expect(requests, ageGatingEnabled ? 1 : 0);
      await tester.pump();
      await tester.pump();
      expect(snapshotRestorations, 2);
    },
  );

  for (final outcome in ['minor', 'adult', 'error', 'malformed', 'timeout']) {
    testWidgets('normal app and push start before native $outcome result', (
      tester,
    ) async {
      final response = Completer<Object?>();
      var requests = 0;
      var restrictions = 0;
      final relaySession = _CountingRelaySessionNotifier();
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(ageSignalChannel, (_) {
            requests += 1;
            return response.future;
          });
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            authProvider.overrideWith(() => _AuthenticatedAuthNotifier()),
            relaySessionProvider.overrideWith(() => relaySession),
            ageSignalProvider.overrideWith(
              () =>
                  AgeSignalNotifier(requestTimeout: const Duration(seconds: 1)),
            ),
            ageAllowedNotificationRestorerProvider.overrideWithValue(
              () async {},
            ),
            ageRestrictedNotificationPurgerProvider.overrideWithValue(() async {
              restrictions++;
            }),
            savedPrefsProvider.overrideWithValue(prefs),
          ],
          child: const AgeSignalPushBootstrap(child: App()),
        ),
      );
      await tester.pump();
      expect(find.byType(HomePage), findsOneWidget);
      expect(find.byType(BuzzPushBootstrap), findsOneWidget);
      expect(relaySession.builds, 1);
      expect(requests, 1);

      if (outcome == 'timeout') {
        await tester.pump(const Duration(seconds: 2));
        response.complete({'status': 'signal', 'ageUpper': 17});
      } else if (outcome == 'error') {
        response.completeError(PlatformException(code: 'unavailable'));
      } else if (outcome == 'malformed') {
        response.complete({'status': 'signal', 'ageUpper': '17'});
      } else {
        response.complete({
          'status': 'signal',
          'ageUpper': outcome == 'minor' ? 17 : 18,
        });
      }
      await tester.pump();
      await tester.pump();
      final restricted = outcome == 'minor';
      expect(restrictions, restricted ? 1 : 0);
      expect(
        find.byType(AgeRestrictionPage),
        restricted ? findsOneWidget : findsNothing,
      );
      expect(find.byType(HomePage), restricted ? findsNothing : findsOneWidget);
      expect(
        find.byType(BuzzPushBootstrap),
        restricted ? findsNothing : findsOneWidget,
      );
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pump();
    });
  }

  testWidgets(
    'purges restricted notifications before community storage recovers',
    (tester) async {
      var purges = 0;

      await tester.pumpWidget(
        ProviderScope(
          retry: (_, _) => null,
          overrides: [
            ageSignalProvider.overrideWith(() => _BlockingAgeSignalNotifier()),
            communityListProvider.overrideWith(
              () => _UnavailableCommunityListNotifier(),
            ),
            ageRestrictedNotificationPurgerProvider.overrideWithValue(() async {
              purges += 1;
            }),
          ],
          child: const AgeSignalPushBootstrap(child: SizedBox()),
        ),
      );
      await tester.pump();

      expect(purges, 1);
    },
  );

  testWidgets('retries a failed restricted notification purge', (tester) async {
    var purges = 0;

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          ageSignalProvider.overrideWith(() => _BlockingAgeSignalNotifier()),
          communityListProvider.overrideWith(
            () => _UnavailableCommunityListNotifier(),
          ),
          ageRestrictedNotificationPurgerProvider.overrideWithValue(() async {
            purges += 1;
            if (purges == 1) {
              throw StateError('injected notification purge failure');
            }
          }),
          ageSignalPushSnapshotRetryWaitProvider.overrideWithValue(
            (_) async {},
          ),
        ],
        child: const AgeSignalPushBootstrap(child: SizedBox()),
      ),
    );
    await tester.pump();
    await tester.pump();

    expect(purges, 2);
  });

  testWidgets(
    'retries a successful purge for interactions donated by stale extensions',
    (tester) async {
      var purges = 0;
      final scheduledMaintenance = <VoidCallback>[];

      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            ageSignalProvider.overrideWith(() => _BlockingAgeSignalNotifier()),
            communityListProvider.overrideWith(
              () => _UnavailableCommunityListNotifier(),
            ),
            ageRestrictedNotificationPurgerProvider.overrideWithValue(() async {
              purges += 1;
            }),
            ageRestrictedNotificationMaintenanceScheduleProvider
                .overrideWithValue((callback) {
                  scheduledMaintenance.add(callback);
                  return () {};
                }),
          ],
          child: const AgeSignalPushBootstrap(child: SizedBox()),
        ),
      );
      await tester.pump();
      expect(purges, 1);
      expect(scheduledMaintenance, hasLength(1));

      for (
        var attempt = 0;
        attempt < ageRestrictedNotificationMaintenancePurgeLimit;
        attempt += 1
      ) {
        scheduledMaintenance.removeAt(0)();
        await tester.pump();
        await tester.pump();
      }

      expect(purges, 1 + ageRestrictedNotificationMaintenancePurgeLimit);
      expect(scheduledMaintenance, isEmpty);
    },
  );
}

class _AuthenticatedAuthNotifier extends AuthNotifier {
  @override
  Future<AuthState> build() async {
    return const AuthState(status: AuthStatus.authenticated);
  }
}

class _UnauthenticatedAuthNotifier extends AuthNotifier {
  @override
  Future<AuthState> build() async {
    return const AuthState(status: AuthStatus.unauthenticated);
  }
}

class _MutableAgeSignalNotifier extends AgeSignalNotifier {
  @override
  AgeSignalState build() => AgeSignalState.allowed;

  @override
  Future<void> request() async {}

  void setState(AgeSignalState next) => state = next;
}

class _BlockingAgeSignalNotifier extends AgeSignalNotifier {
  @override
  AgeSignalState build() => AgeSignalState.restricted;

  @override
  Future<void> request() async {}
}

class _CountingRelaySessionNotifier extends RelaySessionNotifier {
  int builds = 0;

  @override
  SessionState build() {
    builds += 1;
    return const SessionState(status: SessionStatus.disconnected);
  }
}

class _UnavailableCommunityListNotifier extends CommunityListNotifier {
  @override
  Future<List<Community>> build() async {
    throw StateError('secure storage unavailable');
  }
}

class _RecordingPairAuthNotifier extends _UnauthenticatedAuthNotifier {
  int imports = 0;

  @override
  Future<void> authenticateWithCommunity(Community community) async {
    imports += 1;
  }
}

class _PausedCommunityStorage extends CommunityStorage {
  _PausedCommunityStorage() : super(secure: FakeSecureStorage());
  final started = Completer<void>();
  final release = Completer<void>();

  @override
  Future<void> save(Community community) async {
    started.complete();
    await release.future;
    await super.save(community);
  }
}
