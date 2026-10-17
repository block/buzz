import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/hosted/hosted_flow.dart';
import '../../shared/theme/grid.dart';

/// Public community choices only; identity material remains in secure storage.
class HostedCommunitySelection extends HookConsumerWidget {
  const HostedCommunitySelection({super.key, required this.flow});
  final HostedFlow flow;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useListenable(flow);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final community in flow.communities)
          CheckboxListTile(
            key: ValueKey(community.relayUrl),
            title: Text(community.name),
            subtitle: Text(Uri.parse(community.relayUrl).host),
            value: flow.selectedRelays.contains(community.relayUrl),
            onChanged: (value) =>
                flow.selectCommunity(community.relayUrl, value ?? false),
          ),
        const SizedBox(height: Grid.sm),
        FilledButton(
          onPressed: flow.selectedRelays.isEmpty
              ? null
              : flow.addSelectedCommunities,
          child: const Text('Continue'),
        ),
      ],
    );
  }
}
