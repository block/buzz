import 'dart:async';

import 'package:buzz/features/invites/invite_join_provider.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../shared/community/community_storage_test.dart';

void main() {
  for (final finishReadBeforeJoin in [false, true]) {
    test('first invite setup succeeds when old active read finishes '
        '${finishReadBeforeJoin ? 'before' : 'after'} new selection', () async {
      final storage = _PausedActiveReadStorage();
      final old = Community.create(
        name: 'Existing',
        relayUrl: 'wss://old.example.com',
        nsec: nostr.Keys.generate().nsec,
      );
      await storage.save(old);
      await storage.saveActiveId(old.id);
      var recoveryCalls = 0;
      final container = ProviderContainer(
        overrides: [
          communityStorageProvider.overrideWithValue(storage),
          communitySnapshotWriterProvider.overrideWithValue((_) async {}),
          inviteJoinHttpClientProvider.overrideWithValue(
            MockClient(
              (_) async => http.Response(
                '{"status":"joined","host":"new.example.com","role":"member"}',
                200,
              ),
            ),
          ),
          inviteJoinRecoveryProvider.overrideWithValue((_) {
            recoveryCalls++;
            return _SuccessfulRecovery();
          }),
        ],
      );
      addTearDown(container.dispose);
      // The app restores authentication before displaying the invite.
      await container.read(authProvider.future);
      storage.pauseNextRead = true;
      final oldProjection = container.read(activeCommunityProvider.future);
      await storage.readStarted.future;
      if (finishReadBeforeJoin) {
        storage.resumeRead.complete();
        await oldProjection;
      }
      final join = container.read(inviteJoinProvider.notifier);
      await join.prepare(
        const InviteDeepLink(
          relayUrl: 'wss://new.example.com',
          code: 'test-invite',
        ),
      );
      await join.confirmJoin();
      await oldProjection;

      final result = container.read(inviteJoinProvider);
      expect(result.status, InviteJoinStatus.success);
      expect(result.errorMessage, isNull);
      expect(recoveryCalls, 1);
      final communities = await storage.loadAll();
      final joined = communities.singleWhere((c) => c.id != old.id);
      expect(await storage.loadActiveId(), joined.id);
      expect(joined.starterSetupIncomplete, isFalse);
      expect(
        (await container.read(activeCommunityProvider.future))?.id,
        joined.id,
      );
    });
  }
}

class _SuccessfulRecovery implements InviteJoinRecovery {
  @override
  Future<String?> ensureStarterChannels() async => 'welcome';
}

/// Delays the storage read after the provider captures its community list.
/// All reads and writes succeed; only their ordering changes.
class _PausedActiveReadStorage extends CommunityStorage {
  _PausedActiveReadStorage() : super(secure: FakeSecureStorage());

  bool pauseNextRead = false;
  final readStarted = Completer<void>();
  final resumeRead = Completer<void>();

  @override
  Future<String?> loadActiveId() async {
    if (pauseNextRead) {
      pauseNextRead = false;
      readStarted.complete();
      await resumeRead.future;
    }
    return super.loadActiveId();
  }

  @override
  Future<void> saveActiveId(String id) async {
    await super.saveActiveId(id);
    if (readStarted.isCompleted && !resumeRead.isCompleted) {
      resumeRead.complete();
    }
  }
}
