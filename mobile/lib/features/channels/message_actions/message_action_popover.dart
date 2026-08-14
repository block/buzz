part of '../message_actions.dart';

const _messageActionRowHeight = 48.0;
const _messageActionSeparatorHeight = 0.5;
const _messageActionVerticalInset = Grid.half;
const _messageActionMenuMaxWidth = 288.0;
const _messageActionPreviewMaxWidth = 358.0;
const _messageActionPreviewInset = Grid.xxs;
const _messageActionGap = Grid.twelve;
const _messageActionTransitionDuration = _reactionPopoverDuration;
const _iosMessageActionTransitionDuration = Duration(milliseconds: 220);
const _iosNativeMessageActionSurfaceChannel = MethodChannel(
  'buzz/native_message_action_surface',
);

bool _messageActionsPresentationInFlight = false;
bool? _iosNativeMessageActionSurfaceSupported;

Future<bool> _supportsIosNativeMessageActionSurface() async {
  if (!Platform.isIOS) return false;
  final cached = _iosNativeMessageActionSurfaceSupported;
  if (cached != null) return cached;

  try {
    final supported =
        await _iosNativeMessageActionSurfaceChannel.invokeMethod<bool>(
          'isSupported',
        ) ??
        false;
    _iosNativeMessageActionSurfaceSupported = supported;
    return supported;
  } on MissingPluginException {
    _iosNativeMessageActionSurfaceSupported = false;
    return false;
  } on PlatformException {
    _iosNativeMessageActionSurfaceSupported = false;
    return false;
  }
}

bool _tryShowMessageActionsPopover({
  required BuildContext context,
  required WidgetRef ref,
  required TimelineMessage message,
  required String channelId,
  required bool canManageMessage,
  required List<TimelineMessage>? allMessages,
  required String? currentPubkey,
  required bool isMember,
  required bool isArchived,
  required Rect? anchorRect,
  required Future<ui.Image> Function()? captureAnchorSnapshot,
  required VoidCallback? onPopoverPresented,
  required VoidCallback? onPopoverDismissed,
}) {
  if (anchorRect == null || captureAnchorSnapshot == null) return false;
  unawaited(
    _showMessageActionsPopover(
      context: context,
      ref: ref,
      message: message,
      channelId: channelId,
      canManageMessage: canManageMessage,
      allMessages: allMessages,
      currentPubkey: currentPubkey,
      isMember: isMember,
      isArchived: isArchived,
      anchorRect: anchorRect,
      captureAnchorSnapshot: captureAnchorSnapshot,
      onPopoverPresented: onPopoverPresented,
      onPopoverDismissed: onPopoverDismissed,
    ).then((shown) {
      if (shown || !context.mounted) return;
      showMessageActions(
        context: context,
        ref: ref,
        message: message,
        channelId: channelId,
        canManageMessage: canManageMessage,
        allMessages: allMessages,
        currentPubkey: currentPubkey,
        isMember: isMember,
        isArchived: isArchived,
      );
    }),
  );
  return true;
}

