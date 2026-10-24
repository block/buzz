import '../deeplink/deep_link.dart';
import 'builderlab_account.dart';
import 'hosted_environment.dart';

/// Validated, active communities joined by the signed-in Builderlab identity.
/// Discovery never grants admission; each target still goes through HostedAdmission.
Future<List<AddCommunityDeepLink>> loadHostedCommunities(
  BuilderlabAccountClient account,
) async {
  final response = await account.post('/v1/buzz/communities/joined', {});
  final rows = response['communities'];
  if (response['error'] != null || rows is! List) {
    throw AccountFailure.invalidResponse;
  }
  final communities = <String, AddCommunityDeepLink>{};
  for (final row in rows) {
    if (row is! Map<String, dynamic>) throw AccountFailure.invalidResponse;
    final archived = row['archived_at'];
    if (archived != null) {
      if (archived is! String || archived.isEmpty) {
        throw AccountFailure.invalidResponse;
      }
      continue;
    }
    final host = row['normalized_host'];
    final name = row['name'];
    if (host is! String || name is! String || name.trim().isEmpty) {
      throw AccountFailure.invalidResponse;
    }
    final relay = Uri.tryParse(host.contains('://') ? host : 'wss://$host');
    if (relay == null ||
        relay.scheme != 'wss' ||
        !RegExp(r'^[a-z0-9]+(?:[.-][a-z0-9]+)*$').hasMatch(relay.host) ||
        relay.userInfo.isNotEmpty ||
        relay.hasQuery ||
        relay.hasFragment ||
        (relay.path.isNotEmpty && relay.path != '/') ||
        (relay.hasPort && relay.port != 443)) {
      throw AccountFailure.invalidResponse;
    }
    // A hosted listing can contain communities this build cannot enter.
    // Never redirect the account session or discard otherwise usable entries.
    if (!HostedEnvironment.supportsRelay(relay)) continue;
    final canonical = Uri(scheme: 'wss', host: relay.host);
    communities.putIfAbsent(
      canonical.host,
      () => AddCommunityDeepLink(relayUrl: canonical.toString(), name: name),
    );
  }
  return List.unmodifiable(communities.values);
}
