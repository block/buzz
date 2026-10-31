import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:uuid/uuid.dart';

import '../../shared/community/community_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';

/// Reads only the selected owner's community-scoped agent configuration.
final agentInstructionsProvider = FutureProvider.autoDispose
    .family<String?, String>((ref, pubkey) async {
      final community = ref.watch(activeCommunityProvider).asData?.value;
      final owner = community?.pubkey;
      if (owner == null) return null;
      final relay = ref.watch(relaySessionProvider.notifier);
      Future<List<NostrEvent>> query(NostrFilter filter) =>
          relay.queryRelay([filter]);
      return loadAgentInstructions(pubkey, owner, query);
    });

/// Resolves current config before the legacy registration/definition prompt.
/// Unreadable config never falls back to a potentially stale prompt.
Future<String?> loadAgentInstructions(
  String pubkey,
  String owner,
  Future<List<NostrEvent>> Function(NostrFilter) query,
) async {
  final channels = await query(
    NostrFilter(
      kinds: const [39000],
      tags: {
        '#P': [owner],
        '#t': [pubkey],
      },
    ),
  );
  final configs = channels.where((event) {
    final channel = ChannelData.fromEvent(event);
    return event.kind == 39000 &&
        channel.creator == owner &&
        channel.channelType == 'system' &&
        channel.labels.contains('agent-config') &&
        channel.labels.contains(pubkey);
  }).toList();
  if (channels.length >= 100) {
    throw const FormatException('Too many config channels');
  }
  configs.sort((a, b) {
    final left = int.tryParse(a.getTagValue('created_at') ?? '');
    final right = int.tryParse(b.getTagValue('created_at') ?? '');
    final order = left == null
        ? (right == null ? 0 : 1)
        : (right == null ? -1 : left.compareTo(right));
    return order != 0
        ? order
        : (a.getTagValue('d') ?? '').compareTo(b.getTagValue('d') ?? '');
  });
  if (configs.isNotEmpty) {
    final channel = ChannelData.fromEvent(configs.first);
    if (channel.isArchived) return null;
    final id = const Uuid().v5(channel.id, 'buzz.agent.instructions');
    final artifacts = await query(
      NostrFilter(
        kinds: const [45010],
        extensions: const {'artifact': 'current'},
        tags: {
          '#d': [id],
          '#h': [channel.id],
        },
      ),
    );
    if (artifacts.length != 1) return null;
    final event = artifacts.single;
    if (event.kind != 45010 ||
        event.getTagValue('d') != id ||
        event.getTagValue('h') != channel.id ||
        event.getTagValue('type') != 'buzz.agent.instructions' ||
        event.getTagValue('ar') != '1' ||
        !['create', 'update', 'restore'].contains(event.getTagValue('op'))) {
      return null;
    }
    final data = jsonDecode(event.content);
    if (data is! Map ||
        data['version'] != 1 ||
        data['text'] is! String ||
        (data['text'] as String).contains('\u0000')) {
      throw const FormatException('Invalid instructions');
    }
    return data['text'] as String;
  }
  final registrations = await query(
    NostrFilter(
      kinds: const [30177],
      authors: [owner],
      tags: {
        '#d': [pubkey],
      },
    ),
  );
  final matching =
      registrations
          .where(
            (event) =>
                event.kind == 30177 &&
                event.pubkey == owner &&
                event.getTagValue('d') == pubkey,
          )
          .toList()
        ..sort(
          (a, b) => a.createdAt != b.createdAt
              ? b.createdAt.compareTo(a.createdAt)
              : a.id.compareTo(b.id),
        );
  if (matching.isEmpty) return null;
  final registration = matching.first;
  final data = jsonDecode(registration.content);
  final personaId = data is Map ? data['persona_id'] : null;
  if (personaId is String) {
    final slug = _personaDefinitionSlug(personaId);
    final definitions = await query(
      NostrFilter(
        kinds: const [30175],
        authors: [owner],
        tags: {
          '#d': [slug],
        },
      ),
    );
    final matchingDefinitions =
        definitions
            .where(
              (event) =>
                  event.kind == 30175 &&
                  event.pubkey == owner &&
                  event.getTagValue('d') == slug,
            )
            .toList()
          ..sort(
            (a, b) => a.createdAt != b.createdAt
                ? b.createdAt.compareTo(a.createdAt)
                : a.id.compareTo(b.id),
          );
    if (matchingDefinitions.isEmpty) return null;
    final definition = jsonDecode(matchingDefinitions.first.content);
    return definition is Map && definition['system_prompt'] is String
        ? definition['system_prompt'] as String
        : '';
  }
  // A linked definition that cannot be resolved must not show a legacy copy.
  if (data is! Map || data['persona_id'] != null) return null;
  return data['system_prompt'] is String
      ? data['system_prompt'] as String
      : null;
}

// Keep in sync with desktop managed_agents/persona_events.rs normalize_d_tag.
// Iterate Unicode scalars so non-ASCII characters each become one hyphen,
// matching Rust's chars(), and lowercase only ASCII before slug validation.
String _personaDefinitionSlug(String raw) {
  final slug = StringBuffer();
  for (var rune in raw.runes) {
    if (rune >= 65 && rune <= 90) rune += 32;
    final allowed =
        (rune >= 97 && rune <= 122) ||
        (rune >= 48 && rune <= 57) ||
        rune == 95 ||
        rune == 45;
    final character = allowed ? String.fromCharCode(rune) : '-';
    if (slug.isEmpty && (character == '_' || character == '-')) {
      slug.write('a');
    }
    slug.write(character);
    if (slug.length == 64) break;
  }
  return slug.isEmpty ? 'a' : slug.toString();
}

/// Read-only instructions, with explicit unavailable and retry states.
class AgentInstructions extends ConsumerWidget {
  const AgentInstructions({required this.pubkey, super.key});
  final String pubkey;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final instructions = ref.watch(agentInstructionsProvider(pubkey));
    return instructions.when(
      loading: () => const Center(
        child: BuzzLoadingIndicator(semanticLabel: 'Loading instructions'),
      ),
      error: (_, _) => Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Text('Could not load instructions.'),
          TextButton(
            onPressed: () => ref.invalidate(agentInstructionsProvider(pubkey)),
            child: const Text('Retry'),
          ),
        ],
      ),
      data: (text) => SelectableText(
        text == null
            ? 'Instructions aren’t available on mobile for this agent. View them on desktop.'
            : text.trim().isEmpty
            ? 'No instructions set.'
            : text,
        style: context.textTheme.bodyMedium,
      ),
    );
  }
}
