import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/features/invites/invite_join_provider.dart';
import 'package:buzz/shared/hosted/hosted_flow_provider.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_admission.dart';
import 'package:buzz/shared/hosted/hosted_community_connection.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../helpers/hosted_communities_fixture.dart';

final _finishProvider = Provider(
  (ref) =>
      (Community community, bool Function() current) =>
          finishHostedStarterSetup(ref, community, current),
);

class _Recovery implements InviteJoinRecovery {
  _Recovery(this.run);
  final Future<void> Function() run;
  @override
  Future<String?> ensureStarterChannels() async {
    await run();
    return 'welcome-everyone';
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  const invite = InviteDeepLink(
    relayUrl: 'wss://invited.communities.buzz.xyz',
    code: 'single-use',
  );
  late HostedCommunitiesFixture f;
  late CommunityStorage storage;
  late HostedAdmission admission;
  late ProviderContainer container;
  late Community unrelated;
  late List<InviteJoinRecoveryScope> scopes;
  late List<Community> probed;
  late Future<void> Function() recover;
  var claims = 0;
  var current = true;

  setUp(() async {
    FlutterSecureStorage.setMockInitialValues({});
    f = HostedCommunitiesFixture();
    await f.initialize();
    storage = CommunityStorage();
    unrelated = Community.create(
      name: 'Other',
      relayUrl: 'wss://other.example',
      pubkey: f.keys.public,
      nsec: f.keys.nsec,
    );
    await storage.save(unrelated);
    await storage.saveActiveId(unrelated.id);
    claims = 0;
    current = true;
    scopes = [];
    probed = [];
    recover = () async {};
    admission = HostedAdmission(
      account: f.account,
      relayAuth: HostedRelayAuth(f.account, enabled: false),
      store: f.store,
      probe: (community) async {
        probed.add(community);
      },
      client: MockClient((request) async {
        if (request.method == 'GET' && request.url.path == '/api/join-policy') {
          return http.Response('{}', 200);
        }
        expect(request.method, 'POST');
        expect(request.url.path, '/api/invites/claim');
        claims++;
        return http.Response('{}', claims == 1 ? 200 : 410);
      }),
    );
    container = ProviderContainer(
      overrides: [
        communityStorageProvider.overrideWithValue(storage),
        inviteJoinRecoveryProvider.overrideWithValue((scope) {
          scopes.add(scope);
          return _Recovery(() => recover());
        }),
      ],
    );
  });
  tearDown(() async {
    container.dispose();
    admission.dispose();
    await f.dispose();
  });

  Future<Community> savedInvite() async => (await storage.loadAll())
      .singleWhere((community) => community.relayUrl == invite.relayUrl);

  Future<void> connect() => connectHostedCommunity(
    link: invite,
    subject: 'owner',
    pubkey: f.keys.public,
    isCurrent: () => current,
    storage: storage,
    admission: admission,
    persist: (community, isCurrent) =>
        container.read(communityTransitionProvider).runExclusive(() async {
          if (!isCurrent()) return;
          await storage.save(community);
          await storage.saveActiveId(community.id);
          container.invalidate(communityListProvider);
          container.invalidate(activeCommunityProvider);
        }),
    recoverStarterChannels: container.read(_finishProvider),
  );

  test(
    'pending retry still rejects a changed account binding before probe or recovery',
    () async {
      recover = () async {
        throw StateError('Starter membership refused');
      };
      await expectLater(connect(), throwsStateError);
      f.bound = nostr.Keys('2'.padLeft(64, '0')).public;
      await expectLater(connect(), throwsA(AccountFailure.denied));
      expect((await savedInvite()).starterSetupIncomplete, isTrue);
      expect(claims, 1);
      expect(probed, hasLength(1));
      expect(scopes, hasLength(1));
    },
  );

  test('cancellation during recovery keeps durable pending flag', () async {
    recover = () async {
      current = false;
    };
    await expectLater(connect(), throwsA(AccountFailure.unavailable));
    expect((await savedInvite()).starterSetupIncomplete, isTrue);
    expect(claims, 1);
  });

  test(
    'switching active community during recovery cannot clear pending flag',
    () async {
      recover = () => storage.saveActiveId(unrelated.id);
      await expectLater(connect(), throwsA(AccountFailure.unavailable));
      expect((await savedInvite()).starterSetupIncomplete, isTrue);
      expect(await storage.loadActiveId(), unrelated.id);
    },
  );

  test(
    'clearing pending flag preserves newer saved community metadata',
    () async {
      recover = () =>
          container.read(communityTransitionProvider).runExclusive(() async {
            await storage.save(
              (await savedInvite()).copyWith(name: 'Renamed while joining'),
            );
          });
      await connect();
      final saved = await savedInvite();
      expect(saved.starterSetupIncomplete, isFalse);
      expect(saved.name, 'Renamed while joining');
      expect((await storage.loadAll()).first.toJson(), unrelated.toJson());
    },
  );

  test(
    'saved invite with another key is never reclaimed or overwritten',
    () async {
      final other = nostr.Keys('2'.padLeft(64, '0'));
      final saved = Community.create(
        name: 'Existing',
        relayUrl: invite.relayUrl,
        nsec: other.nsec,
        pubkey: other.public,
        starterSetupIncomplete: true,
      );
      await storage.save(saved);
      await expectLater(
        connect(),
        throwsA(isA<SavedCommunityIdentityConflict>()),
      );
      expect(claims, 0);
      expect(probed, isEmpty);
      expect(scopes, isEmpty);
      expect((await savedInvite()).toJson(), saved.toJson());
    },
  );
}
