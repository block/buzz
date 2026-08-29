import 'package:flutter/cupertino.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:intl/intl.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/custom_emoji/custom_emoji.dart';
import '../../shared/custom_emoji/custom_emoji_provider.dart';
import '../../shared/custom_emoji/custom_emoji_render.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/app_list.dart';
import '../../shared/widgets/app_list_card.dart';
import '../../shared/widgets/buzz_titled_sheet_layout.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../channels/emoji_picker.dart';
import 'user_status.dart';
import 'user_status_provider.dart';

const _emojiWellSize = 48.0;
const _emojiGlyphSize = 28.0;

const _statusPresets = [
  (text: 'In a meeting', emoji: '\u{1F5E3}\u{FE0F}'),
  (text: 'Commuting', emoji: '\u{1F68C}'),
  (text: 'Out sick', emoji: '\u{1F912}'),
  (text: 'Vacationing', emoji: '\u{1F3D6}\u{FE0F}'),
  (text: 'Working remotely', emoji: '\u{1F3E0}'),
];

enum _StatusDuration {
  oneHour('1 hour', Duration(hours: 1)),
  eightHours('8 hours', Duration(hours: 8)),
  oneDay('1 day', Duration(days: 1)),
  oneWeek('1 week', Duration(days: 7)),
  custom('Custom', null);

  const _StatusDuration(this.label, this.duration);

  final String label;
  final Duration? duration;
}

void showSetStatusSheet(BuildContext context, {UserStatus? currentStatus}) {
  showBuzzModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showCloseButton: false,
    showDragHandle: false,
    builder: (_) => _SetStatusSheet(currentStatus: currentStatus),
  );
}

class _SetStatusSheet extends HookConsumerWidget {
  const _SetStatusSheet({this.currentStatus});

  final UserStatus? currentStatus;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final textController = useTextEditingController(
      text: currentStatus?.text ?? '',
    );
    final emoji = useState(currentStatus?.emoji ?? '');
    final text = useState(currentStatus?.text ?? '');
    final currentExpiration = currentStatus?.expirationDateTime;
    final duration = useState(
      currentExpiration == null
          ? _StatusDuration.oneDay
          : _StatusDuration.custom,
    );
    final customUntil = useState(
      currentExpiration ?? DateTime.now().add(_StatusDuration.oneDay.duration!),
    );
    final isSaving = useState(false);

    useEffect(() {
      void listener() => text.value = textController.text;
      textController.addListener(listener);
      return () => textController.removeListener(listener);
    }, [textController]);

    final hasContent = text.value.trim().isNotEmpty || emoji.value.isNotEmpty;
    final hasExistingStatus = currentStatus != null && !currentStatus!.isEmpty;

    DateTime expiresAt() => switch (duration.value) {
      _StatusDuration.custom => customUntil.value,
      final preset => DateTime.now().add(preset.duration!),
    };

