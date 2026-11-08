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
    localRelays: () async =>
        (await ref.read(communityStorageProvider).loadAll())
            .map((community) => community.relayUrl)
            .toList(),
  );
  ref.onDispose(sync.dispose);
  return sync;
});
