import '../community/community.dart';
import '../community/community_storage.dart';
import '../deeplink/deep_link.dart';
import 'builderlab_account.dart';
import 'hosted_admission.dart';

/// A saved community uses another identity. Only explicit skip/leave is safe.
class SavedCommunityIdentityConflict implements Exception {
  const SavedCommunityIdentityConflict();
}

/// Canonical community identity shared by discovery and deep-link entry.
({bool secure, String host, int port}) relayIdentity(String value) {
  final uri = Uri.parse(value);
  final secure = uri.scheme == 'wss' || uri.scheme == 'https';
  return (
    secure: secure,
    host: uri.host.toLowerCase(),
    port: uri.hasPort ? uri.port : (secure ? 443 : 80),
  );
}

/// Reuses existing records and their ids before the normal admission/persistence
/// boundary. A different existing key is never replaced.
Future<void> connectHostedCommunity({
  required BuzzDeepLink link,
  required String subject,
  required String pubkey,
  required bool Function() isCurrent,
  required CommunityStorage storage,
  required HostedAdmission admission,
  required Future<void> Function(Community, bool Function()) persist,
  required Future<void> Function(Community, bool Function())
  recoverStarterChannels,
}) async {
  final relay = link.relayUrl ?? (throw AccountFailure.invalidResponse);
  final communities = await storage.loadAll();
  final existing = communities
      .where((item) => relayIdentity(item.relayUrl) == relayIdentity(relay))
      .firstOrNull;
  if (existing != null && existing.pubkey != pubkey) {
    throw const SavedCommunityIdentityConflict();
  }
  if (!isCurrent()) return;
  // Saved membership survives a partially completed invite setup. Revalidate
  // identity and probe the relay, but never reclaim its consumed invite.
  final admissionLink = existing != null && link is InviteDeepLink
      ? AddCommunityDeepLink(relayUrl: relay, name: existing.name)
      : link;
  final prepared = await admission.prepare(
    admissionLink,
    subject,
    pubkey,
    isCurrent: isCurrent,
  );
  final community = existing == null
      ? prepared.copyWith(starterSetupIncomplete: link is InviteDeepLink)
      : existing.copyWith(nsec: prepared.nsec);
  if (!isCurrent()) return;
  await persist(community, isCurrent);
  if (!isCurrent()) return;
  if (community.starterSetupIncomplete) {
    await recoverStarterChannels(community, isCurrent);
  }
}
