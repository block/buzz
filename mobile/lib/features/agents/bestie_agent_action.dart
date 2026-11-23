import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/me/default_agent.dart';
import '../../shared/me/default_agent_writer.dart';
import '../../shared/me/me_sync.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/profile_action_chip.dart';

/// Saves the selected owned agent using the shared desktop/mobile preference.
class BestieAgentAction extends HookConsumerWidget {
  const BestieAgentAction({
    required this.pubkey,
    this.compact = false,
    super.key,
  });

  final String pubkey;
  final bool compact;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final sync = ref.watch(meSyncProvider);
    final selected = ref.watch(defaultAgentProvider).asData?.value == pubkey;
    final saving = useState(false);
    final error = useState<String?>(null);

    Future<void> choose() async {
      if (saving.value) return;
      saving.value = true;
      error.value = null;
      try {
        await saveDefaultAgent(sync, pubkey);
        if (!context.mounted || ref.read(meSyncProvider) != sync) return;
        ref.invalidate(defaultAgentProvider);
      } catch (failure) {
        if (context.mounted && ref.read(meSyncProvider) == sync) {
          error.value = 'Could not save Bestie. $failure';
        }
      } finally {
        if (context.mounted) saving.value = false;
      }
    }

    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: compact
          ? CrossAxisAlignment.start
          : CrossAxisAlignment.stretch,
      children: [
        if (compact)
          ProfileActionChip(
            label: selected ? 'Your Bestie' : 'Make Bestie',
            selected: selected,
            onPressed: saving.value || selected ? null : choose,
            leading: saving.value
                ? const BuzzLoadingIndicator(size: 14)
                : selected
                ? const Icon(Icons.star, size: 14)
                : null,
          )
        else
          Semantics(
            selected: selected,
            child: FilledButton.icon(
              onPressed: saving.value || selected ? null : choose,
              style: FilledButton.styleFrom(
                backgroundColor: context.colors.surfaceContainerHighest,
                foregroundColor: context.colors.onSurface,
                disabledBackgroundColor: context.colors.surfaceContainerHighest,
                disabledForegroundColor: context.colors.onSurface,
                minimumSize: Size(
                  compact ? 0 : double.infinity,
                  compact ? 48 : Grid.xxl,
                ),
                shape: RoundedRectangleBorder(
                  borderRadius: BorderRadius.circular(
                    compact ? Radii.full : Radii.dialog,
                  ),
                ),
              ),
              icon: saving.value
                  ? const BuzzLoadingIndicator(size: 18)
                  : selected
                  ? const Icon(Icons.star, size: 22)
                  : null,
              label: Text(selected ? 'Your Bestie' : 'Make Bestie'),
            ),
          ),
        if (error.value != null)
          Semantics(
            liveRegion: true,
            child: Text(
              error.value!,
              style: TextStyle(color: context.colors.error),
            ),
          ),
      ],
    );
  }
}