Future<bool> _showMessageActionsPopover({
  required BuildContext context,
  required WidgetRef ref,
  required TimelineMessage message,
  required String channelId,
  required bool canManageMessage,
  required List<TimelineMessage>? allMessages,
  required String? currentPubkey,
  required bool isMember,
  required bool isArchived,
  required Rect anchorRect,
  required Future<ui.Image> Function() captureAnchorSnapshot,
  required VoidCallback? onPopoverPresented,
  required VoidCallback? onPopoverDismissed,
}) async {
  if (_messageActionsPresentationInFlight) return true;
  _messageActionsPresentationInFlight = true;

  try {
    final actions = _buildPopoverMessageActions(
      context: context,
      ref: ref,
      message: message,
      channelId: channelId,
      canManageMessage: canManageMessage,
      allMessages: allMessages,
      currentPubkey: currentPubkey,
      isMember: isMember,
      isArchived: isArchived,
    );
    if (actions.isEmpty) return false;
    final nativeActionSurfaceSupport = _supportsIosNativeMessageActionSurface();
    final isIos = defaultTargetPlatform == TargetPlatform.iOS;

    unawaited(HapticFeedback.mediumImpact());

    final ui.Image snapshot;
    try {
      snapshot = await captureAnchorSnapshot();
    } catch (_) {
      return false;
    }
    if (!context.mounted) {
      snapshot.dispose();
      return false;
    }
    final useIosNativeActionSurface = await nativeActionSurfaceSupport;
    if (!context.mounted) {
      snapshot.dispose();
      return false;
    }

    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    onPopoverPresented?.call();

    String? selectedActionId;
    try {
      selectedActionId = await showGeneralDialog<String>(
        context: context,
        barrierDismissible: true,
        barrierLabel: 'Dismiss message actions',
        barrierColor: Colors.transparent,
        transitionDuration: reduceMotion
            ? Duration.zero
            : isIos
            ? _iosMessageActionTransitionDuration
            : _messageActionTransitionDuration,
        transitionBuilder: (context, animation, secondaryAnimation, child) =>
            child,
        pageBuilder: (dialogContext, animation, secondaryAnimation) =>
            _MessageActionsPopover(
              anchorRect: anchorRect,
              anchorSnapshot: snapshot,
              animation: animation,
              message: message,
              pageContext: context,
              pageRef: ref,
              actions: actions,
              useIosNativeActionSurface: useIosNativeActionSurface,
            ),
      );
    } finally {
      snapshot.dispose();
      if (context.mounted) onPopoverDismissed?.call();
    }

    for (final action in actions) {
      if (action.id != selectedActionId) continue;
      await Future<void>.sync(action.onSelected);
      break;
    }
    return true;
  } finally {
    _messageActionsPresentationInFlight = false;
  }
}

List<_PopoverMessageAction> _buildPopoverMessageActions({
  required BuildContext context,
  required WidgetRef ref,
  required TimelineMessage message,
  required String channelId,
  required bool canManageMessage,
  required List<TimelineMessage>? allMessages,
  required String? currentPubkey,
  required bool isMember,
  required bool isArchived,
}) {
  final actions = <_PopoverMessageAction>[];
  final messages = allMessages;
  final canRemind = ref.read(reminderServiceProvider) != null;

  if (!message.isSystem) {
    if (messages != null) {
      actions.add(
        _PopoverMessageAction(
          id: 'reply',
          title: 'Reply',
          icon: LucideIcons.messageSquareReply,
          group: _PopoverMessageActionGroup.primary,
          onSelected: () {
            if (!context.mounted) return;
            Navigator.of(context).push(
              MaterialPageRoute<void>(
                builder: (_) => ThreadDetailPage(
                  threadHead: message,
                  allMessages: messages,
                  channelId: channelId,
                  currentPubkey: currentPubkey,
                  isMember: isMember,
                  isArchived: isArchived,
                ),
              ),
            );
          },
        ),
      );
    }
    actions.add(
      _PopoverMessageAction(
        id: 'copyLink',
        title: 'Copy link',
        icon: LucideIcons.link2,
        group: _PopoverMessageActionGroup.utility,
        onSelected: () {
          if (!context.mounted) return;
          copyToClipboard(
            context,
            messageLinkFor(message: message, channelId: channelId),
            message: 'Message link copied',
          );
        },
      ),
    );
    if (canRemind) {
      actions.add(
        _PopoverMessageAction(
          id: 'remind',
          title: 'Remind me',
          icon: LucideIcons.clock,
          group: _PopoverMessageActionGroup.utility,
          onSelected: () {
            if (!context.mounted) return;
            showRemindMeLaterSheet(
              context: Navigator.of(context, rootNavigator: true).context,
              ref: ref,
              target: ReminderTarget(
                eventId: message.id,
                channelId: channelId,
                preview: message.content.characters
                    .take(_reminderPreviewLength)
                    .toString(),
                authorPubkey: message.pubkey,
              ),
            );
          },
        ),
      );
    }

    final readState = ref.read(readStateProvider);
    if (readState.isReady) {
      final unread = isMessageUnread(
        readState,
        channelId: channelId,
        messageId: message.id,
        createdAt: message.createdAt,
        threadRootId: message.rootId,
      );
      actions.add(
        _PopoverMessageAction(
          id: unread ? 'markRead' : 'markUnread',
          title: unread ? 'Mark read' : 'Mark unread',
          icon: unread ? LucideIcons.mailCheck : LucideIcons.mailOpen,
          group: _PopoverMessageActionGroup.primary,
          onSelected: () {
            final notifier = ref.read(readStateProvider.notifier);
            if (unread) {
              notifier.markContextRead(
                msgContextKey(message.id),
                message.createdAt,
              );
            } else {
              notifier.markContextUnread(
                msgContextKey(message.id),
                channelId: channelId,
              );
            }
          },
        ),
      );
    }

    final rootId = message.rootId ?? message.id;
    final following = ref.read(threadFollowsProvider).isFollowing(rootId);
    actions.add(
      _PopoverMessageAction(
        id: following ? 'unfollowThread' : 'followThread',
        title: following ? 'Unfollow thread' : 'Follow thread',
        icon: following ? LucideIcons.bellOff : LucideIcons.bellRing,
        group: _PopoverMessageActionGroup.utility,
        onSelected: () {
          final notifier = ref.read(threadFollowsProvider.notifier);
          if (following) {
            notifier.unfollowThread(rootId);
          } else {
            notifier.followThread(rootId);
          }
        },
      ),
    );
    actions.add(
      _PopoverMessageAction(
        id: 'copyText',
        title: 'Copy text',
        icon: LucideIcons.copy,
        group: _PopoverMessageActionGroup.utility,
        onSelected: () =>
            Clipboard.setData(ClipboardData(text: message.content)),
      ),
    );
  }

  if (canManageMessage) {
    actions.add(
      _PopoverMessageAction(
        id: 'edit',
        title: 'Edit message',
        icon: LucideIcons.pencil,
        group: _PopoverMessageActionGroup.primary,
        onSelected: () {
          if (!context.mounted) return;
          _showEditSheet(
            context: context,
            ref: ref,
            message: message,
            channelId: channelId,
          );
        },
      ),
    );
    actions.add(
      _PopoverMessageAction(
        id: 'delete',
        title: 'Delete message',
        icon: LucideIcons.trash2,
        group: _PopoverMessageActionGroup.destructive,
        destructive: true,
        onSelected: () {
          if (!context.mounted) return;
          _confirmDelete(
            context: context,
            ref: ref,
            channelId: channelId,
            messageId: message.id,
          );
        },
      ),
    );
  }

  const actionOrder = {
    'reply': 0,
    'markRead': 1,
    'markUnread': 1,
    'edit': 2,
    'copyText': 3,
    'copyLink': 4,
    'remind': 5,
    'followThread': 6,
    'unfollowThread': 6,
    'delete': 7,
  };
  actions.sort(
    (left, right) => actionOrder[left.id]!.compareTo(actionOrder[right.id]!),
  );
  return actions;
}

