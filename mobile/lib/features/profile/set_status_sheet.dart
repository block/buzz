import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/theme/theme.dart';
import '../custom_emoji/custom_emoji.dart';
import '../custom_emoji/custom_emoji_provider.dart';
import '../custom_emoji/custom_emoji_render.dart';
import '../channels/emoji_picker.dart';
import 'user_status.dart';
import 'user_status_provider.dart';

const _emojiOptions = [
  (emoji: '\u{1F5E3}\u{FE0F}', label: 'In a meeting'),
  (emoji: '\u{1F68C}', label: 'Commuting'),
  (emoji: '\u{1F912}', label: 'Out sick'),
  (emoji: '\u{1F3D6}\u{FE0F}', label: 'Vacationing'),
  (emoji: '\u{1F3E0}', label: 'Working remotely'),
  (emoji: '\u{1F354}', label: 'Lunch'),
  (emoji: '\u{1F3AF}', label: 'Focus'),
  (emoji: '\u{1F4AA}', label: 'Exercising'),
];

const _presets = [
  (text: 'In a meeting', emoji: '\u{1F5E3}\u{FE0F}'),
  (text: 'Commuting', emoji: '\u{1F68C}'),
  (text: 'Out sick', emoji: '\u{1F912}'),
  (text: 'Vacationing', emoji: '\u{1F3D6}\u{FE0F}'),
  (text: 'Working remotely', emoji: '\u{1F3E0}'),
];

void showSetStatusSheet(BuildContext context, {UserStatus? currentStatus}) {
  showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    builder: (_) => _SetStatusSheet(currentStatus: currentStatus),
  );
}

class _SetStatusSheet extends HookConsumerWidget {
  final UserStatus? currentStatus;

  const _SetStatusSheet({this.currentStatus});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final textController = useTextEditingController(
      text: currentStatus?.text ?? '',
    );
    final emoji = useState(currentStatus?.emoji ?? '');
    final text = useState(currentStatus?.text ?? '');
    final isSaving = useState(false);
    final customEmoji = ref.watch(customEmojiListProvider);

    useEffect(() {
      void listener() => text.value = textController.text;
      textController.addListener(listener);
      return () => textController.removeListener(listener);
    }, [textController]);

    final hasContent = text.value.trim().isNotEmpty || emoji.value.isNotEmpty;
    final hasExistingStatus = currentStatus != null && !currentStatus!.isEmpty;

    Future<void> handleSave() async {
      if (isSaving.value) return;
      isSaving.value = true;
      try {
        await ref
            .read(userStatusProvider.notifier)
            .setStatus(text.value, emoji.value);
        if (context.mounted) Navigator.of(context).pop();
      } finally {
        isSaving.value = false;
      }
    }

    Future<void> handleClear() async {
      if (isSaving.value) return;
      isSaving.value = true;
      try {
        await ref.read(userStatusProvider.notifier).clearStatus();
        if (context.mounted) Navigator.of(context).pop();
      } finally {
        isSaving.value = false;
      }
    }

