import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/custom_emoji/custom_emoji.dart';
import '../../shared/custom_emoji/custom_emoji_provider.dart';
import '../../shared/custom_emoji/custom_emoji_render.dart';
import '../../shared/theme/theme.dart';
import '../channels/emoji_picker.dart';
import 'user_status.dart';
import 'user_status_provider.dart';

/// The emoji well and the text field are sized to be the two things you reach
/// for, so they carry no borders — the sheet has no other controls to compete
/// with them.
const _emojiWellSize = 56.0;
const _emojiGlyphSize = 32.0;
const _saveButtonHeight = 52.0;

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
        Grid.gutter,
        0,
        Grid.gutter,
        // The sheet ends in the Save button, so it owns its own breathing room
        // above whichever is taller: the keyboard, or the home indicator.
        Grid.gutter +
            math.max(
              MediaQuery.viewInsetsOf(context).bottom,
              MediaQuery.viewPaddingOf(context).bottom,
            ),
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

          // The emoji well doubles as the picker's entry point, which is why
          // there is no separate row of emoji suggestions below.
          Row(
            children: [
              Stack(
                clipBehavior: Clip.none,
                children: [
                  Semantics(
                    button: true,
                    label: 'Choose a status emoji',
                    child: InkWell(
                      borderRadius: BorderRadius.circular(Radii.lg),
                      onTap: () => showEmojiPicker(
                        context: context,
                        onSelect: (value) => emoji.value = value,
                      ),
                      child: SizedBox.square(
                        dimension: _emojiWellSize,
                        child: Center(
                          child: _StatusEmojiPreview(emoji: emoji.value),
                        ),
                      ),
                    ),
                  ),
                  if (emoji.value.isNotEmpty)
                    Positioned(
                      top: -Grid.quarter,
                      right: -Grid.quarter,
                      child: SizedBox.square(
                        dimension: Grid.sm,
                        child: IconButton(
                          onPressed: isSaving.value
                              ? null
                              : () => emoji.value = '',
                          tooltip: 'Remove status emoji',
                          visualDensity: VisualDensity.compact,
                          style: IconButton.styleFrom(
                            backgroundColor: context.colors.surface,
                            minimumSize: const Size.square(Grid.sm),
                            maximumSize: const Size.square(Grid.sm),
                            padding: EdgeInsets.zero,
                            tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                          ),
                          icon: Icon(
                            LucideIcons.x,
                            size: 14,
                            color: context.colors.onSurface,
                          ),
                        ),
                      ),
                    ),
                ],
              ),
              const SizedBox(width: Grid.half),
              Expanded(
                child: TextField(
                  controller: textController,
                  autofocus: true,
                  style: context.textTheme.titleMedium,
                  decoration: InputDecoration(
                    hintText: 'What\u2019s your status?',
                    hintStyle: context.textTheme.titleMedium?.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                    // The theme outlines inputs; this one is the sheet's whole
                    // content, so every border state is cleared explicitly.
                    border: InputBorder.none,
                    enabledBorder: InputBorder.none,
                    focusedBorder: InputBorder.none,
                    isDense: false,
                    contentPadding: EdgeInsets.zero,
                  ),
                  textInputAction: TextInputAction.done,
                  onSubmitted: (_) {
                    if (hasContent) handleSave();
                  },
                ),
              ),
            ],
          ),
          const SizedBox(height: Grid.gutter),

          SizedBox(
            width: double.infinity,
            height: _saveButtonHeight,
            child: FilledButton(
              onPressed: hasContent && !isSaving.value ? handleSave : null,
              child: const Text('Save'),
            ),
          ),
          // No Cancel \u2014 the sheet dismisses by swiping down.
          if (hasExistingStatus)
            Padding(
              padding: const EdgeInsets.only(top: Grid.half),
              child: SizedBox(
                width: double.infinity,
                child: TextButton(
                  onPressed: isSaving.value ? null : handleClear,
                  child: const Text('Clear status'),
                ),
              ),
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
      return Icon(
        LucideIcons.smilePlus,
        size: _emojiGlyphSize,
        color: context.colors.onSurfaceVariant,
      );
    }
    final palette = ref.watch(customEmojiListProvider);
    final shortcode = normalizeShortcode(emoji);
    if (shortcode != null) {
      for (final entry in palette) {
        if (entry.shortcode == shortcode) {
          return CustomEmojiImage(
            shortcode: shortcode,
            url: entry.url,
            size: _emojiGlyphSize,
          );
        }
      }
    }
    return Text(emoji, style: const TextStyle(fontSize: _emojiGlyphSize));
  }
}
