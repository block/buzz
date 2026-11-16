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
      final builtFor = account.account?.subject;
      // A new account reloads, so it never shows the previous account's list;
      // any other change refreshes behind the current list. The first listener
      // to fire disposes the other, so both decide from the account itself.
      void changed() =>
          ref.invalidateSelf(asReload: account.account?.subject != builtFor);
      sync?.addListener(changed);
      ref.onDispose(() => sync?.removeListener(changed));
      account.subjectChanges.addListener(changed);
      ref.onDispose(() => account.subjectChanges.removeListener(changed));
      final session = account.account ?? await account.restore();
      if (!ref.mounted || session == null) return const [];
      final communities = await loadKnownAccountCommunities(account);
      if (!ref.mounted || account.account?.subject != session.subject) {
        return const [];
      }
      return communities;
    });
