part of 'channel_actions_sheet.dart';

Future<void> _renameConversation(
  BuildContext context,
  WidgetRef ref,
  Channel channel, {
  ValueChanged<String>? onRenamed,
}) => showTextEditor(
  context: context,
  title: 'Rename',
  hintText: 'Name',
  initialValue: channel.name,
  requireNonEmpty: true,
  onSave: (name) async {
    final trimmed = name.trim().replaceFirst(RegExp(r'^#+'), '').trim();
    if (trimmed.isEmpty) {
      throw const FormatException('Channel name is required.');
    }
    if (trimmed == channel.name) return;
    await ref
        .read(channelActionsProvider)
        .updateChannel(channelId: channel.id, name: trimmed);
    if (context.mounted) onRenamed?.call(trimmed);
  },
);

Future<bool?> showConversationPopover({
  required BuildContext anchorContext,
  required Channel channel,
  required String? viewer,
  required bool personal,
  required bool isUnread,
  String? sectionId,
}) => showAnchoredPopoverContent<bool>(
  context: anchorContext,
  width: 220,
  alignment: AnchoredPopoverAlignment.start,
  items: [
    HookConsumer(
      builder: (context, ref, _) {
        final members =
            ref.watch(channelMembersProvider(channel.id)).value ?? [];
        final owners = ref.watch(agentOwnersProvider).value;
        final key =
            viewer?.toLowerCase() ??
            ref.watch(currentPubkeyProvider)?.toLowerCase();
        final manage =
            key != null &&
            members.any(
              (member) =>
                  (member.pubkey.toLowerCase() == key && member.isElevated) ||
                  (member.isOwner &&
                      owners?[member.pubkey.toLowerCase()]?.toLowerCase() ==
                          key),
            );
        final sections = ref.watch(channelSectionsProvider);
        final workspace = useMeWorkspace(
          context,
          ref,
          enabled: personal,
          channelId: channel.id,
        );
        Widget item(String label, IconData icon, VoidCallback onTap) =>
            ListTile(
              leading: Icon(icon),
              title: Text(label),
              onTap: onTap,
              dense: true,
            );
        Future<void> finish(Future<void> action) async {
          await action;
          if (context.mounted && ModalRoute.of(context)?.isCurrent == true) {
            Navigator.of(context).pop();
          }
        }

        return Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (manage && !channel.isArchived)
              item(
                'Rename',
                BuzzIcons.pencil,
                () => finish(_renameConversation(context, ref, channel)),
              ),
            if (manage)
              item(
                channel.isArchived ? 'Unarchive' : 'Archive',
                BuzzIcons.archive,
                () {
                  final verb = channel.isArchived ? 'Unarchive' : 'Archive';
                  _confirmAndRun(
                    context,
                    ref,
                    title: '$verb ${channel.name}?',
                    body: channel.isArchived
                        ? 'Members can post again.'
                        : 'The conversation will become read-only.',
                    confirmLabel: verb,
                    action: () {
                      final actions = ref.read(channelActionsProvider);
                      return channel.isArchived
                          ? actions.unarchiveChannel(channel.id)
                          : actions.archiveChannel(channel.id);
                    },
                  );
                },
              ),
            item(isUnread ? 'Mark read' : 'Mark unread', BuzzIcons.mail, () {
              if (isUnread) {
                _markChannelReadNow(ref, channel.id);
                Navigator.of(context).pop();
              } else if (ref
                  .read(readStateProvider.notifier)
                  .markContextUnread(channel.id, channelId: channel.id)) {
                Navigator.of(context).pop(true);
              } else {
                ScaffoldMessenger.of(context).showFeedbackSnackBar(
                  const SnackBar(
                    content: Text('Couldn’t mark as unread. Try again.'),
                  ),
                  kind: ToastKind.error,
                );
              }
            }),
            if (workspace.openPicker != null)
              item(
                'Change workspace',
                BuzzIcons.folder,
                () => finish(workspace.openPicker!()),
              ),
            if (!personal && sections.editable)
              item('Move to section…', BuzzIcons.folderInput, () {
                Navigator.of(context).pop();
                _showMoveSectionSheet(
                  anchorContext,
                  channel: channel,
                  sectionId: sections.isReady
                      ? sections.store.assignments[channel.id]
                      : sectionId,
                );
              }),
          ],
        );
      },
    ),
  ],
);