enum _PopoverMessageActionGroup { primary, utility, destructive }

class _PopoverMessageAction {
  final String id;
  final String title;
  final IconData icon;
  final _PopoverMessageActionGroup group;
  final bool destructive;
  final FutureOr<void> Function() onSelected;

  const _PopoverMessageAction({
    required this.id,
    required this.title,
    required this.icon,
    required this.group,
    required this.onSelected,
    this.destructive = false,
  });

  String get iosSymbol => switch (id) {
    'reply' => 'arrowshape.turn.up.left',
    'markRead' => 'envelope.open',
    'markUnread' => 'envelope.badge',
    'edit' => 'pencil',
    'copyText' => 'doc.on.doc',
    'copyLink' => 'link',
    'remind' => 'clock',
    'followThread' => 'bell',
    'unfollowThread' => 'bell.slash',
    'delete' => 'trash',
    _ => 'ellipsis',
  };

  Map<String, Object> toPlatformArguments() => {
    'id': id,
    'title': title,
    'symbol': iosSymbol,
    'group': group.name,
    'destructive': destructive,
  };
}

class _IosNativeMessageActionSurface extends HookWidget {
  final List<_PopoverMessageAction> actions;
  final ValueChanged<String> onSelected;

  const _IosNativeMessageActionSurface({
    required this.actions,
    required this.onSelected,
  });

