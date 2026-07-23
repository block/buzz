/// Parsing for `buzz://` deep links.
///
/// Mirrors the desktop handler in `desktop/src-tauri/src/deep_link.rs`:
/// `buzz://message?channel=<uuid>&id=<hex>[&thread=<hex>]` references a
/// message (optionally inside a thread) in a channel. Required params that
/// are missing or empty make the link invalid — the caller never sees a
/// half-formed target.
library;

/// A parsed deep link supported by the app.
sealed class BuzzDeepLink {
  const BuzzDeepLink();
}

/// A parsed relay invite link.
///
/// Canonical share links are `https://<relay>/invite/<code>`. The custom
/// `buzz://join?relay=<ws(s)://relay>&code=<code>` form is only an installed-app
/// handoff from the web landing page.
class InviteDeepLink extends BuzzDeepLink {
  /// Relay URL normalized to the websocket scheme used by the app.
  final String relayUrl;

  /// Invite code from the link.
  final String code;

  /// Optional receipt proving acceptance of the relay's current join policy.
  final String? policyReceipt;

  const InviteDeepLink({
    required this.relayUrl,
    required this.code,
    this.policyReceipt,
  });

  @override
  bool operator ==(Object other) =>
      other is InviteDeepLink &&
      other.relayUrl == relayUrl &&
      other.code == code &&
      other.policyReceipt == policyReceipt;

  @override
  int get hashCode => Object.hash(relayUrl, code, policyReceipt);

  @override
  String toString() =>
      'InviteDeepLink(relay: $relayUrl, code: $code, policyReceipt: $policyReceipt)';
}

/// A parsed `buzz://message` deep link.
class MessageDeepLink extends BuzzDeepLink {
  /// Channel UUID from the `channel` query param.
  final String channelId;

  /// Event ID (hex) from the `id` query param.
  final String messageId;

  /// Optional thread root event ID from the `thread` query param.
  final String? threadRootId;

  const MessageDeepLink({
    required this.channelId,
    required this.messageId,
    this.threadRootId,
  });

  @override
  bool operator ==(Object other) =>
      other is MessageDeepLink &&
      other.channelId == channelId &&
      other.messageId == messageId &&
      other.threadRootId == threadRootId;

  @override
  int get hashCode => Object.hash(channelId, messageId, threadRootId);

  @override
  String toString() =>
      'MessageDeepLink(channel: $channelId, id: $messageId, '
      'thread: $threadRootId)';
}

/// Parse a `buzz://message?…` URI into a [MessageDeepLink].
///
/// Returns `null` for non-`buzz` schemes, non-`message` hosts (e.g.
/// `buzz://connect` which is desktop-only), or links missing a non-empty
/// `channel` or `id` param.
MessageDeepLink? parseMessageDeepLink(Uri uri) {
  if (uri.scheme != 'buzz' || uri.host != 'message') return null;

  final channel = uri.queryParameters['channel'];
  final id = uri.queryParameters['id'];
  if (channel == null || channel.isEmpty || id == null || id.isEmpty) {
    return null;
  }

  final thread = uri.queryParameters['thread'];
  return MessageDeepLink(
    channelId: channel,
    messageId: id,
    threadRootId: (thread == null || thread.isEmpty) ? null : thread,
  );
}

/// Parse canonical HTTPS invite links and `buzz://join` app handoffs.
///
/// Accepted forms:
/// - `https://<relay>/invite/<code>` -> `wss://<relay>` + code
/// - `http://<relay>/invite/<code>` -> `ws://<relay>` + code
/// - `buzz://join?relay=<ws(s)://relay>&code=<code>` -> relay + code
///
/// Rejects credentials, fragments, missing params, nested relay credentials, and
/// non-invite paths so scanners do not accidentally treat arbitrary URLs as
/// community admission links.
InviteDeepLink? parseInviteDeepLink(Uri uri) {
  if (uri.hasFragment || uri.userInfo.isNotEmpty) return null;

  if (uri.scheme == 'buzz') {
    if (uri.host != 'join') return null;
    final relay = uri.queryParameters['relay'];
    final code = uri.queryParameters['code'];
    if (relay == null || relay.isEmpty || code == null || code.isEmpty) {
      return null;
    }
    final relayUri = Uri.tryParse(relay);
    if (relayUri == null ||
        (relayUri.scheme != 'ws' && relayUri.scheme != 'wss') ||
        relayUri.host.isEmpty ||
        relayUri.userInfo.isNotEmpty ||
        relayUri.hasFragment) {
      return null;
    }
    final normalizedRelay = Uri(
      scheme: relayUri.scheme,
      host: relayUri.host,
      port: relayUri.hasPort ? relayUri.port : null,
    ).toString();
    final policyReceipt = uri.queryParameters['policy_receipt'];
    return InviteDeepLink(
      relayUrl: normalizedRelay,
      code: code,
      policyReceipt: policyReceipt == null || policyReceipt.isEmpty
          ? null
          : policyReceipt,
    );
  }

  if (uri.scheme == 'https' || uri.scheme == 'http') {
    if (uri.host.isEmpty) return null;
    final segments = uri.pathSegments;
    if (segments.length != 2 ||
        segments[0] != 'invite' ||
        segments[1].isEmpty) {
      return null;
    }
    final relayScheme = uri.scheme == 'https' ? 'wss' : 'ws';
    final relay = Uri(
      scheme: relayScheme,
      host: uri.host,
      port: uri.hasPort ? uri.port : null,
    ).toString();
    return InviteDeepLink(relayUrl: relay, code: segments[1]);
  }

  return null;
}

/// Parse any supported Buzz deep link.
BuzzDeepLink? parseBuzzDeepLink(Uri uri) =>
    parseInviteDeepLink(uri) ?? parseMessageDeepLink(uri);
