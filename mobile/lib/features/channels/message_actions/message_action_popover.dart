part of '../message_actions.dart';

const _messageActionRowHeight = 48.0;
const _messageActionRowVerticalPadding = Grid.xxs;
const _messageActionSeparatorHeight = 0.5;
const _messageActionVerticalInset = Grid.half;
const _messageActionMenuMaxWidth = 288.0;
const _messageActionPreviewMaxWidth = 358.0;
const _messageActionPreviewInset = Grid.xxs;
const _messageActionGap = Grid.twelve;
const _messageActionReactionSelection = '__reaction__';
const _messageActionTransitionDuration = _reactionPopoverDuration;
const _iosMessageActionTransitionDuration = Duration(milliseconds: 220);
const _iosNativeMessageActionSurfaceChannel = MethodChannel(
  'buzz/native_message_action_surface',
);

bool _messageActionPresentationInFlight = false;
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
  required ValueChanged<bool>? onPopoverPreviewVisibilityChanged,
  required VoidCallback? onPopoverDismissed,
  required FocusNode? composerFocusNode,
  required VoidCallback? restoreComposerFocus,
}) {
  if (anchorRect == null || captureAnchorSnapshot == null) return false;
  final shouldRestoreComposerFocus = composerFocusNode?.hasFocus ?? false;
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
      onPopoverPreviewVisibilityChanged: onPopoverPreviewVisibilityChanged,
      onPopoverDismissed: onPopoverDismissed,
      composerFocusNode: composerFocusNode,
      restoreComposerFocus: restoreComposerFocus,
      shouldRestoreComposerFocus: shouldRestoreComposerFocus,
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
  required ValueChanged<bool>? onPopoverPreviewVisibilityChanged,
  required VoidCallback? onPopoverDismissed,
  required FocusNode? composerFocusNode,
  required VoidCallback? restoreComposerFocus,
  required bool shouldRestoreComposerFocus,
}) async {
  if (_messageActionPresentationInFlight) return true;
  _messageActionPresentationInFlight = true;

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
    if (shouldRestoreComposerFocus) composerFocusNode!.unfocus();

    messageActionBackdropActive.value = true;
    // Give the timeline one frame to replace UIKit glass platform views with
    // composable Flutter stand-ins before the full-screen blur is presented.
    await WidgetsBinding.instance.endOfFrame;
    if (!context.mounted) {
      messageActionBackdropActive.value = false;
      snapshot.dispose();
      return false;
    }

    String? selectedActionId;
    final dialogRoute = RawDialogRoute<String>(
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
            onPreviewVisibilityChanged: onPopoverPreviewVisibilityChanged,
          ),
    );
    var routePushed = false;
    try {
      final popResult = Navigator.of(
        context,
        rootNavigator: true,
      ).push(dialogRoute);
      routePushed = true;
      selectedActionId = await popResult;
    } finally {
      if (routePushed) await dialogRoute.completed;
      messageActionBackdropActive.value = false;
      snapshot.dispose();
      if (context.mounted) onPopoverDismissed?.call();
    }

    for (final action in actions) {
      if (action.id != selectedActionId) continue;
      await Future<void>.sync(action.onSelected);
      break;
    }
    if (selectedActionId == null &&
        shouldRestoreComposerFocus &&
        context.mounted) {
      restoreComposerFocus?.call();
    }
    return true;
  } finally {
    _messageActionPresentationInFlight = false;
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
  final double rowHeight;
  final ValueChanged<String> onSelected;

  const _IosNativeMessageActionSurface({
    required this.actions,
    required this.rowHeight,
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
        'rowHeight': rowHeight,
      },
      creationParamsCodec: const StandardMessageCodec(),
      onPlatformViewCreated: (id) => viewId.value = id,
    );
  }
}

class _MessageActionsPopover extends HookWidget {
  final Rect anchorRect;
  final ui.Image anchorSnapshot;
  final Animation<double> animation;
  final TimelineMessage message;
  final BuildContext pageContext;
  final WidgetRef pageRef;
  final List<_PopoverMessageAction> actions;
  final bool useIosNativeActionSurface;
  final ValueChanged<bool>? onPreviewVisibilityChanged;

  const _MessageActionsPopover({
    required this.anchorRect,
    required this.anchorSnapshot,
    required this.animation,
    required this.message,
    required this.pageContext,
    required this.pageRef,
    required this.actions,
    required this.useIosNativeActionSurface,
    required this.onPreviewVisibilityChanged,
  });