  @override
  Widget build(BuildContext context) {
    final viewId = useState<int?>(null);
    useEffect(() {
      final id = viewId.value;
      if (id == null) return null;
      final channel = MethodChannel('buzz/native_message_action_surface/$id');
      channel.setMethodCallHandler((call) async {
        if (call.method != 'selected' || call.arguments is! Map) return;
        final actionId = (call.arguments as Map)['id'];
        if (actionId is String) onSelected(actionId);
      });
      return () => channel.setMethodCallHandler(null);
    }, [viewId.value, onSelected]);

    return UiKitView(
      key: const ValueKey('ios-native-message-action-surface'),
      viewType: 'buzz/native_message_action_surface',
      creationParams: <String, Object>{
        'actions': [for (final action in actions) action.toPlatformArguments()],
        'surfaceColor': context.colors.surface.toARGB32(),
        'foregroundColor': context.colors.onSurface.toARGB32(),
        'separatorColor': context.colors.outlineVariant.toARGB32(),
        'errorColor': context.colors.error.toARGB32(),
        'interfaceStyle': context.colors.brightness.name,
      },
      creationParamsCodec: const StandardMessageCodec(),
      onPlatformViewCreated: (id) => viewId.value = id,
    );
  }
}

class _MessageActionsPopover extends StatelessWidget {
  final Rect anchorRect;
  final ui.Image anchorSnapshot;
  final Animation<double> animation;
  final TimelineMessage message;
  final BuildContext pageContext;
  final WidgetRef pageRef;
  final List<_PopoverMessageAction> actions;
  final bool useIosNativeActionSurface;

  const _MessageActionsPopover({
    required this.anchorRect,
    required this.anchorSnapshot,
    required this.animation,
    required this.message,
    required this.pageContext,
    required this.pageRef,
    required this.actions,
    required this.useIosNativeActionSurface,
  });

