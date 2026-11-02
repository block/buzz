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
    return parseAgentCard(json, link.cardId, cdn);
  }

  /// Reads a card's draft from its `agent.json`, validated against the card
  /// schema (v1 or v2). `v`, `id` (matching [cardId]), `name` and `avatarUrl`
  /// are required; `instructions`, `model` and `reasoningEffort` are optional
  /// and left for the user when absent. A required field that is missing, or
  /// any known field of the wrong type, makes the card invalid. Other keys are
  /// ignored. The avatar must be an https URL on [cdn], the card's own host.
  static AgentCreationDraft parseAgentCard(
    Map<String, dynamic> json,
    String cardId,
    Uri cdn,
  ) {
    const invalid = AgentCardException('This agent card is invalid.');
    final version = json['v'];
    if (version is! int || (version != 1 && version != 2)) throw invalid;
    if (json['id'] != cardId) throw invalid;
    final name = json['name'];
    final avatarUrl = json['avatarUrl'];
    if (name is! String || name.trim().isEmpty || avatarUrl is! String) {
      throw invalid;
    }
    final avatar = Uri.tryParse(avatarUrl);
    if (avatar == null ||
        avatar.scheme != 'https' ||
        avatar.host != cdn.host ||
        avatar.hasPort ||
        avatar.userInfo.isNotEmpty) {
      throw invalid;
    }
    String? optional(String key) {
      final value = json[key];
      if (value != null && value is! String) throw invalid;
      final text = (value as String?)?.trim();
      return text == null || text.isEmpty ? null : text;
    }

    final modelId = optional('model');
    final reasoningEffort = optional('reasoningEffort');
    if (reasoningEffort != null && modelId == null) throw invalid;
    return AgentCreationDraft(
      name: name.trim(),
      instructions: optional('instructions') ?? '',
      avatar: AgentAvatarSelection.url(avatarUrl),
      modelId: modelId,
      reasoningEffort: reasoningEffort,
    );
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
