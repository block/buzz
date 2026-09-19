/// The "New task" bottom sheet.
///
/// Presentation follows `remind_me_later_sheet` (top-level `show…Sheet`
/// function, messenger captured before the first await, snackbar confirmation)
/// and its form follows `manage_channel_sheet` (derived-boolean validation,
/// inline error text, disabled submit with an in-button "Creating…" label).
/// Neither pattern uses `Form`/`TextFormField`, and neither does this.
library;

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../mentions/agent_identity_provider.dart';
import '../profile/user_cache_provider.dart';
import '../theme/theme.dart';
import '../widgets/app_list_card.dart';
import '../widgets/app_list.dart';
import '../widgets/modal_presentation.dart';
import 'task.dart';
import 'task_due_presets.dart';
import 'tasks_api.dart';

/// Title length past which the remaining-character count becomes useful.
const _titleCounterThreshold = 160;

/// Opens the "New task" sheet for [channelId] and reports the outcome.
///
/// [channelName] labels the channel-scope row. [sourceEventId] is the message
/// or thread head the task was opened from; it travels as the task's
/// `source_ref` so the task records where it came from.
Future<void> showCreateTaskSheet({
  required BuildContext context,
  required WidgetRef ref,
  required String channelId,
  String channelName = '',
  String? sourceEventId,
}) async {
  // Resolved before the sheet is shown so the confirmation survives its pop.
  final messenger = ScaffoldMessenger.of(context);
  if (!ref.read(tasksApiProvider).canSign) {
    messenger.showSnackBar(
      const SnackBar(content: Text('Sign in to create tasks')),
    );
    return;
  }

  final created = await showBuzzModalBottomSheet<Task>(
    context: context,
    title: 'New task',
    isScrollControlled: true,
    showDragHandle: true,
    constraints: BoxConstraints(
      maxWidth: 640,
      maxHeight: MediaQuery.sizeOf(context).height * 0.9,
    ),
    builder: (_) => _CreateTaskSheet(
      channelId: channelId,
      channelName: channelName,
      sourceEventId: sourceEventId,
    ),
  );

  if (created != null) {
    messenger.showSnackBar(const SnackBar(content: Text('Task created')));
  }
}

class _CreateTaskSheet extends HookConsumerWidget {
  const _CreateTaskSheet({
    required this.channelId,
    required this.channelName,
    required this.sourceEventId,
  });

  final String channelId;
  final String channelName;
  final String? sourceEventId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final titleController = useTextEditingController();
    final bodyController = useTextEditingController();
    useListenable(titleController);

    final dueAt = useState<DateTime?>(null);
    final scopeToChannel = useState(true);
    final assignToAgents = useState(false);
    final isSubmitting = useState(false);
    final actionError = useState<String?>(null);

    // Presets are resolved once per sheet rather than per rebuild, so a chip
    // cannot silently change the instant it stands for while the sheet is open.
    final presets = useMemoized(taskDuePresets);

    final agentPubkeys =
        ref.watch(channelBotPubkeysProvider(channelId)).asData?.value ??
        const <String>{};
    final userCache = ref.watch(userCacheProvider);
    final agentHandles = resolveAgentHandles(
      agentPubkeys: agentPubkeys,
      // Only the agents' names, not a copy of the whole profile cache: a title
      // keystroke rebuilds this widget.
      profileNames: {
        for (final pubkey in agentPubkeys)
          pubkey.toLowerCase(): ?userCache[pubkey.toLowerCase()]?.displayName,
      },
      directoryNames: ref.watch(agentDirectoryDisplayNamesProvider),
    );

    final title = titleController.text.trim();
    final titleLength = taskTitleLength(title);
    final titleTooLong = titleLength > maxTaskTitleChars;
    final canSubmit = title.isNotEmpty && !titleTooLong && !isSubmitting.value;

    Future<void> submit() async {
      if (!canSubmit) return;
      isSubmitting.value = true;
      actionError.value = null;
      try {
        final task = await ref
            .read(tasksApiProvider)
            .createTask(
              title: title,
              body: composeTaskBody(
                body: bodyController.text,
                agentHandles: assignToAgents.value ? agentHandles : const [],
              ),
              channelId: scopeToChannel.value ? channelId : null,
              sourceRef: sourceEventId,
              dueAt: dueAt.value,
            );
        if (context.mounted) Navigator.of(context).pop(task);
      } catch (error) {
        // The sheet is dismissible while the request is in flight, so a state
        // write after it has gone would be a `setState() after dispose()`.
        if (context.mounted) actionError.value = error.toString();
      } finally {
        if (context.mounted) isSubmitting.value = false;
      }
    }

