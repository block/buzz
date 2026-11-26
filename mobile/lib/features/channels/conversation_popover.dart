part of 'channel_actions_sheet.dart';

Future<void> _renameConversation(
  BuildContext context,
  WidgetRef ref,
  Channel channel, {
  ValueChanged<String>? onRenamed,
}) {
  final actions = ref.read(channelActionsProvider);
  return showTextEditor(
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
      await actions.updateChannel(channelId: channel.id, name: trimmed);
      if (context.mounted) onRenamed?.call(trimmed);
    },
  );
}

Future<bool?> showConversationPopover({
  required BuildContext anchorContext,
  required Channel channel,
  required String? viewer,
  required bool personal,
  required bool isUnread,
  String? sectionId,
  ConversationMenuSession? nativeSession,
}) {
  final box = anchorContext.findRenderObject() as RenderBox;
  final rect = box.localToGlobal(Offset.zero) & box.size;
  final native = nativeSession != null || usesAndroidConversationMenu;
  final content = HookConsumer(
    builder: (context, ref, _) {
      final members =
          ref.watch(channelMembersProvider(channel.id)).value ??
          ref
              .read(channelsProvider.notifier)
              .cachedMembersForChannel(channel.id);
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
                    owners?[member.pubkey.toLowerCase()]?.toLowerCase() == key),
          );
      final sections = ref.watch(channelSectionsProvider);
      final workspace = useMeWorkspace(
        context,
        ref,
        enabled: personal,
        channelId: channel.id,
      );
      final starred =
          ref.watch(channelStarsProvider).store.channels[channel.id]?.enabled ==
          true;
      final pinned =
          ref.watch(channelPinsProvider).store.channels[channel.id]?.enabled ==
          true;
      final callbacks = <String, FutureOr<void> Function()>{};
      Widget item(
        String label,
        IconData icon,
        FutureOr<void> Function() onTap,
      ) {
        callbacks[label] = onTap;
        return ListTile(
          leading: Icon(icon),
          title: Text(label),
          onTap: () => onTap(),
          dense: true,
        );
      }

      Future<void> finish(Future<void> action) async {
        await action;
        if (context.mounted && ModalRoute.of(context)?.isCurrent == true) {
          Navigator.of(context).pop();
        }
      }

      final menu = Column(
        mainAxisSize: MainAxisSize.min,
        children: [
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
          item(
            pinned ? 'Unpin' : 'Pin',
            pinned ? BuzzIcons.pinnedOff : BuzzIcons.pinned,
            () {
              ref
                  .read(channelPinsProvider.notifier)
                  .setEnabled(channel.id, !pinned);
              Navigator.of(context).pop();
            },
          ),
          item(
            starred ? 'Unstar' : 'Star',
            starred ? BuzzIcons.starOff : BuzzIcons.star,
            () {
              ref
                  .read(channelStarsProvider.notifier)
                  .setEnabled(channel.id, !starred);
              Navigator.of(context).pop();
            },
          ),
          if (manage && !channel.isArchived)
            item(
              'Rename',
              BuzzIcons.pencil,
              () => finish(_renameConversation(context, ref, channel)),
            ),
          if (workspace.openPicker != null)
            item(
              'Change workspace',
              BuzzIcons.folder,
              () => finish(workspace.openPicker!()),
            ),
          if (!personal && sections.editable)
            item('Move to section…', BuzzIcons.folderInput, () {
              final pageContext = Navigator.of(
                context,
                rootNavigator: true,
              ).context;
              Navigator.of(context).pop();
              _showMoveSectionSheet(
                pageContext,
                channel: channel,
                sectionId: sections.isReady
                    ? sections.store.assignments[channel.id]
                    : sectionId,
              );
            }),
          if (manage)
            item(
              channel.isArchived ? 'Unarchive' : 'Archive',
              BuzzIcons.archive,
              () {
                final verb = channel.isArchived ? 'Unarchive' : 'Archive';
                return _confirmAndRun(
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
        ],
      );
      if (!native) return menu;
      return _NativeMenuRequest(
        callbacks: callbacks,
        rect: rect,
        session: nativeSession,
      );
    },
  );
  if (!native) {
    return showAnchoredPopoverContent<bool>(
      context: anchorContext,
      width: 220,
      alignment: AnchoredPopoverAlignment.start,
      items: [content],
    );
  }
  return _showNativeChannelSheet(anchorContext, (_) => content);
}

class _NativeMenuRequest extends HookWidget {
  const _NativeMenuRequest({
    required this.callbacks,
    required this.rect,
    this.session,
  });
  final Map<String, FutureOr<void> Function()> callbacks;
  final Rect rect;
  final ConversationMenuSession? session;
  static const android = MethodChannel('buzz/conversation_popup');
  static int nextRequest = 0;

  @override
  Widget build(BuildContext context) {
    final id = useMemoized(() => ++nextRequest, const []);
    final latest = useRef(callbacks)..value = callbacks;
    final waiting = useRef(true);
    final sent = useRef<List<String>?>(null);
    final host = ModalRoute.of(context)!;
    final navigator = Navigator.of(context);
    final labels = callbacks.keys.toList();
    void retire() {
      if (host.isCurrent) {
        navigator.pop();
      } else if (host.isActive) {
        navigator.removeRoute(host);
      }
    }

    Future<void> dismiss() async {
      if (session != null) {
        await session!.bridge?.invokeMethod<void>('dismiss');
        session!.close();
      } else {
        await android.invokeMethod<void>('dismiss', {'id': id});
      }
    }

    Future<void> sync() async {
      if (!context.mounted || !waiting.value) return;
      if (!host.isCurrent) {
        await dismiss();
        if (context.mounted) retire();
        return;
      }
      if (listEquals(sent.value, labels)) return;
      final initial = sent.value == null;
      sent.value = labels;
      if (!initial) {
        if (session != null) {
          await session!.bridge?.invokeMethod<void>('update', labels);
        } else {
          await android.invokeMethod<void>('update', {
            'id': id,
            'labels': labels,
          });
        }
        return;
      }
      String? selected;
      try {
        if (session != null) {
          if (!session!.labels.isCompleted) session!.labels.complete(labels);
          selected = await session!.selection.future;
        } else {
          selected = await android.invokeMethod<String>('show', {
            'id': id,
            'labels': labels,
            'surface': Theme.of(
              context,
            ).colorScheme.surfaceContainerHigh.toARGB32(),
            'foreground': Theme.of(context).colorScheme.onSurface.toARGB32(),
            'x': rect.left,
            'y': rect.top,
            'width': rect.width,
            'height': rect.height,
          });
        }
      } on PlatformException catch (error) {
        debugPrint('Conversation menu presentation failed: ${error.code}');
      }
      waiting.value = false;
      if (!context.mounted) return;
      try {
        if (host.isCurrent) await latest.value[selected]?.call();
      } finally {
        if (context.mounted) retire();
      }
    }

    WidgetsBinding.instance.addPostFrameCallback((_) => sync());
    useEffect(
      () => () {
        if (waiting.value && sent.value != null) unawaited(dismiss());
      },
      const [],
    );
    return const SizedBox.shrink();
  }
}
