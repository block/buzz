part of 'hosted_flow.dart';

/// The browser sign-in intent that carries [target] through authentication.
Map<String, String> _encode(BuzzDeepLink target) {
  if (target is HostedCreateCommunity) {
    return {'type': 'create', 'name': target.name};
  }
  if (target is NostrBindDeepLink) return {'type': 'bind', ...target.fields};
  if (target is AddCommunityDeepLink) {
    return {'type': 'add', 'relay': target.relayUrl, 'name': target.name};
  }
  final invite = target as InviteDeepLink;
  return {
    'type': 'invite',
    'relay': invite.relayUrl,
    'code': invite.code,
    if (invite.policyReceipt != null) 'policy_receipt': invite.policyReceipt!,
  };
}

/// The deep link a browser sign-in intent carried, or null if it's invalid.
BuzzDeepLink? _decode(Map<String, String> intent) {
  if (intent['type'] == 'create') {
    final name = intent['name'];
    return name != null && HostedCreateCommunity.validName(name)
        ? HostedCreateCommunity(name)
        : null;
  }
  final fields = Map<String, String>.of(intent)
    ..remove('type')
    ..remove('import_identity')
    ..remove('paired_pubkey')
    ..remove('paired_protection');
  final host = switch (intent['type']) {
    'bind' => 'nostr-bind',
    'add' => 'add-community',
    'invite' => 'join',
    _ => '',
  };
  return parseBuzzDeepLink(
    Uri(scheme: 'buzz', host: host, queryParameters: fields),
  );
}