    Future<void> pickDueDate() async {
      final now = DateTime.now();
      final picked = await showBuzzDialog<DateTime>(
        context: context,
        builder: (_) => DatePickerDialog(
          initialDate: dueAt.value ?? now,
          firstDate: DateTime(now.year, now.month, now.day),
          lastDate: now.add(const Duration(days: 365 * 2)),
        ),
      );
      // Land on 9am so a date-only pick matches the preset convention rather
      // than becoming midnight, which reads as "the day before" to a user.
      if (picked != null && context.mounted) {
        dueAt.value = DateTime(picked.year, picked.month, picked.day, 9);
      }
    }

    final scopeLabel = channelName.trim().isEmpty
        ? 'This conversation'
        : '#${channelName.trim()}';

    return Padding(
      padding: EdgeInsets.fromLTRB(
        Grid.gutter,
        0,
        Grid.gutter,
        MediaQuery.viewInsetsOf(context).bottom + Grid.xs,
      ),
      child: SafeArea(
        top: false,
        // Scrolling fields with a pinned action row: the form is tall enough
        // (two fields, a chip row, two option groups) to overflow a short
        // viewport, and "Create task" must not be the part that falls below
        // the fold.
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Flexible(
              child: SingleChildScrollView(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    _TaskField(
                      fieldKey: const ValueKey('create-task-title'),
                      controller: titleController,
                      enabled: !isSubmitting.value,
                      hintText: 'What needs doing?',
                      textInputAction: TextInputAction.next,
                    ),
                    if (titleTooLong)
                      _FieldMessage(
                        'Titles are limited to $maxTaskTitleChars characters.',
                        isError: true,
                      )
                    else if (titleLength > _titleCounterThreshold)
                      _FieldMessage('$titleLength/$maxTaskTitleChars'),
                    const SizedBox(height: Grid.xs),
                    _TaskField(
                      fieldKey: const ValueKey('create-task-body'),
                      controller: bodyController,
                      enabled: !isSubmitting.value,
                      hintText: 'Add detail (optional)',
                      minLines: 2,
                      maxLines: 4,
                      textInputAction: TextInputAction.newline,
                    ),
                    const SizedBox(height: Grid.xs),
                    _DueDatePicker(
                      presets: presets,
                      selected: dueAt.value,
                      enabled: !isSubmitting.value,
                      onSelect: (value) => dueAt.value = value,
                      onPickDate: pickDueDate,
                    ),
                    const SizedBox(height: Grid.xs),
                    AppListCard(
                      label: 'Scope',
                      children: [
                        AppListRow(
                          key: const ValueKey('create-task-scope-channel'),
                          icon: LucideIcons.hash,
                          title: scopeLabel,
                          trailing: _SelectedCheck(
                            selected: scopeToChannel.value,
                          ),
                          onTap: isSubmitting.value
                              ? null
                              : () => scopeToChannel.value = true,
                        ),
                        AppListRow(
                          key: const ValueKey('create-task-scope-community'),
                          icon: LucideIcons.globe,
                          title: 'Whole community',
                          trailing: _SelectedCheck(
                            selected: !scopeToChannel.value,
                          ),
                          onTap: isSubmitting.value
                              ? null
                              : () => scopeToChannel.value = false,
                        ),
                      ],
                    ),
                    // Hidden rather than disabled when the channel has no agents: an
                    // always-visible row that can never do anything is just noise.
                    if (agentHandles.isNotEmpty)
                      AppListCard(
                        children: [
                          AppListRow(
                            key: const ValueKey('create-task-assign-agents'),
                            icon: LucideIcons.bot,
                            title: 'Mention this channel’s agents',
                            subtitle: agentHandles.map((h) => '@$h').join(' '),
                            subtitleMaxLines: 2,
                            trailing: _SelectedCheck(
                              selected: assignToAgents.value,
                            ),
                            onTap: isSubmitting.value
                                ? null
                                : () => assignToAgents.value =
                                      !assignToAgents.value,
                          ),
                        ],
                      ),
                    if (actionError.value case final error?)
                      _FieldMessage(error, isError: true),
                  ],
                ),
              ),
            ),
            const SizedBox(height: Grid.xs),
            FilledButton(
              key: const ValueKey('create-task-submit'),
              onPressed: canSubmit ? submit : null,
              child: Text(isSubmitting.value ? 'Creating…' : 'Create task'),
            ),
          ],
        ),
      ),
    );
  }
}

