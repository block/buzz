import '../deeplink/deep_link.dart';
import 'builderlab_account.dart';
import 'hosted_environment.dart';

enum HostedCreationFailure {
  rejected,

  /// Another community already uses this address.
  taken,
}

/// Internal setup intent; no external deep link can request provisioning.
class HostedCreateCommunity extends AddCommunityDeepLink {
  HostedCreateCommunity(String name)
    : super(
        name: name,
        relayUrl: 'wss://$name.${HostedEnvironment.communitySuffix}',
      ) {
    if (!validName(name)) throw ArgumentError.value(name, 'name');
  }

  static const _path = '/v1/buzz/communities';

  static bool validName(String name) =>
      name.length <= 63 && RegExp(r'^[a-z0-9]+(?:-[a-z0-9]+)*$').hasMatch(name);

  /// The server reconciles the same owner/name on replay, including a lost
  /// response. Keep the name stable and validate ownership before admission.
  Future<AddCommunityDeepLink> provision(
    BuilderlabAccountClient account,
    String pubkey,
    bool Function() isCurrent,
  ) async {
    final subject = account.account?.subject;
    void check() {
      if (!isCurrent() ||
          subject == null ||
          account.account?.subject != subject) {
        throw AccountFailure.unavailable;
      }
    }

    check();
    final Map<String, dynamic> response;
    account.lastRejectedRequest = null;
    try {
      response = await account.post(_path, {'name': name});
    } on AccountFailure catch (failure) {
      check();
      if (failure == AccountFailure.rejected) {
        throw switch (account.lastRejectedRequest) {
          (path: _path, status: 409, code: 'taken') =>
            HostedCreationFailure.taken,
          _ => HostedCreationFailure.rejected,
        };
      }
      rethrow;
    }
    check();
    if (response['error'] != null) throw AccountFailure.invalidResponse;
    return _validate(response['community'], pubkey);
  }

  AddCommunityDeepLink _validate(dynamic row, String pubkey) {
    if (row is! Map<String, dynamic> ||
        row['normalized_host'] != Uri.parse(relayUrl).host ||
        row['owner_pubkey'] != pubkey ||
        row['name'] != name) {
      throw AccountFailure.invalidResponse;
    }
    if (row['archived_at'] != null) throw HostedCreationFailure.rejected;
    return AddCommunityDeepLink(relayUrl: relayUrl, name: name);
  }
}