  @override
  Widget build(BuildContext context) {
    final mediaQuery = MediaQuery.of(context);
    return LayoutBuilder(
      builder: (context, constraints) {
        final safeLeft = mediaQuery.padding.left + Grid.xxs;
        final safeRight =
            constraints.maxWidth - mediaQuery.padding.right - Grid.xxs;
        final safeTop = mediaQuery.padding.top + Grid.xxs;
        final safeBottom =
            constraints.maxHeight - mediaQuery.padding.bottom - Grid.xxs;
        final availableWidth = math.max(1.0, safeRight - safeLeft);
        final availableHeight = math.max(1.0, safeBottom - safeTop);
        final trayWidth = math.min(_reactionTrayMaxWidth, availableWidth);
        final menuWidth = math.min(_messageActionMenuMaxWidth, availableWidth);
        final preferredMenuHeight = _messageActionSurfacePreferredHeight(
          actions,
        );
        final menuBudget = math.max(
          _messageActionRowHeight,
          availableHeight -
              _reactionTrayMaxHeight -
              (_messageActionGap * 2) -
              72,
        );
        final menuHeight = math.min(preferredMenuHeight, menuBudget);
        final previewMaximumHeight = math.max(
          48.0,
          availableHeight -
              _reactionTrayMaxHeight -
              menuHeight -
              (_messageActionGap * 2),
        );
        final previewInsetExtent = _messageActionPreviewInset * 2;
        final previewWidthRatio =
            (math.min(_messageActionPreviewMaxWidth, availableWidth) -
                previewInsetExtent) /
            math.max(anchorRect.width, 1);
        final previewHeightRatio =
            (previewMaximumHeight - previewInsetExtent) /
            math.max(anchorRect.height, 1);
        final previewScale = math.min(
          1.0,
          math.max(0.1, math.min(previewWidthRatio, previewHeightRatio)),
        );
        final previewSize = Size(
          (anchorRect.width * previewScale) + previewInsetExtent,
          (anchorRect.height * previewScale) + previewInsetExtent,
        );
        final totalHeight =
            _reactionTrayMaxHeight +
            _messageActionGap +
            previewSize.height +
            _messageActionGap +
            menuHeight;
        final contentTop = math.max(safeTop, safeBottom - totalHeight);
        final surfaceLeft =
            safeLeft + ((availableWidth - math.max(trayWidth, menuWidth)) / 2);
        final trayRect = Rect.fromLTWH(
          surfaceLeft,
          contentTop,
          trayWidth,
          _reactionTrayMaxHeight,
        );
        final trayHostWidth = math.min(
          trayWidth + _reactionTraySpringAllowance,
          safeRight - surfaceLeft,
        );
        final previewRect = Rect.fromLTWH(
          surfaceLeft,
          trayRect.bottom + _messageActionGap,
          previewSize.width,
          previewSize.height,
        );
        final menuRect = Rect.fromLTWH(
          surfaceLeft,
          previewRect.bottom + _messageActionGap,
          menuWidth,
          menuHeight,
        );

        return Stack(
          children: [
            Positioned.fill(
              child: BackdropFilter(
                filter: ui.ImageFilter.blur(
                  sigmaX: defaultTargetPlatform == TargetPlatform.iOS ? 4 : 8,
                  sigmaY: defaultTargetPlatform == TargetPlatform.iOS ? 4 : 8,
                ),
                child: AnimatedBuilder(
                  animation: animation,
                  builder: (context, child) {
                    final opacity = Curves.easeOutCubic.transform(
                      animation.value,
                    );
                    return ColoredBox(
                      key: const ValueKey('message-actions-background'),
                      color: context.colors.inverseSurface.withValues(
                        alpha: 0.14 * opacity,
                      ),
                    );
                  },
                ),
              ),
            ),
            Positioned.fill(
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onTap: () => Navigator.of(context).pop(),
              ),
            ),
            Positioned.fromRect(
              rect: previewRect,
              child: AnimatedBuilder(
                animation: animation,
                child: RepaintBoundary(
                  child: _LiftedMessagePreview(anchorSnapshot: anchorSnapshot),
                ),
                builder: (context, child) {
                  final movement =
                      (defaultTargetPlatform == TargetPlatform.iOS
                              ? Curves.easeOutCubic
                              : Curves.easeInOutCubic)
                          .transform(animation.value);
                  final sourceRect = anchorRect.inflate(
                    _messageActionPreviewInset,
                  );
                  final translation = Offset(
                    ui.lerpDouble(
                      sourceRect.left - previewRect.left,
                      0,
                      movement,
                    )!,
                    ui.lerpDouble(
                      sourceRect.top - previewRect.top,
                      0,
                      movement,
                    )!,
                  );
                  final scaleX = ui.lerpDouble(
                    sourceRect.width / previewRect.width,
                    1,
                    movement,
                  )!;
                  final scaleY = ui.lerpDouble(
                    sourceRect.height / previewRect.height,
                    1,
                    movement,
                  )!;
                  return Transform.translate(
                    offset: translation,
                    child: Transform(
                      alignment: Alignment.topLeft,
                      transform: Matrix4.diagonal3Values(scaleX, scaleY, 1),
                      child: child,
                    ),
                  );
                },
              ),
            ),
            Positioned(
              left: trayRect.left,
              top: trayRect.top,
              width: trayHostWidth,
              height: trayRect.height,
              child: _MessageReactionTray(
                animation: animation,
                trayWidth: trayWidth,
                message: message,
                pageContext: pageContext,
                pageRef: pageRef,
              ),
            ),
            Positioned.fromRect(
              rect: menuRect,
              child: AnimatedBuilder(
                animation: animation,
                child: useIosNativeActionSurface
                    ? _IosNativeMessageActionSurface(
                        actions: actions,
                        onSelected: (actionId) =>
                            Navigator.of(context).pop(actionId),
                      )
                    : _MessageActionSurface(
                        actions: actions,
                        onSelected: (actionId) =>
                            Navigator.of(context).pop(actionId),
                      ),
                builder: (context, child) {
                  final appearance = const Interval(
                    0.08,
                    0.82,
                    curve: Curves.easeOutCubic,
                  ).transform(animation.value);
                  final fadedChild = Opacity(opacity: appearance, child: child);
                  if (defaultTargetPlatform == TargetPlatform.iOS) {
                    return fadedChild;
                  }
                  return Transform.scale(
                    alignment: Alignment.topLeft,
                    scale: ui.lerpDouble(0.96, 1, appearance)!,
                    child: fadedChild,
                  );
                },
              ),
            ),
          ],
        );
      },
    );
  }
}

class _LiftedMessagePreview extends StatelessWidget {
  final ui.Image anchorSnapshot;

