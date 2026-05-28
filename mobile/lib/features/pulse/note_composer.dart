import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../profile/profile_provider.dart';
import 'pulse_actions.dart';
import 'pulse_models.dart';

class NoteComposer extends HookConsumerWidget {
  final UserNote? replyTo;
  final VoidCallback? onSent;
  final String hintText;

  const NoteComposer({
    super.key,
    this.replyTo,
    this.onSent,
    this.hintText = 'What’s on your mind?',
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = useTextEditingController();
    final isSending = useState(false);
    final hasText = useListenableSelector(
      controller,
      () => controller.text.trim().isNotEmpty,
    );
    final profile = ref.watch(profileProvider).asData?.value;

    return Container(
      padding: const EdgeInsets.all(Grid.xs),
      decoration: BoxDecoration(
        color: context.colors.surfaceContainerHighest.withValues(alpha: 0.82),
        borderRadius: BorderRadius.circular(Radii.lg),
        border: Border.all(
          color: context.colors.outlineVariant.withValues(alpha: 0.55),
        ),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          CircleAvatar(
            radius: 16,
            backgroundColor: context.colors.primaryContainer,
            backgroundImage: profile?.avatarUrl != null
                ? NetworkImage(profile!.avatarUrl!)
                : null,
            child: profile?.avatarUrl == null
                ? Text(
                    profile?.initial ?? '?',
                    style: context.textTheme.labelMedium?.copyWith(
                      color: context.colors.onPrimaryContainer,
                    ),
                  )
                : null,
          ),
          const SizedBox(width: Grid.xs),
          Expanded(
            child: TextField(
              controller: controller,
              minLines: 1,
              maxLines: 5,
              textInputAction: TextInputAction.newline,
              decoration: InputDecoration(
                hintText: hintText,
                border: InputBorder.none,
                enabledBorder: InputBorder.none,
                focusedBorder: InputBorder.none,
                isDense: true,
                contentPadding: const EdgeInsets.symmetric(vertical: Grid.half),
              ),
            ),
          ),
          const SizedBox(width: Grid.half),
          SizedBox(
            width: 38,
            height: 38,
            child: FilledButton(
              onPressed: hasText && !isSending.value
                  ? () async {
                      isSending.value = true;
                      try {
                        await publishNote(
                          ref,
                          content: controller.text,
                          replyTo: replyTo,
                        );
                        controller.clear();
                        onSent?.call();
                      } finally {
                        isSending.value = false;
                      }
                    }
                  : null,
              style: FilledButton.styleFrom(padding: EdgeInsets.zero),
              child: isSending.value
                  ? const SizedBox(
                      width: 16,
                      height: 16,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    )
                  : const Icon(LucideIcons.send, size: 16),
            ),
          ),
        ],
      ),
    );
  }
}
