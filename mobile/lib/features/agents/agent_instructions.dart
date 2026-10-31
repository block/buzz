import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:uuid/uuid.dart';

import '../../shared/community/community_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/relay/relay_author_provider.dart';
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
      final relayAuthor = await ref.watch(
        relayAuthorProvider(community!.relayUrl).future,
      );
      if (relayAuthor == null) return null;
      return loadAgentInstructions(pubkey, owner, relayAuthor, query);
    });

/// Only published config is authoritative; local desktop overrides cannot be
/// inferred from legacy registration or definition prompts.
Future<String?> loadAgentInstructions(
  String pubkey,
  String owner,
  String relayAuthor,
  Future<List<NostrEvent>> Function(NostrFilter) query,
) async {
  final channels = await query(
    NostrFilter(
      kinds: const [39000],
      authors: [relayAuthor],
      tags: {
        '#P': [owner],
        '#t': [pubkey],
      },
    ),
  );
  final configs = channels.where((event) {
    final channel = ChannelData.fromEvent(event);
    return event.kind == 39000 &&
        event.pubkey == relayAuthor &&
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
  return null;
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
            onPressed: () {
              final community = ref.read(activeCommunityProvider).asData?.value;
              if (community != null) {
                ref.invalidate(relayAuthorProvider(community.relayUrl));
              }
              ref.invalidate(agentInstructionsProvider(pubkey));
            },
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
