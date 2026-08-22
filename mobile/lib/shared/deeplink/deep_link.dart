/// Parsing for `buzz://` deep links.
///
/// Mirrors the desktop handler in `desktop/src-tauri/src/deep_link.rs`:
/// `buzz://message?channel=<uuid>&id=<hex>[&thread=<hex>]` references a
/// message (optionally inside a thread) in a channel. Required params that
/// are missing or empty make the link invalid — the caller never sees a
/// half-formed target.
library;

import 'dart:convert';

import '../relay/relay_validation.dart';

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

/// A parsed channel-only deep link.
///
/// Canonical form: `buzz://channel/<channel-uuid>`.
class ChannelDeepLink extends BuzzDeepLink {
  /// Channel UUID from the sole path segment.
  final String channelId;

  const ChannelDeepLink({required this.channelId});

  @override
  bool operator ==(Object other) =>
      other is ChannelDeepLink && other.channelId == channelId;

  @override
  int get hashCode => channelId.hashCode;

  @override
  String toString() => 'ChannelDeepLink(channel: $channelId)';
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

/// One released track embedded in a signed release-run message link.
class ReleaseRunTrack {
  final String id;
  final String artist;
  final String title;
  final String? version;
  final String? label;
  final String releaseDate;
  final String? artworkUrl;
  final String source;
  final String? sourceUrl;
  final String? detailsUrl;

  const ReleaseRunTrack({
    required this.id,
    required this.artist,
    required this.title,
    required this.releaseDate,
    required this.source,
    this.version,
    this.label,
    this.artworkUrl,
    this.sourceUrl,
    this.detailsUrl,
  });
}

/// A bounded, self-contained release result carried by `buzz://release-run`.
class ReleaseRunDeepLink extends BuzzDeepLink {
  final String runId;
  final String runName;
  final String status;
  final int checked;
  final int released;
  final int held;
  final String sourceHealth;
  final DateTime finishedAt;
  final List<ReleaseRunTrack> tracks;

