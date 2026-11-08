import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../deeplink/deep_link.dart';
import 'hosted_communities.dart';
import 'hosted_environment.dart';
import 'hosted_relay_auth.dart';
import 'known_community_sync_provider.dart';

/// Account destinations for the switcher. Listing never imports a community or
/// changes the active identity; admission happens only after a selection.
final knownCommunitiesProvider =
    FutureProvider.autoDispose<List<AddCommunityDeepLink>>((ref) async {
      if (!HostedEnvironment.enabled) return const [];
      final account = ref.watch(builderlabAccountProvider);
      final sync = ref.watch(knownCommunitySyncProvider);
      void changed() => ref.invalidateSelf();
      sync?.addListener(changed);
      ref.onDispose(() => sync?.removeListener(changed));
      account.subjectChanges.addListener(changed);
      ref.onDispose(() => account.subjectChanges.removeListener(changed));
      final session = await account.restore();
      if (!ref.mounted || session == null) return const [];
      final communities = await loadKnownAccountCommunities(account);
      if (!ref.mounted || account.account?.subject != session.subject) {
        return const [];
      }
      final removed =
          await sync?.pendingRemovals(session.subject) ?? <String>{};
      if (!ref.mounted || account.account?.subject != session.subject) {
        return const [];
      }
      return communities
          .where((community) => !removed.contains(community.relayUrl))
          .toList(growable: false);
    });
