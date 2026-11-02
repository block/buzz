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

  /// Reads a card's draft. Only `name`, `avatarUrl`, `instructions`, `model`
  /// and `reasoningEffort` are defined; cards published before instructions
  /// and model existed have just the first two. Any field may be missing or
  /// spelled in snake_case or a common synonym, and is then left for the user.
  /// An avatar is used only from [cdn], the host the card itself came from.
  static AgentCreationDraft parseAgentCard(Map<String, dynamic> json, Uri cdn) {
    final model = json['model'];
    final avatarUrl = _string(json, const [
      'avatarUrl',
      'avatar_url',
      'avatar',
      'pictureUrl',
      'picture_url',
      'picture',
    ]);
    final avatar = Uri.tryParse(avatarUrl ?? '');
    return AgentCreationDraft(
      name: _string(json, const ['name', 'displayName', 'display_name']) ?? '',
      instructions:
          _string(json, const [
            'instructions',
            'systemPrompt',
            'system_prompt',
            'prompt',
          ]) ??
          '',
      avatar:
          avatar != null &&
              avatar.scheme == 'https' &&
              avatar.host == cdn.host &&
              !avatar.hasPort &&
              avatar.userInfo.isEmpty
          ? AgentAvatarSelection.url(avatarUrl!)
          : null,
      // `model` may also be an object such as {"id": ..., "reasoningEffort": ...}.
      modelId: model is Map<String, dynamic>
          ? _string(model, const ['id', 'modelId', 'model_id', 'name'])
          : _string(json, const ['model', 'modelId', 'model_id']),
      reasoningEffort:
          _string(json, _effortKeys) ??
          (model is Map<String, dynamic> ? _string(model, _effortKeys) : null),
    );
  }

  static const _effortKeys = [
    'reasoningEffort',
    'reasoning_effort',
    'effort',
    'thinking',
  ];

  /// The first non-blank string among [keys], trimmed.
  static String? _string(Map<String, dynamic> json, List<String> keys) {
    for (final key in keys) {
      final value = json[key];
      if (value is String && value.trim().isNotEmpty) return value.trim();
    }
    return null;
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
