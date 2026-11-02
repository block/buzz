import 'dart:convert';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;

import '../../shared/deeplink/deep_link.dart';
import '../../shared/hosted/hosted_environment.dart';
import '../../shared/utils/bounded_fetch.dart';
import 'agent_avatar_picker.dart';

/// Creates the HTTP client that fetches a card's `agent.json`; overridden in
/// tests.
final agentCardClientProvider = Provider<http.Client Function()>(
  (ref) => http.Client.new,
);

/// Largest `agent.json` accepted, far above any real card.
const maxAgentCardBytes = 2 * 1024 * 1024;

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
    // The id is opaque; pathSegments percent-encodes anything path-unsafe.
    final url = cdn.replace(pathSegments: [link.cardId, 'agent.json']);
    final BoundedResponse response;
    try {
      response = await boundedGet(
        client,
        url,
        maxBytes: maxAgentCardBytes,
        headers: const {'accept': 'application/json'},
        // A redirect could lead off the CDN; treat it as unavailable.
        followRedirects: false,
      );
    } on ResponseTooLargeException {
      throw const AgentCardException('This agent card is too large.');
    } on Exception {
      throw const AgentCardException(
        'Couldn’t load this agent card. Check your connection and reopen the '
        'link.',
      );
    }
    if (response.statusCode == 403 || response.statusCode == 404) {
      throw const AgentCardException('This agent card isn’t available.');
    }
    if (response.statusCode != 200) {
      throw AgentCardException(
        'Couldn’t load this agent card (HTTP ${response.statusCode}).',
      );
    }
    final Object? json;
    try {
      json = jsonDecode(utf8.decode(response.bytes));
    } on FormatException {
      throw const AgentCardException('This agent card is invalid.');
    }
    if (json is! Map<String, dynamic>) {
      throw const AgentCardException('This agent card is invalid.');
    }
    return parseAgentCard(json, link.cardId);
  }

  /// Reads a card's draft from its `agent.json`. `id` (matching [cardId]),
  /// `name` and `avatarUrl` (an https URL) are required; `instructions`,
  /// `model` and `reasoningEffort` are optional and left for the user when
  /// absent or blank. A missing required field, or a known field of the wrong
  /// type, makes the card invalid. Other keys, including `v`, are ignored.
  static AgentCreationDraft parseAgentCard(
    Map<String, dynamic> json,
    String cardId,
  ) {
    const invalid = AgentCardException('This agent card is invalid.');
    if (json['id'] != cardId) throw invalid;
    final name = json['name'];
    final avatarUrl = json['avatarUrl'];
    if (name is! String || name.trim().isEmpty || avatarUrl is! String) {
      throw invalid;
    }
    final avatar = Uri.tryParse(avatarUrl);
    if (avatar == null || avatar.scheme != 'https' || avatar.host.isEmpty) {
      throw invalid;
    }
    String? optional(String key) {
      final value = json[key];
      if (value != null && value is! String) throw invalid;
      final text = (value as String?)?.trim();
      return text == null || text.isEmpty ? null : text;
    }

    return AgentCreationDraft(
      name: name.trim(),
      instructions: optional('instructions') ?? '',
      avatar: AgentAvatarSelection.url(avatarUrl),
      modelId: optional('model'),
      reasoningEffort: optional('reasoningEffort'),
    );
  }
}
