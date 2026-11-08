import 'builderlab_account.dart';
import 'hosted_identity_service.dart';

/// The organization account's bound identity, which its relay admits and
/// nothing else. Null when the account has no binding yet: the tenant binds
/// the key of its first relay assertion.
Future<String?> organizationBoundPubkey(BuilderlabAccountClient account) async {
  final response = await account.currentIdentity();
  final identity = response['identity'];
  if (response['error'] != null) throw AccountFailure.invalidResponse;
  if (identity == null) return null;
  final pubkey = identity is Map ? identity['pubkey_hex'] : null;
  if (pubkey is! String || !RegExp(r'^[a-fA-F0-9]{64}$').hasMatch(pubkey)) {
    throw AccountFailure.invalidResponse;
  }
  return pubkey.toLowerCase();
}

/// The bound key from the organization's backups, for a member whose key is
/// not on this device: after signing out, or first use of this device. Null
/// when no openable backup holds that exact key.
Future<String?> recoverOrganizationKey(
  BuilderlabAccountClient account,
  String bound,
) async {
  final Map<String, dynamic> recovered;
  try {
    recovered = await account.post('/v1/buzz/nsec-backups/recovery', {});
  } on AccountFailure catch (error) {
    // A tenant without backups refuses definitively.
    if (error == AccountFailure.rejected) return null;
    rethrow;
  }
  return openRecoveredBackups(recovered)?[bound]?.keys.nsec;
}