    return Padding(
      padding: EdgeInsets.fromLTRB(
        Grid.xs,
        0,
        Grid.xs,
        MediaQuery.viewInsetsOf(context).bottom,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('Set a status', style: context.textTheme.titleMedium),
          const SizedBox(height: Grid.half),
          Text(
            'Let others know what you\u2019re up to.',
            style: context.textTheme.bodySmall?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
          const SizedBox(height: Grid.twelve),

          // Text input with emoji preview
          Row(
            children: [
              Container(
                width: 40,
                height: 40,
                alignment: Alignment.center,
                decoration: BoxDecoration(
                  border: Border.all(color: context.colors.outlineVariant),
                  borderRadius: BorderRadius.circular(Radii.md),
                ),
                child: _StatusEmojiPreview(emoji: emoji.value),
              ),
              const SizedBox(width: Grid.xxs),
              Expanded(
                child: TextField(
                  controller: textController,
                  autofocus: true,
                  decoration: const InputDecoration(
                    hintText: 'What\u2019s your status?',
                    border: OutlineInputBorder(),
                    contentPadding: EdgeInsets.symmetric(
                      horizontal: Grid.twelve,
                      vertical: Grid.xxs,
                    ),
                  ),
                  textInputAction: TextInputAction.done,
                  onSubmitted: (_) {
                    if (hasContent) handleSave();
                  },
                ),
              ),
            ],
          ),
          const SizedBox(height: Grid.twelve),

          Wrap(
            spacing: Grid.half,
            runSpacing: Grid.half,
            children: [
              for (final option in _emojiOptions)
                _EmojiButton(
                  emoji: option.emoji,
                  label: option.label,
                  selected: emoji.value == option.emoji,
                  onTap: () {
                    emoji.value = emoji.value == option.emoji
                        ? ''
                        : option.emoji;
                  },
                ),
              if (customEmoji.isNotEmpty)
                _PickCustomEmojiButton(
                  selected: emoji.value.startsWith(':'),
                  onTap: () => showEmojiPicker(
                    context: context,
                    onSelect: (value) => emoji.value = value,
                  ),
                ),
            ],
          ),
          const SizedBox(height: Grid.twelve),

          // Presets
          Wrap(
            spacing: Grid.half,
            runSpacing: Grid.half,
            children: [
              for (final preset in _presets)
                ActionChip(
                  label: Text('${preset.emoji} ${preset.text}'),
                  labelStyle: context.textTheme.labelSmall,
                  onPressed: () {
                    textController.text = preset.text;
                    emoji.value = preset.emoji;
                  },
                ),
            ],
          ),
          const SizedBox(height: Grid.xs),

          Row(
            children: [
              if (hasExistingStatus)
                TextButton(
                  onPressed: isSaving.value ? null : handleClear,
                  child: const Text('Clear status'),
                ),
              const Spacer(),
              TextButton(
                onPressed: isSaving.value
                    ? null
                    : () => Navigator.of(context).pop(),
                child: const Text('Cancel'),
              ),
              const SizedBox(width: Grid.xxs),
              FilledButton(
                onPressed: hasContent && !isSaving.value ? handleSave : null,
                child: const Text('Save'),
              ),
            ],
          ),
        ],
      ),
    );
  }
}

class _StatusEmojiPreview extends ConsumerWidget {
  final String emoji;

  const _StatusEmojiPreview({required this.emoji});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    if (emoji.isEmpty) {
      return const Text('\u{1F4AC}', style: TextStyle(fontSize: 18));
    }
    final palette = ref.watch(customEmojiListProvider);
    final shortcode = normalizeShortcode(emoji);
    if (shortcode != null) {
      for (final entry in palette) {
        if (entry.shortcode == shortcode) {
          return CustomEmojiImage(
            shortcode: shortcode,
            url: entry.url,
            size: 22,
          );
        }
      }
    }
    return Text(emoji, style: const TextStyle(fontSize: 18));
  }
}

class _PickCustomEmojiButton extends StatelessWidget {
  final bool selected;
  final VoidCallback onTap;

  const _PickCustomEmojiButton({required this.selected, required this.onTap});

  @override
  Widget build(BuildContext context) {
    return Tooltip(
      message: 'Custom emoji',
      child: InkWell(
        borderRadius: BorderRadius.circular(Radii.md),
        onTap: onTap,
        child: Container(
          width: 36,
          height: 36,
          alignment: Alignment.center,
          decoration: BoxDecoration(
            color: selected
                ? context.colors.secondaryContainer
                : Colors.transparent,
            borderRadius: BorderRadius.circular(Radii.md),
            border: selected ? Border.all(color: context.colors.outline) : null,
          ),
          child: Icon(
            Icons.add_reaction_outlined,
            size: 18,
            color: context.colors.onSurfaceVariant,
          ),
        ),
      ),
    );
  }
}

class _EmojiButton extends StatelessWidget {
  final String emoji;
  final String label;
  final bool selected;
  final VoidCallback onTap;

  const _EmojiButton({
    required this.emoji,
    required this.label,
    required this.selected,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    return Tooltip(
      message: label,
      child: InkWell(
        borderRadius: BorderRadius.circular(Radii.md),
        onTap: onTap,
        child: Container(
          width: 36,
          height: 36,
          alignment: Alignment.center,
          decoration: BoxDecoration(
            color: selected
                ? context.colors.secondaryContainer
                : Colors.transparent,
            borderRadius: BorderRadius.circular(Radii.md),
            border: selected ? Border.all(color: context.colors.outline) : null,
          ),
          child: Text(emoji, style: const TextStyle(fontSize: 18)),
        ),
      ),
    );
  }
}