    Future<void> handleSave() async {
      if (isSaving.value || !hasContent) return;
      isSaving.value = true;
      try {
        await ref
            .read(userStatusProvider.notifier)
            .setStatus(text.value, emoji.value, expiresAt: expiresAt());
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

    Future<void> pickCustomUntil() async {
      final picked = await _showNativeDateTimePicker(
        context,
        initial: customUntil.value,
      );
      if (picked == null) return;
      duration.value = _StatusDuration.custom;
      customUntil.value = picked;
    }

    Future<void> chooseDuration() async {
      FocusScope.of(context).unfocus();
      final selected = await _showStatusDurationSheet(
        context,
        selected: duration.value,
      );
      if (selected == null || !context.mounted) return;
      if (selected == _StatusDuration.custom) {
        await pickCustomUntil();
      } else {
        duration.value = selected;
      }
    }

    final headerAction = SizedBox.square(
      dimension: 44,
      child: IconButton(
        key: const ValueKey('save-status-button'),
        tooltip: 'Save status',
        onPressed: hasContent && !isSaving.value ? handleSave : null,
        style: IconButton.styleFrom(
          padding: EdgeInsets.zero,
          backgroundColor: context.colors.surfaceContainerHighest,
          foregroundColor: context.colors.onSurface,
          disabledForegroundColor: context.colors.onSurfaceVariant.withValues(
            alpha: 0.38,
          ),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(Radii.dialog),
          ),
        ),
        icon: const Icon(LucideIcons.check, size: 22),
      ),
    );

    return SafeArea(
      top: false,
      child: BuzzTitledSheetLayout(
        title: 'Set a status',
        showDragHandle: true,
        leading: headerAction,
        child: SingleChildScrollView(
          padding: EdgeInsets.only(
            // Keep the sheet at its resting height while the keyboard
            // overlays its lower portion. Adding viewInsets here makes the
            // modal itself grow upward by the full keyboard height.
            bottom: Grid.gutter + MediaQuery.viewPaddingOf(context).bottom,
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Padding(
                padding: const EdgeInsets.fromLTRB(
                  Grid.gutter,
                  0,
                  Grid.gutter,
                  Grid.xs,
                ),
                child: _StatusInput(
                  controller: textController,
                  emoji: emoji.value,
                  enabled: !isSaving.value,
                  onChooseEmoji: () => showEmojiPicker(
                    context: context,
                    onSelect: (value) => emoji.value = value,
                  ),
                  onRemoveEmoji: () => emoji.value = '',
                  onSubmitted: handleSave,
                ),
              ),
              AppListCard(
                label: 'Duration',
                children: [
                  AppListRow(
                    icon: LucideIcons.clock3,
                    title: 'Duration',
                    value: duration.value.label,
                    trailing: const Icon(LucideIcons.chevronDown, size: 18),
                    onTap: chooseDuration,
                  ),
                  if (duration.value == _StatusDuration.custom)
                    AppListRow(
                      icon: LucideIcons.calendarClock,
                      title: 'Until',
                      value: _formatUntil(customUntil.value),
                      onTap: pickCustomUntil,
                    ),
                ],
              ),
              AppListCard(
                label: 'Quick statuses',
                children: [
                  for (final preset in _statusPresets)
                    AppListRowRaw(
                      leading: SizedBox(
                        width: 22,
                        child: Center(
                          child: Text(
                            preset.emoji,
                            style: const TextStyle(fontSize: 20),
                          ),
                        ),
                      ),
                      title: Text(
                        preset.text,
                        style: context.textTheme.bodyLarge,
                      ),
                      onTap: () {
                        textController.text = preset.text;
                        emoji.value = preset.emoji;
                      },
                    ),
                ],
              ),
              if (hasExistingStatus)
                AppListCard(
                  children: [
                    AppListRow(
                      icon: LucideIcons.trash2,
                      title: 'Clear status',
                      titleColor: context.colors.error,
                      onTap: isSaving.value ? null : handleClear,
                    ),
                  ],
                ),
            ],
          ),
        ),
      ),
    );
  }
}

class _StatusInput extends ConsumerWidget {
  const _StatusInput({
    required this.controller,
    required this.emoji,
    required this.enabled,
    required this.onChooseEmoji,
    required this.onRemoveEmoji,
    required this.onSubmitted,
  });