  const ReleaseRunDeepLink({
    required this.runId,
    required this.runName,
    required this.status,
    required this.checked,
    required this.released,
    required this.held,
    required this.sourceHealth,
    required this.finishedAt,
    required this.tracks,
  });
}

const _releaseRunMaxEncodedLength = 48000;
const _releaseRunMaxTracks = 50;

String? _boundedString(Object? value, int maxLength) {
  if (value is! String) return null;
  final trimmed = value.trim();
  if (trimmed.isEmpty || trimmed.length > maxLength) return null;
  return trimmed;
}

String? _optionalBoundedString(Object? value, int maxLength) {
  if (value == null || value == '') return '';
  return _boundedString(value, maxLength);
}

int? _boundedCount(Object? value) {
  if (value is! int || value < 0 || value > 1000000) return null;
  return value;
}

String? _optionalHttpsUrl(Object? value) {
  final candidate = _optionalBoundedString(value, 2000);
  if (candidate == null || candidate.isEmpty) return candidate;
  final uri = Uri.tryParse(candidate);
  if (uri == null ||
      uri.scheme != 'https' ||
      uri.host.isEmpty ||
      uri.userInfo.isNotEmpty) {
    return null;
  }
  return uri.toString();
}

ReleaseRunTrack? _parseReleaseRunTrack(Object? value) {
  if (value is! Map<String, dynamic>) return null;
  final id = _boundedString(value['id'], 160);
  final artist = _boundedString(value['artist'], 200);
  final title = _boundedString(value['title'], 240);
  final version = _optionalBoundedString(value['version'], 160);
  final label = _optionalBoundedString(value['label'], 200);
  final releaseDate = _boundedString(value['releaseDate'], 40);
  final artworkUrl = _optionalHttpsUrl(value['artworkUrl']);
  final source = _boundedString(value['source'], 100);
  final sourceUrl = _optionalHttpsUrl(value['sourceUrl']);
  final detailsUrl = _optionalHttpsUrl(value['detailsUrl']);
  if (id == null ||
      artist == null ||
      title == null ||
      version == null ||
      label == null ||
      releaseDate == null ||
      artworkUrl == null ||
      source == null ||
      sourceUrl == null ||
      detailsUrl == null) {
    return null;
  }
  return ReleaseRunTrack(
    id: id,
    artist: artist,
    title: title,
    version: version.isEmpty ? null : version,
    label: label.isEmpty ? null : label,
    releaseDate: releaseDate,
    artworkUrl: artworkUrl.isEmpty ? null : artworkUrl,
    source: source,
    sourceUrl: sourceUrl.isEmpty ? null : sourceUrl,
    detailsUrl: detailsUrl.isEmpty ? null : detailsUrl,
  );
}

/// Parse a strict, versioned `buzz://release-run?data=<base64url-json>` link.
///
/// The payload is bounded before decoding, accepts only HTTPS media/actions,
/// and requires the released count to equal the embedded track list exactly.
ReleaseRunDeepLink? parseReleaseRunDeepLink(Uri uri) {
  if (uri.scheme != 'buzz' || uri.host != 'release-run') return null;
  if ((uri.path.isNotEmpty && uri.path != '/') ||
      uri.hasFragment ||
      uri.userInfo.isNotEmpty ||
      uri.hasPort) {
    return null;
  }
  if (uri.queryParametersAll.keys.any((key) => key != 'data') ||
      uri.queryParametersAll['data']?.length != 1) {
    return null;
  }
  final data = uri.queryParameters['data'];
  if (data == null ||
      data.isEmpty ||
      data.length > _releaseRunMaxEncodedLength ||
      !RegExp(r'^[A-Za-z0-9_-]+$').hasMatch(data)) {
    return null;
  }

  Object? decoded;
  try {
    decoded = jsonDecode(
      utf8.decode(base64Url.decode(base64Url.normalize(data))),
    );
  } on FormatException {
    return null;
  }
  if (decoded is! Map<String, dynamic> || decoded['version'] != 1) return null;

  final runId = _boundedString(decoded['runId'], 120);
  final runName = _boundedString(decoded['runName'], 200);
  final status = _boundedString(decoded['status'], 80);
  final checked = _boundedCount(decoded['checked']);
  final released = _boundedCount(decoded['released']);
  final held = _boundedCount(decoded['held']);
  final sourceHealth = _boundedString(decoded['sourceHealth'], 500);
  final finishedAtText = _boundedString(decoded['finishedAt'], 80);
  final finishedAt = finishedAtText == null
      ? null
      : DateTime.tryParse(finishedAtText);
  final rawTracks = decoded['tracks'];
  if (runId == null ||
      !RegExp(r'^[A-Za-z0-9._:-]{1,120}$').hasMatch(runId) ||
      runName == null ||
      status == null ||
      checked == null ||
      released == null ||
      held == null ||
      sourceHealth == null ||
      finishedAt == null ||
      rawTracks is! List<dynamic> ||
      rawTracks.length > _releaseRunMaxTracks) {
    return null;
  }
  final tracks = rawTracks.map(_parseReleaseRunTrack).toList();
  if (tracks.any((track) => track == null) || released != tracks.length) {
    return null;
  }

  return ReleaseRunDeepLink(
    runId: runId,
    runName: runName,
    status: status,
    checked: checked,
    released: released,
    held: held,
    sourceHealth: sourceHealth,
    finishedAt: finishedAt,
    tracks: tracks.cast<ReleaseRunTrack>(),
  );
}

/// Build a canonical `buzz://message` link for a channel message.
///
/// Mirrors `desktop/src/features/messages/lib/messageLink.ts` so links copied
/// or shared from mobile round-trip through every client's parser:
/// `buzz://message?channel=<uuid>&id=<eventId>[&thread=<rootId>]`.
///
/// An empty [threadRootId] is treated as "no thread" so callers can pass
/// through a nullable thread reference without extra checks.
String buildMessageLink({
  required String channelId,
  required String messageId,
  String? threadRootId,
}) {
  if (channelId.isEmpty) {
    throw ArgumentError('buildMessageLink: channelId is required');
  }
  if (messageId.isEmpty) {
    throw ArgumentError('buildMessageLink: messageId is required');
  }

  final params = <String, String>{
    'channel': channelId,
    'id': messageId,
    if (threadRootId != null && threadRootId.isNotEmpty) 'thread': threadRootId,
  };
  return Uri(
    scheme: 'buzz',
    host: 'message',
    queryParameters: params,
  ).toString();
}

/// Parse a canonical `buzz://channel/<channel-uuid>` URI.
///
/// The channel ID must be the URI's sole non-empty path segment. Query
/// parameters and fragments are rejected so malformed or ambiguous links never
/// become navigation targets.
ChannelDeepLink? parseChannelDeepLink(Uri uri) {
  if (uri.scheme != 'buzz' || uri.host != 'channel') return null;
  if (uri.hasQuery ||
      uri.hasFragment ||
      uri.userInfo.isNotEmpty ||
      uri.hasPort) {
    return null;
  }
  if (uri.pathSegments.length != 1 || uri.pathSegments.single.isEmpty) {
    return null;
  }
  final channelId = uri.pathSegments.single;
  if (!RegExp(
    r'^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$',
    caseSensitive: false,
  ).hasMatch(channelId)) {
    return null;
  }
  return ChannelDeepLink(channelId: channelId.toLowerCase());
}

/// Parse a `buzz://message?…` URI into a [MessageDeepLink].
///
/// Returns `null` unless the URI exactly matches the canonical message-link
/// shape: no path, fragment, credentials, duplicate or unknown parameters; a
/// UUID channel; and 64-character hexadecimal message/thread event IDs.
MessageDeepLink? parseMessageDeepLink(Uri uri) {
  if (uri.scheme != 'buzz' || uri.host != 'message') return null;
  if (uri.path.isNotEmpty ||
      uri.hasFragment ||
      uri.userInfo.isNotEmpty ||
      uri.hasPort) {
    return null;
  }

  const allowedParams = {'channel', 'id', 'thread'};
  if (uri.queryParametersAll.keys.any((key) => !allowedParams.contains(key)) ||
      uri.queryParametersAll.values.any((values) => values.length != 1)) {
    return null;
  }

  final channel = uri.queryParameters['channel'];
  final id = uri.queryParameters['id'];
  final thread = uri.queryParameters['thread'];
  final uuid = RegExp(
    r'^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$',
    caseSensitive: false,
  );
  final eventId = RegExp(r'^[0-9a-f]{64}$', caseSensitive: false);
  if (channel == null ||
      !uuid.hasMatch(channel) ||
      id == null ||
      !eventId.hasMatch(id) ||
      (thread != null && !eventId.hasMatch(thread))) {
    return null;
  }

  return MessageDeepLink(
    channelId: channel.toLowerCase(),
    messageId: id.toLowerCase(),
    threadRootId: thread?.toLowerCase(),
  );
}

/// Parse canonical HTTPS invite links and `buzz://join` app handoffs.
///
/// Accepted forms:
/// - `https://<relay>/invite/<code>` -> `wss://<relay>` + code
/// - `http://localhost/invite/<code>` -> `ws://localhost` + code in debug builds
/// - `buzz://join?relay=<wss://relay>&code=<code>` -> relay + code
/// - `buzz://join?relay=<ws://localhost>&code=<code>` -> local relay in debug
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
    try {
      validateInviteRelayUri(relayUri);
    } on FormatException {
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
    final relayUri = Uri(
      scheme: relayScheme,
      host: uri.host,
      port: uri.hasPort ? uri.port : null,
    );
    try {
      validateInviteRelayUri(relayUri);
    } on FormatException {
      return null;
    }
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
    parseInviteDeepLink(uri) ??
    parseChannelDeepLink(uri) ??
    parseMessageDeepLink(uri) ??
    parseReleaseRunDeepLink(uri);

/// A validated Buzz repository, pull request, or issue permalink.
class EntityDeepLink extends BuzzDeepLink {
  final String type;
  final String owner;
  final String repository;
  final String? eventId;

  const EntityDeepLink({
    required this.type,
    required this.owner,
    required this.repository,
    this.eventId,
  });
}

/// Parse canonical `buzz://repo|pr|issue` permalinks for inline presentation.
EntityDeepLink? parseEntityDeepLink(Uri uri) {
  if (uri.scheme != 'buzz' || !{'repo', 'pr', 'issue'}.contains(uri.host)) {
    return null;
  }
  if (uri.path.isNotEmpty ||
      uri.hasFragment ||
      uri.userInfo.isNotEmpty ||
      uri.hasPort) {
    return null;
  }
  final allowed = uri.host == 'repo' ? {'owner', 'd'} : {'id', 'owner', 'd'};
  final queryParameters = uri.queryParametersAll;
  final parameterKeys = queryParameters.keys.toSet();
  if (parameterKeys.difference(allowed).isNotEmpty ||
      allowed.difference(parameterKeys).isNotEmpty ||
      allowed.any((key) => queryParameters[key]?.length != 1)) {
    return null;
  }
  final owner = uri.queryParameters['owner'];
  final repository = uri.queryParameters['d'];
  final eventId = uri.queryParameters['id'];
  final hex = RegExp(r'^[0-9a-f]{64}$', caseSensitive: false);
  final repositoryName = RegExp(r'^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$');
  if (owner == null ||
      !hex.hasMatch(owner) ||
      repository == null ||
      !repositoryName.hasMatch(repository) ||
      repository.contains('..')) {
    return null;
  }
  if (uri.host != 'repo' && (eventId == null || !hex.hasMatch(eventId))) {
    return null;
  }
  return EntityDeepLink(
    type: uri.host,
    owner: owner.toLowerCase(),
    repository: repository,
    eventId: eventId?.toLowerCase(),
  );
}
