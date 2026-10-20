import '../community/community.dart';
import 'account_identity_store.dart';
import 'builderlab_account.dart';
import 'hosted_environment.dart';

/// The same hosted requirements apply to every new community, regardless of
/// whether its credentials came from an invite, key import, or verified pairing.
abstract final class CommunityEntryRequirements {
  /// Nostr-only entry cannot bypass a configured account requirement.
  static void requireSelfHosted(String relayUrl) {
    if (HostedEnvironment.requiresAuth(Uri.parse(relayUrl).host)) {
      throw StateError(
        'This community requires Builderlab sign-in and identity '
        'backup. Open its link through community setup.',
      );
    }
  }

  static Future<void> verify(
    Community community,
    BuilderlabAccountClient account,
  ) async {
    final uri = Uri.parse(community.relayUrl);
    if (!HostedEnvironment.requiresAuth(uri.host)) return;
    if (!HostedEnvironment.supportsRelay(uri)) throw AccountFailure.denied;
    {
      final session = account.account ?? await account.restore();
      if (session == null) throw AccountFailure.signedOut;
      final nsec = community.nsec;
      if (nsec == null) throw AccountIdentityFailure.absent;
      final pubkey = AccountIdentityStore.publicKeyForNsec(nsec);
      if (community.pubkey != pubkey) throw AccountIdentityFailure.mismatch;
      await verifyHostedIdentity(
        account: account,
        store: AccountIdentityStore(),
        subject: session.subject,
        pubkey: pubkey,
      );
    }
  }

  static Future<void> verifyHostedIdentity({
    required BuilderlabAccountClient account,
    required AccountIdentityStore store,
    required String subject,
    required String pubkey,
  }) async {
    if (account.account?.subject != subject) {
      throw AccountFailure.differentAccount;
    }
    final snapshot = await store.forSubject(subject).read();
    if (snapshot?.pubkey != pubkey ||
        !snapshot!.hasBackupFor(
          backend: HostedEnvironment.origin.origin,
          subject: subject,
        )) {
      throw AccountFailure.denied;
    }
    final binding = await account.currentIdentity();
    if (account.account?.subject != subject) {
      throw AccountFailure.differentAccount;
    }
    final identity = binding['identity'];
    if (identity is! Map || identity['pubkey_hex'] != pubkey) {
      throw AccountFailure.denied;
    }
  }
}
