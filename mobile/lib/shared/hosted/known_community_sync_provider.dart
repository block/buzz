import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../community/community_provider.dart';
import 'hosted_environment.dart';
import 'hosted_relay_auth.dart';
import 'known_community_sync.dart';

/// Account sync is owned by the app, independent of the active relay session.
final knownCommunitySyncProvider = Provider<KnownCommunitySync?>((ref) {
  if (!HostedEnvironment.enabled) return null;
  final sync = KnownCommunitySync(
    account: ref.watch(builderlabAccountProvider),
    localCommunities: () =>
        ref.read(communityStorageProvider).loadForAccountSync(),
    acknowledgeRemoval: (revision) =>
        ref.read(communityStorageProvider).acknowledgeRemoval(revision),
  );
  ref.onDispose(sync.dispose);
  return sync;
});
