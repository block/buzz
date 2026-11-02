import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;

import '../../shared/deeplink/deep_link.dart';
import '../../shared/hosted/hosted_environment.dart';
import 'agent_avatar_picker.dart';

/// Creates the HTTP client that fetches a card's `agent.json`; overridden in
/// tests.
final agentCardClientProvider = Provider<http.Client Function()>(
  (ref) => http.Client.new,
);

/// Largest `agent.json` accepted, far above any real card.
const maxAgentCardBytes = 256 * 1024;

const _fetchTimeout = Duration(seconds: 15);

/// Why a card's draft could not be loaded, worded for the user.
class AgentCardException implements Exception {
  const AgentCardException(this.message);

  final String message;

  @override
  String toString() => 'AgentCardException: $message';
}

/// Initial, fully editable values for [CreateAgentPage], from a link.
///
/// Unlike an unfinished setup in `RemoteAgentController.pending`, a draft has
/// created nothing: every field stays editable and nothing is persisted.
class AgentCreationDraft {
  const AgentCreationDraft({
    required this.name,
    required this.instructions,
    this.avatar,
    this.modelId,
    this.reasoningEffort,
  });

  final String name;
  final String instructions;

  /// Card avatar, or null to let the form pick its usual default.
  final AgentAvatarSelection? avatar;

  /// Requested model, applied only if the live catalog lists it.
  final String? modelId;

  /// Requested effort, applied only if the chosen model supports it.
  final String? reasoningEffort;

  /// Fetches [link]'s card from this build's avatars CDN and reads its draft.
  static Future<AgentCreationDraft> fetch(
    AgentCreationDeepLink link,
    http.Client client,
  ) async {
    final cdn = HostedEnvironment.avatarsCdnOrigin;
    final url = cdn.replace(path: '/${link.cardId}/agent.json');
    final http.StreamedResponse response;
    final List<int> body;
    try {
      response = await client
          .send(
            http.Request('GET', url)
              ..headers['accept'] = 'application/json'
              // A redirect could lead off the CDN; treat it as unavailable.
              ..followRedirects = false,
          )
          .timeout(_fetchTimeout);
      if (response.statusCode == 403 || response.statusCode == 404) {
        throw const AgentCardException('This agent card isn’t available.');
      }
      if (response.statusCode != 200) {
        throw AgentCardException(
          'Couldn’t load this agent card (HTTP ${response.statusCode}).',
        );
      }
      if ((response.contentLength ?? 0) > maxAgentCardBytes) {
        throw const AgentCardException('This agent card is too large.');
      }
      body = await _readBounded(response.stream).timeout(_fetchTimeout);
    } on AgentCardException {
      rethrow;
    } on Exception {
      throw const AgentCardException(
        'Couldn’t load this agent card. Check your connection and reopen the '
        'link.',
      );
    }
    final Object? json;
    try {
      json = jsonDecode(utf8.decode(body));
    } on FormatException {
      throw const AgentCardException('This agent card is invalid.');
    }
    if (json is! Map<String, dynamic>) {
      throw const AgentCardException('This agent card is invalid.');
    }
    return parseAgentCard(json, cdn);
  }

  /// Reads a card's draft from the `agent.json` schema's `name`,
  /// `avatarUrl`, `instructions`, `model` and `reasoningEffort`. Cards
  /// published before instructions and model existed have just the first two.
  /// A missing field, or one that isn't a non-blank string, is left for the
  /// user; other keys are ignored. An avatar is used only from [cdn], the host
  /// the card itself came from.
  static AgentCreationDraft parseAgentCard(Map<String, dynamic> json, Uri cdn) {
    final avatarUrl = _string(json, 'avatarUrl');
    final avatar = Uri.tryParse(avatarUrl ?? '');
    return AgentCreationDraft(
      name: _string(json, 'name') ?? '',
      instructions: _string(json, 'instructions') ?? '',
      avatar:
          avatar != null &&
              avatar.scheme == 'https' &&
              avatar.host == cdn.host &&
              !avatar.hasPort &&
              avatar.userInfo.isEmpty
          ? AgentAvatarSelection.url(avatarUrl!)
          : null,
      modelId: _string(json, 'model'),
      reasoningEffort: _string(json, 'reasoningEffort'),
    );
  }

  /// [key]'s value if it is a non-blank string, trimmed.
  static String? _string(Map<String, dynamic> json, String key) {
    final value = json[key];
    return value is String && value.trim().isNotEmpty ? value.trim() : null;
  }

  static Future<List<int>> _readBounded(Stream<List<int>> stream) async {
    final bytes = BytesBuilder(copy: false);
    await for (final chunk in stream) {
      if (bytes.length + chunk.length > maxAgentCardBytes) {
        throw const AgentCardException('This agent card is too large.');
      }
      bytes.add(chunk);
    }
    return bytes.takeBytes();
  }
}
