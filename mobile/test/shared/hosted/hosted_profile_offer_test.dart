import 'dart:async';

import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/shared/hosted/hosted_flow_provider.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_flow.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../helpers/hosted_communities_fixture.dart';

final _offerProvider = Provider(
  (ref) =>
      (
        HostedFlow flow,
        BuzzDeepLink link,
        String pubkey,
        bool Function() current,
      ) => offerHostedProfile(ref, flow, link, pubkey, current),
);

class _Profile extends ProfileNotifier {
  _Profile(this.load, this.writes);
  final Future<UserProfile?> Function() load;
  final List<String> writes;
  @override
  Future<UserProfile?> build() => load();
  @override
  Future<void> updateDisplayName(String name) async => writes.add(name);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  late HostedCommunitiesFixture f;
  late CommunityStorage storage;
  late ProviderContainer container;
  late Community community;
  late Future<UserProfile?> Function() load;
  late List<String> writes;
  var current = true;
  var reads = 0;
  const link = AddCommunityDeepLink(
    relayUrl: 'wss://joined.communities.buzz.xyz',
    name: 'Joined',
  );

  setUp(() async {
    FlutterSecureStorage.setMockInitialValues({});
    f = HostedCommunitiesFixture();
    await f.initialize();
    storage = CommunityStorage();
    community = Community.create(
      name: 'Joined',
      relayUrl: link.relayUrl,
      pubkey: f.keys.public,
      nsec: f.keys.nsec,
    );
    await storage.save(community);
    await storage.saveActiveId(community.id);
    writes = [];
    reads = 0;
    current = true;
    load = () async => null;
    container = ProviderContainer(
      overrides: [
        communityStorageProvider.overrideWithValue(storage),
        profileProvider.overrideWith(
          () => _Profile(() {
            reads++;
            return load();
          }, writes),
        ),
      ],
    );
  });
  tearDown(() async {
    container.dispose();
    await f.dispose();
  });

  Future<void> offer() => container.read(_offerProvider)(
    f.flow,
    link,
    f.keys.public,
    () => current,
  );
  Future<void> prompted() async {
    for (var i = 0; i < 50 && f.flow.stage != HostedFlowStage.profile; i++) {
      await Future<void>.delayed(Duration.zero);
    }
    expect(f.flow.stage, HostedFlowStage.profile);
  }

  test('account cancellation while loading prevents prompt', () async {
    final loading = Completer<UserProfile?>();
    load = () => loading.future;
    final pending = offer();
    while (reads == 0) {
      await Future<void>.delayed(Duration.zero);
    }
    current = false;
    loading.complete(null);
    await pending;
    expect(f.flow.stage, HostedFlowStage.idle);
    expect(writes, isEmpty);
  });

  test('changed active community blocks a pending save', () async {
    final pending = offer();
    await prompted();
    final other = Community.create(
      name: 'Other',
      relayUrl: 'wss://other.example',
      pubkey: f.keys.public,
      nsec: f.keys.nsec,
    );
    await storage.save(other);
    await storage.saveActiveId(other.id);
    container.invalidate(communityListProvider);
    container.invalidate(activeCommunityProvider);
    expect(
      (await container.read(activeCommunityProvider.future))?.id,
      other.id,
    );
    await expectLater(
      f.flow.saveProfile!('Wrong scope'),
      throwsA(AccountFailure.unavailable),
    );
    expect(writes, isEmpty);
    f.flow.finishProfile();
    await pending;
  });

  test(
    'new target releases pending profile and rejects retained save',
    () async {
      final pending = f.flow.requestProfile((name) async => writes.add(name));
      final save = f.flow.saveProfile!;
      // Unsupported environment finishes locally without browser or network IO.
      final next = f.flow.start(
        const AddCommunityDeepLink(
          relayUrl: 'wss://other.example',
          name: 'Other',
        ),
      );
      await pending.timeout(const Duration(seconds: 2));
      await next;
      await expectLater(
        save('Stale prompt'),
        throwsA(AccountFailure.unavailable),
      );
      expect(writes, isEmpty);
      expect(f.flow.stage, isNot(HostedFlowStage.profile));
    },
  );

  test(
    'profile draft retries without completing and skip does not prepare media',
    () async {
      var prepares = 0;
      var attempts = 0;
      final pending = f.flow.requestProfile(
        (_) async {},
        saveDraft: (name, prepare) async {
          await prepare?.call();
          if (++attempts == 1) throw StateError('offline');
          writes.add(name);
        },
      );
      final save = f.flow.saveProfileDraft!;
      Future<String> prepare() async {
        prepares++;
        return 'https://media.example/photo';
      }

      await expectLater(save('Name', prepare), throwsStateError);
      expect(f.flow.stage, HostedFlowStage.profile);
      await save('Name', prepare);
      await pending;
      expect(writes, ['Name']);
      expect(prepares, 2);
      final skipped = f.flow.requestProfile(
        (_) async {},
        saveDraft: (_, prepare) async {
          await prepare?.call();
        },
      );
      final stale = f.flow.saveProfileDraft!;
      f.flow.finishProfile();
      await skipped;
      await expectLater(
        stale('Old', prepare),
        throwsA(AccountFailure.unavailable),
      );
      expect(prepares, 2);
    },
  );

  for (final cancelAdmission in [false, true]) {
    test(
      'leave rejects captured save (cancel admission: $cancelAdmission)',
      () async {
        final pending = offer();
        await prompted();
        final save = f.flow.saveProfile!;
        if (cancelAdmission) current = false;
        await f.flow.leave();
        await pending;
        await expectLater(
          save('After leave'),
          throwsA(AccountFailure.unavailable),
        );
        expect(writes, isEmpty);
      },
    );
  }
}