  @override
  Widget build(BuildContext context) {
    final mediaQuery = MediaQuery.of(context);
    final selectionStarted = useRef(false);

    void select(Object? result, [VoidCallback? effect]) {
      if (selectionStarted.value) return;
      selectionStarted.value = true;
      Navigator.of(context).pop(result);
      effect?.call();
    }

    void selectAction(String actionId) => select(actionId);
    return LayoutBuilder(
      builder: (context, constraints) {
        final safeLeft = mediaQuery.padding.left + Grid.xxs;
        final safeRight =
            constraints.maxWidth - mediaQuery.padding.right - Grid.xxs;
        final safeTop = mediaQuery.padding.top + Grid.xxs;
        final safeBottom =
            constraints.maxHeight -
            mediaQuery.padding.bottom -
            mediaQuery.viewInsets.bottom -
            Grid.xxs;
        final availableWidth = math.max(1.0, safeRight - safeLeft);
        final availableHeight = math.max(1.0, safeBottom - safeTop);
        final trayWidth = math.min(_reactionTrayMaxWidth, availableWidth);
        final menuWidth = math.min(_messageActionMenuMaxWidth, availableWidth);
        final menuLayout = _MessageActionSurfaceLayout.from(context, actions);
        final preferredMenuHeight = menuLayout.preferredHeight;
        final minimumMenuHeight = math.min(
          menuLayout.rowHeight,
          availableHeight,
        );
        final showReactionTray =
            availableHeight >=
            _reactionTrayMaxHeight + _messageActionGap + minimumMenuHeight;
        final trayHeight = showReactionTray ? _reactionTrayMaxHeight : 0.0;
        final trayGap = showReactionTray ? _messageActionGap : 0.0;
        final heightAfterTray = availableHeight - trayHeight - trayGap;
        final showPreview =
            showReactionTray &&
            heightAfterTray >= minimumMenuHeight + _messageActionGap + 48.0;
        final previewMinimumHeight = showPreview ? 48.0 : 0.0;
        final previewGap = showPreview ? _messageActionGap : 0.0;
        final menuBudget = math.max(
          minimumMenuHeight,
          availableHeight -
              trayHeight -
              trayGap -
              previewMinimumHeight -
              previewGap,
        );
        final menuHeight = math.min(preferredMenuHeight, menuBudget);
        final previewMaximumHeight = showPreview
            ? math.max(
                previewMinimumHeight,
                availableHeight -
                    trayHeight -
                    trayGap -
                    menuHeight -
                    previewGap,
              )
            : 0.0;
        final previewInsetExtent = _messageActionPreviewInset * 2;
        final previewSize = showPreview
            ? () {
                final previewWidthRatio =
                    (math.min(_messageActionPreviewMaxWidth, availableWidth) -
                        previewInsetExtent) /
                    math.max(anchorRect.width, 1);
                final previewHeightRatio =
                    (previewMaximumHeight - previewInsetExtent) /
                    math.max(anchorRect.height, 1);
                final previewScale = math.min(
                  1.0,
                  math.max(
                    0.0,
                    math.min(previewWidthRatio, previewHeightRatio),
                  ),
                );
                return Size(
                  (anchorRect.width * previewScale) + previewInsetExtent,
                  (anchorRect.height * previewScale) + previewInsetExtent,
                );
              }()
            : Size.zero;
        final totalHeight =
            trayHeight + trayGap + previewSize.height + previewGap + menuHeight;
        final contentTop = math.max(safeTop, safeBottom - totalHeight);
        final surfaceLeft =
            safeLeft + ((availableWidth - math.max(trayWidth, menuWidth)) / 2);
        final trayRect = Rect.fromLTWH(
          surfaceLeft,
          contentTop,
          trayWidth,
          trayHeight,
        );
        final trayHostWidth = math.min(
          trayWidth + _reactionTraySpringAllowance,
          safeRight - surfaceLeft,
        );
        final previewRect = Rect.fromLTWH(
          surfaceLeft,
          trayRect.bottom + trayGap,
          previewSize.width,
          previewSize.height,
        );
        final menuRect = Rect.fromLTWH(
          surfaceLeft,
          previewRect.bottom + previewGap,
          menuWidth,
          menuHeight,
        );
        return _MessageActionPreviewVisibility(
          visible: showPreview,
          onChanged: onPreviewVisibilityChanged,
          child: Stack(
            children: [
              Positioned.fill(
                child: BackdropFilter(
                  key: const ValueKey('message-actions-backdrop-filter'),
                  filter: _messageActionBackdropFilter,
                  child: AnimatedBuilder(
                    animation: animation,
                    builder: (context, child) {
                      final opacity = Curves.easeOutCubic.transform(
                        animation.value,
                      );
                      return ColoredBox(
                        key: const ValueKey('message-actions-background'),
                        color: context.colors.inverseSurface.withValues(
                          alpha: _messageActionBackdropTintOpacity * opacity,
                        ),
                      );
                    },
                  ),
                ),
              ),
              Positioned.fill(
                child: GestureDetector(
                  key: const ValueKey('message-actions-backdrop'),
                  behavior: HitTestBehavior.opaque,
                  onTap: () => select(null),
                ),
              ),
              if (showPreview)
                Positioned.fromRect(
                  rect: previewRect,
                  child: AnimatedBuilder(
                    animation: animation,
                    child: RepaintBoundary(
                      child: _LiftedMessagePreview(
                        anchorSnapshot: anchorSnapshot,
                      ),
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
              if (showReactionTray)
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
                    popResult: _messageActionReactionSelection,
                    onSelected: (result, effect) => select(result, effect),
                  ),
                ),
              Positioned.fromRect(
                rect: menuRect,
                child: AnimatedBuilder(
                  animation: animation,
                  child: useIosNativeActionSurface
                      ? _IosNativeMessageActionSurface(
                          actions: actions,
                          rowHeight: menuLayout.rowHeight,
                          onSelected: selectAction,
                        )
                      : _MessageActionSurface(
                          actions: actions,
                          onSelected: selectAction,
                        ),
                  builder: (context, child) {
                    final appearance = const Interval(
                      0.08,
                      0.82,
                      curve: Curves.easeOutCubic,
                    ).transform(animation.value);
                    final fadedChild = Opacity(
                      opacity: appearance,
                      child: child,
                    );
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
          ),
        );
      },
    );
  }
}