  final TextEditingController controller;
  final String emoji;
  final bool enabled;
  final VoidCallback onChooseEmoji;
  final VoidCallback onRemoveEmoji;
  final VoidCallback onSubmitted;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return DecoratedBox(
      key: const ValueKey('status-input-outline'),
      decoration: BoxDecoration(
        border: Border.all(
          color: context.colors.outlineVariant.withValues(alpha: 0.8),
        ),
        borderRadius: BorderRadius.circular(Radii.container),
      ),
      child: Row(
        children: [
          SizedBox.square(
            dimension: _emojiWellSize,
            child: Stack(
              clipBehavior: Clip.none,
              children: [
                Semantics(
                  button: true,
                  label: 'Choose a status emoji',
                  child: InkWell(
                    borderRadius: BorderRadius.circular(Radii.container),
                    onTap: enabled ? onChooseEmoji : null,
                    child: Center(child: _StatusEmojiPreview(emoji: emoji)),
                  ),
                ),
                if (emoji.isNotEmpty)
                  Positioned(
                    top: -Grid.half,
                    right: -Grid.half,
                    child: SizedBox.square(
                      dimension: Grid.sm,
                      child: IconButton(
                        onPressed: enabled ? onRemoveEmoji : null,
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
          ),
          Expanded(
            child: TextField(
              controller: controller,
              enabled: enabled,
              autofocus: false,
              style: context.textTheme.bodyLarge,
              decoration: InputDecoration(
                hintText: 'What\u2019s your status?',
                hintStyle: context.textTheme.bodyLarge?.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
                border: InputBorder.none,
                enabledBorder: InputBorder.none,
                focusedBorder: InputBorder.none,
                isDense: true,
                contentPadding: const EdgeInsets.only(
                  left: Grid.xxs,
                  right: Grid.xs,
                ),
              ),
              textInputAction: TextInputAction.done,
              onSubmitted: (_) => onSubmitted(),
            ),
          ),
        ],
      ),
    );
  }
}

class _StatusEmojiPreview extends ConsumerWidget {
  const _StatusEmojiPreview({required this.emoji});

  final String emoji;

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

Future<_StatusDuration?> _showStatusDurationSheet(
  BuildContext context, {
  required _StatusDuration selected,
}) {
  return showBuzzModalBottomSheet<_StatusDuration>(
    context: context,
    title: 'Duration',
    isScrollControlled: true,
    showDragHandle: true,
    builder: (sheetContext) => SingleChildScrollView(
      child: SafeArea(
        top: false,
        child: AppListCard(
          children: [
            for (final option in _StatusDuration.values)
              AppListRow(
                title: option.label,
                trailing: option == selected
                    ? const Icon(LucideIcons.check, size: 18)
                    : null,
                onTap: () => Navigator.of(sheetContext).pop(option),
              ),
          ],
        ),
      ),
    ),
  );
}

Future<DateTime?> _showNativeDateTimePicker(
  BuildContext context, {
  required DateTime initial,
}) async {
  final now = DateTime.now();
  final minimum = now.add(const Duration(minutes: 5));
  final safeInitial = initial.isAfter(minimum) ? initial : minimum;

  if (defaultTargetPlatform == TargetPlatform.iOS) {
    var selected = safeInitial;
    return showCupertinoModalPopup<DateTime>(
      context: context,
      builder: (pickerContext) => Material(
        color: context.colors.surface,
        child: SafeArea(
          top: false,
          child: SizedBox(
            height: 300,
            child: Column(
              children: [
                Align(
                  alignment: Alignment.centerRight,
                  child: TextButton(
                    onPressed: () => Navigator.of(pickerContext).pop(selected),
                    child: const Text('Done'),
                  ),
                ),
                Expanded(
                  child: CupertinoDatePicker(
                    mode: CupertinoDatePickerMode.dateAndTime,
                    minimumDate: minimum,
                    initialDateTime: safeInitial,
                    use24hFormat: MediaQuery.alwaysUse24HourFormatOf(context),
                    onDateTimeChanged: (value) => selected = value,
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  final lastDate = now.add(const Duration(days: 365));
  final androidInitial = safeInitial.isAfter(lastDate) ? lastDate : safeInitial;
  final date = await showDatePicker(
    context: context,
    initialDate: androidInitial,
    firstDate: now,
    lastDate: lastDate,
  );
  if (date == null || !context.mounted) return null;
  final time = await showTimePicker(
    context: context,
    initialTime: TimeOfDay.fromDateTime(androidInitial),
  );
  if (time == null) return null;
  final selected = DateTime(
    date.year,
    date.month,
    date.day,
    time.hour,
    time.minute,
  );
  return selected.isBefore(minimum) ? minimum : selected;
}

String _formatUntil(DateTime value) =>
    DateFormat('EEE, MMM d \u00B7 h:mm a').format(value);