  const _LiftedMessagePreview({required this.anchorSnapshot});

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      key: const ValueKey('message-action-preview'),
      decoration: BoxDecoration(
        color: context.colors.surfaceContainerHigh,
        borderRadius: BorderRadius.circular(Radii.md),
        border: Border.all(
          color: context.colors.outlineVariant.withValues(alpha: 0.7),
          width: 0.5,
        ),
        boxShadow: [
          BoxShadow(
            color: Colors.black.withValues(alpha: 0.18),
            blurRadius: 18,
            offset: const Offset(0, 8),
          ),
        ],
      ),
      child: Padding(
        padding: const EdgeInsets.all(_messageActionPreviewInset),
        child: ClipRRect(
          borderRadius: BorderRadius.circular(Radii.xs),
          child: RawImage(
            image: anchorSnapshot,
            fit: BoxFit.fill,
            filterQuality: FilterQuality.medium,
          ),
        ),
      ),
    );
  }
}

class _MessageReactionTray extends StatelessWidget {
  final Animation<double> animation;
  final double trayWidth;
  final TimelineMessage message;
  final BuildContext pageContext;
  final WidgetRef pageRef;

  const _MessageReactionTray({
    required this.animation,
    required this.trayWidth,
    required this.message,
    required this.pageContext,
    required this.pageRef,
  });

  @override
  Widget build(BuildContext context) {
    return _AnimatedReactionTray(
      trayKey: const ValueKey('message-action-reaction-tray'),
      animation: animation,
      trayWidth: trayWidth,
      scaleAlignment: Alignment.bottomLeft,
      message: message,
      pageContext: pageContext,
      pageRef: pageRef,
    );
  }
}

class _MessageActionSurface extends StatelessWidget {
  final List<_PopoverMessageAction> actions;
  final ValueChanged<String> onSelected;

  const _MessageActionSurface({
    required this.actions,
    required this.onSelected,
  });

  @override
  Widget build(BuildContext context) {
    return Material(
      key: const ValueKey('message-action-surface'),
      color: context.colors.surface,
      surfaceTintColor: Colors.transparent,
      elevation: 10,
      shadowColor: Colors.black.withValues(alpha: 0.22),
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(Radii.dialog),
        side: BorderSide(
          color: context.colors.outlineVariant.withValues(alpha: 0.55),
          width: 0.5,
        ),
      ),
      clipBehavior: Clip.antiAlias,
      child: SingleChildScrollView(
        child: Padding(
          padding: const EdgeInsets.symmetric(
            vertical: _messageActionVerticalInset,
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              for (var index = 0; index < actions.length; index++) ...[
                if (index > 0 &&
                    actions[index - 1].group != actions[index].group)
                  Divider(
                    key: ValueKey(
                      'message-action-divider-${actions[index].group.name}',
                    ),
                    height: _messageActionSeparatorHeight,
                    thickness: _messageActionSeparatorHeight,
                    indent: Grid.xs,
                    endIndent: Grid.xs,
                  ),
                _MessageActionRow(
                  action: actions[index],
                  onSelected: onSelected,
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

class _MessageActionRow extends StatelessWidget {
  final _PopoverMessageAction action;
  final ValueChanged<String> onSelected;

  const _MessageActionRow({required this.action, required this.onSelected});

  @override
  Widget build(BuildContext context) {
    final foreground = action.destructive
        ? context.colors.error
        : context.colors.onSurface;
    return Semantics(
      button: true,
      label: action.title,
      excludeSemantics: true,
      child: InkWell(
        key: ValueKey('message-action-${action.id}'),
        onTap: () {
          unawaited(HapticFeedback.lightImpact());
          onSelected(action.id);
        },
        child: SizedBox(
          height: _messageActionRowHeight,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: Grid.xs),
            child: Row(
              children: [
                SizedBox(
                  width: 32,
                  child: Center(
                    child: Icon(action.icon, size: 22, color: foreground),
                  ),
                ),
                const SizedBox(width: Grid.twelve),
                Expanded(
                  child: Text(
                    action.title,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: context.textTheme.bodyLarge?.copyWith(
                      color: foreground,
                    ),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

double _messageActionSurfacePreferredHeight(
  List<_PopoverMessageAction> actions,
) {
  var separatorCount = 0;
  for (var index = 1; index < actions.length; index++) {
    if (actions[index - 1].group != actions[index].group) separatorCount += 1;
  }
  return (_messageActionVerticalInset * 2) +
      (actions.length * _messageActionRowHeight) +
      (separatorCount * _messageActionSeparatorHeight);
}