/// The due-date chip row: one chip per preset, plus a custom-date chip.
class _DueDatePicker extends StatelessWidget {
  const _DueDatePicker({
    required this.presets,
    required this.selected,
    required this.enabled,
    required this.onSelect,
    required this.onPickDate,
  });

  final List<TaskDuePreset> presets;
  final DateTime? selected;
  final bool enabled;
  final ValueChanged<DateTime?> onSelect;
  final VoidCallback onPickDate;

  @override
  Widget build(BuildContext context) {
    final isCustom =
        selected != null && !presets.any((preset) => preset.dueAt == selected);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Padding(
          padding: const EdgeInsets.only(left: Grid.half, bottom: Grid.xxs),
          child: Text(
            'Due',
            style: context.textTheme.labelMedium?.copyWith(
              color: context.colors.onSurfaceVariant,
              fontWeight: FontWeight.w600,
            ),
          ),
        ),
        Wrap(
          spacing: Grid.half,
          runSpacing: Grid.half,
          children: [
            for (final preset in presets)
              InputChip(
                key: ValueKey('create-task-due-${preset.label}'),
                label: Text(preset.label),
                selected: selected == preset.dueAt,
                // Re-tapping the selected chip clears the due date, so the
                // field stays optional without a separate clear control.
                onPressed: enabled
                    ? () => onSelect(
                        selected == preset.dueAt ? null : preset.dueAt,
                      )
                    : null,
              ),
            InputChip(
              key: const ValueKey('create-task-due-custom'),
              avatar: const Icon(LucideIcons.calendarClock, size: 16),
              label: Text(isCustom ? _formatDate(selected!) : 'Pick a date'),
              selected: isCustom,
              onPressed: enabled ? onPickDate : null,
            ),
          ],
        ),
      ],
    );
  }
}

/// `2026-08-24`-style label — unambiguous, and no `intl` locale to thread here.
String _formatDate(DateTime value) {
  final month = value.month.toString().padLeft(2, '0');
  final day = value.day.toString().padLeft(2, '0');
  return '${value.year}-$month-$day';
}

class _SelectedCheck extends StatelessWidget {
  const _SelectedCheck({required this.selected});

  final bool selected;

  @override
  Widget build(BuildContext context) {
    if (!selected) return const SizedBox.shrink();
    return Icon(LucideIcons.check, size: 18, color: context.colors.primary);
  }
}

/// Inline helper or error line under a field.
class _FieldMessage extends StatelessWidget {
  const _FieldMessage(this.message, {this.isError = false});

  final String message;
  final bool isError;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(top: Grid.half, left: Grid.half),
      child: Text(
        message,
        style: context.textTheme.bodySmall?.copyWith(
          color: isError
              ? context.colors.error
              : context.colors.onSurfaceVariant,
        ),
      ),
    );
  }
}

/// Bordered, borderless-inside text field matching `_ManageChannelTextField`.
class _TaskField extends StatelessWidget {
  const _TaskField({
    required this.fieldKey,
    required this.controller,
    required this.enabled,
    required this.hintText,
    required this.textInputAction,
    this.minLines = 1,
    this.maxLines = 1,
  });

  final Key fieldKey;
  final TextEditingController controller;
  final bool enabled;
  final String hintText;
  final TextInputAction textInputAction;
  final int minLines;
  final int maxLines;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: BoxDecoration(
        border: Border.all(
          color: context.colors.outlineVariant.withValues(alpha: 0.8),
        ),
        borderRadius: BorderRadius.circular(Radii.card),
      ),
      child: Semantics(
        label: hintText,
        textField: true,
        child: TextField(
          key: fieldKey,
          controller: controller,
          enabled: enabled,
          minLines: minLines,
          maxLines: maxLines,
          style: context.textTheme.bodyLarge,
          decoration: InputDecoration(
            hintText: hintText,
            hintStyle: context.textTheme.bodyLarge?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
            border: InputBorder.none,
            enabledBorder: InputBorder.none,
            focusedBorder: InputBorder.none,
            isDense: true,
            contentPadding: const EdgeInsets.symmetric(
              horizontal: Grid.xs,
              vertical: Grid.twelve,
            ),
          ),
          textInputAction: textInputAction,
        ),
      ),
    );
  }
}
