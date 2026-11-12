part of '../settings_page.dart';

const autoMentionAgentsStorageKey = 'buzz.messages.keepMentionedAgentsPinned';

class AutoMentionAgentsNotifier extends Notifier<bool> {
  @override
  bool build() {
    final prefs = ref.watch(savedPrefsProvider);
    return prefs.getBool(autoMentionAgentsStorageKey) ?? false;
  }

  Future<void> setEnabled(bool value) async {
    state = value;
    final prefs = ref.read(savedPrefsProvider);
    await prefs.setBool(autoMentionAgentsStorageKey, value);
  }
}

final autoMentionAgentsProvider =
    NotifierProvider<AutoMentionAgentsNotifier, bool>(
      AutoMentionAgentsNotifier.new,
    );

class _AgentsSection extends ConsumerWidget {
  const _AgentsSection({
    this.onAgents,
    this.onCreateAgent,
  });

  final void Function(BuildContext context)? onAgents;
  final void Function(BuildContext context)? onCreateAgent;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final ownedAgents = ref.watch(ownedAgentDirectoryProvider).asData?.value;
    final bestiePubkey = ref.watch(defaultAgentProvider).asData?.value;
    final agentCount = ownedAgents?.length ?? 0;
    final autoMention = ref.watch(autoMentionAgentsProvider);

    final String subtitle;
    if (bestiePubkey != null && bestiePubkey.isNotEmpty) {
      final names = watchIdentityNames(
        ref,
        [bestiePubkey],
        agentPubkeys: {bestiePubkey},
      );
      final bestieName = names.labelFor(bestiePubkey);
      subtitle = agentCount > 1
          ? 'Bestie: $bestieName · $agentCount agents'
          : 'Bestie: $bestieName';
    } else if (agentCount > 0) {
      subtitle = agentCount == 1 ? '1 agent' : '$agentCount agents';
    } else {
      subtitle = 'Manage your Bestie and custom agents';
    }

    return AppListCard(
      key: const ValueKey('settings-agents-options'),
      dividerIndent: Grid.xs,
      verticalPadding: Grid.twelve,
      children: [
        AppListRow(
          key: const ValueKey('settings-agents-row'),
          title: 'Agents',
          subtitle: subtitle,
          trailing: const _RowChevron(),
          onTap: () {
            unawaited(HapticFeedback.selectionClick());
            if (onAgents != null) {
              onAgents!(context);
            } else {
              unawaited(showAgents(context));
            }
          },
        ),
        AppListRow(
          key: const ValueKey('settings-create-agent-row'),
          title: 'Create agent',
          subtitle: 'Create a custom agent for this community',
          trailing: const _RowChevron(),
          onTap: () {
            unawaited(HapticFeedback.selectionClick());
            if (onCreateAgent != null) {
              onCreateAgent!(context);
            } else {
              unawaited(showAgents(context, createOnOpen: true));
            }
          },
        ),
        MergeSemantics(
          key: const ValueKey('settings-auto-mention-agents-setting'),
          child: Stack(
            alignment: Alignment.centerRight,
            children: [
              AppListRow(
                key: const ValueKey('settings-auto-mention-agents-row'),
                title: 'Automatically mention agents',
                subtitle: 'Address selected agents in thread replies',
                trailing: const SizedBox(width: 60),
                onTap: () {
                  unawaited(HapticFeedback.selectionClick());
                  unawaited(
                    ref
                        .read(autoMentionAgentsProvider.notifier)
                        .setEnabled(!autoMention),
                  );
                },
              ),
              Positioned.fill(
                right: Grid.xs,
                child: Align(
                  alignment: Alignment.centerRight,
                  child: Switch.adaptive(
                    key: const ValueKey('settings-auto-mention-agents-switch'),
                    value: autoMention,
                    onChanged: (value) {
                      unawaited(HapticFeedback.selectionClick());
                      unawaited(
                        ref
                            .read(autoMentionAgentsProvider.notifier)
                            .setEnabled(value),
                      );
                    },
                  ),
                ),
              ),
            ],
          ),
        ),
      ],
    );
  }
}
