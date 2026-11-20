import 'package:buzz/shared/widgets/feedback_snackbar.dart';

import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:duo_kit/duo_kit.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';

import '../../shared/clipboard_utils.dart';
import '../../shared/relay/relay.dart';
import '../../shared/identity_archives/identity_archives.dart';
import '../../shared/identity_names/identity_names_provider.dart';
import 'dm_channel_labels.dart';
import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/sheet_action_section.dart';
import '../../shared/widgets/app_list.dart';
import '../../shared/widgets/app_list_card.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/buzz_action_tile.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';
import '../../shared/widgets/tabler_star_icon.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../../shared/widgets/duo_toolbar.dart';
import 'channel.dart';
import 'add_members_sheet.dart';
import 'channel_identity_names_provider.dart';
import 'channel_management_provider.dart';
import 'channel_mutes/channel_mutes_provider.dart';
import 'channel_sections/channel_sections_provider.dart';
import 'channel_stars/channel_stars_provider.dart';
import 'channels_provider.dart';
import 'manage_channel_sheet.dart';
import 'members_sheet.dart';
import 'unread_badge/observed_unread_event.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/read_state/read_state_provider.dart';
import '../activity/inbox_local_state_provider.dart';
import '../../shared/read_state/read_state_time.dart';
import '../../shared/widgets/covered_route_keyboard_insets.dart';

part 'channel_details_page.dart';
part 'native_channel_actions.dart';
part 'native_channel_sheet_app.dart';

/// Opens the mobile channel actions sheet and returns whether its parent page
/// should close after a successful lifecycle action.
Future<bool?> showChannelActionsSheet({
  required BuildContext context,
  required Channel channel,
  required bool isUnread,
  String? sectionId,
}) {
  final native = _usesNativeChannelSheet(context);
  Widget content(BuildContext context) => ChannelActionsSheet(
    channel: channel,
    isUnread: isUnread,
    sectionId: sectionId,
    nativePresentation: native,
  );
  if (native) return _showNativeChannelSheet(context, content);
  return _showChannelLongPressSheet(context: context, builder: content);
}

bool _usesNativeChannelSheet(BuildContext context) {
  final view = View.of(context);
  final size = view.physicalSize / view.devicePixelRatio;
  return defaultTargetPlatform == TargetPlatform.iOS &&
      (DuoToolbarScope.maybeOf(context)?.strip ??
              FoldGeometry.of(context).strip) !=
          null &&
      size.width > size.height &&
      !isCompactDuoWindow(context);
}

// Keep the result alive after retiring the invisible route for relay work.
class _NativeChannelSheetRoute extends PageRouteBuilder<bool> {
  _NativeChannelSheetRoute(WidgetBuilder builder)
    : super(
        opaque: false,
        barrierColor: null,
        transitionDuration: Duration.zero,
        reverseTransitionDuration: Duration.zero,
        pageBuilder: (context, _, _) => builder(context),
      );

  Future<bool?>? lifecycleResult;
}

Future<bool?> _showNativeChannelSheet(
  BuildContext context,
  WidgetBuilder builder,
) async {
  final route = _NativeChannelSheetRoute(builder);
  final result = await Navigator.of(context, rootNavigator: true).push(route);
  return route.lifecycleResult == null ? result : await route.lifecycleResult;
}

// Other postures retain the existing bottom-sheet presentation.
Future<bool?> _showChannelLongPressSheet({
  required BuildContext context,
  required WidgetBuilder builder,
}) {
  return showBuzzModalBottomSheet<bool>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    constraints: BoxConstraints(
      maxWidth: 640,
      maxHeight: MediaQuery.sizeOf(context).height * 0.7,
    ),
    builder: builder,
  );
}

/// Marks [channelId] read through the newest message known at the moment of
/// the call, its listed last message or a newer observed unread event, so a
/// message that arrived while a menu was open is covered and
/// one that arrives later stays unread. With no known message time it only
/// releases forced-unread flags.
void _markChannelReadNow(WidgetRef ref, String channelId) {
  final channels = ref.read(channelsProvider.notifier);
  var timestamp = dateTimeToUnixSeconds(
    channels.channelById(channelId)?.lastMessageAt,
  );
  for (final event
      in channels.observedUnreadEventsByChannel[channelId]?.values ??
          const <ObservedUnreadEvent>[]) {
    if (timestamp == null || event.createdAt > timestamp) {
      timestamp = event.createdAt;
    }
  }
  // Time 0 never advances a read marker, so it only releases the flags.
  ref
      .read(readStateProvider.notifier)
      .markContextRead(channelId, timestamp ?? 0, clearForcedMessages: true);
  if (timestamp != null) {
    ref.read(inboxLocalStateProvider.notifier).clearScope(channelId);
    channels.clearObservedUnreadCoveredByRead(channelId, timestamp);
  }
}

/// Mobile action sheet for channel-level read, organization, and lifecycle
/// operations.
class ChannelActionsSheet extends HookConsumerWidget {
  const ChannelActionsSheet({
    super.key,
    required this.channel,
    required this.isUnread,
    this.sectionId,
    this.nativePresentation = false,
  });

  final Channel channel;
  final bool isUnread;
  final bool nativePresentation;
  final String? sectionId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final displayedChannel = useState(channel);
    final currentChannel = displayedChannel.value;
    final isMuted =
        ref.watch(channelMutesProvider).store.channels[channel.id]?.muted ==
        true;
    // Desktop's unified workspaces are read-only here.
    final canFile = ref.watch(
      channelSectionsProvider.select((state) => state.editable),
    );
    final isStarred =
        !channel.isDm &&
        ref.watch(channelStarsProvider).store.channels[channel.id]?.starred ==
            true;
    final membersAsync = channel.isDm
        ? const AsyncValue<List<ChannelMember>>.data([])
        : ref.watch(channelMembersProvider(channel.id));
    final agentOwnersAsync = channel.isDm
        ? const AsyncValue<Map<String, String>>.data({})
        : ref.watch(agentOwnersProvider);
    final currentPubkey = ref.watch(currentPubkeyProvider)?.toLowerCase();
    final currentMember = membersAsync.value?.cast<ChannelMember?>().firstWhere(
      (member) => member?.pubkey.toLowerCase() == currentPubkey,
      orElse: () => null,
    );
    final ownsOwnerAgent =
        currentPubkey != null &&
        membersAsync.value?.any(
              (member) =>
                  member.isOwner &&
                  agentOwnersAsync.value?[member.pubkey.toLowerCase()]
                          ?.toLowerCase() ==
                      currentPubkey,
            ) ==
            true;
    final canManageLifecycle =
        currentMember?.isElevated == true || ownsOwnerAgent;
    final canArchive = !channel.isArchived && canManageLifecycle;
    final canUnarchive = channel.isArchived && canManageLifecycle;
    final canDelete =
        !channel.isArchived &&
        (currentMember?.isOwner == true || ownsOwnerAgent);
    final lifecycleCapabilitiesLoading =
        membersAsync.isLoading || agentOwnersAsync.isLoading;
    final lifecycleCapabilitiesUnavailable =
        membersAsync.hasError || agentOwnersAsync.hasError;

    void close() => Navigator.of(context).pop();

    final groups = <Widget>[
      if (!channel.isDm) ...[
        _ChannelQuickActionsRow(
          isStarred: isStarred,
          isUnread: isUnread,
          onToggleStar: () {
            close();
            final notifier = ref.read(channelStarsProvider.notifier);
            isStarred
                ? notifier.unstarChannel(channel.id)
                : notifier.starChannel(channel.id);
          },
          onToggleRead: () {
            close();
            if (isUnread) {
              _markChannelReadNow(ref, channel.id);
            } else {
              final marked = ref
                  .read(readStateProvider.notifier)
                  .markContextUnread(channel.id, channelId: channel.id);
              if (!marked) {
                ScaffoldMessenger.maybeOf(context)?.showFeedbackSnackBar(
                  const SnackBar(
                    content: Text(
                      'Couldn’t mark this channel unread. Try again.',
                    ),
                  ),
                  kind: ToastKind.error,
                );
              }
            }
          },
        ),
        const SizedBox(height: Grid.xs),
      ],
      SheetActionSection(
        children: [
          if (!channel.isDm && canFile)
            _ChannelSheetRow(
              leading: const Icon(BuzzIcons.folderInput),
              title: const Text('Move to section…'),
              onTap: () async {
                final pageContext = Navigator.of(
                  context,
                  rootNavigator: true,
                ).context;
                close();
                await _showMoveSectionSheet(
                  pageContext,
                  channel: channel,
                  sectionId: sectionId,
                );
              },
            ),
          _ChannelSheetRow(
            leading: Icon(isMuted ? BuzzIcons.bell : BuzzIcons.bellOff),
            title: Text(isMuted ? 'Unmute channel' : 'Mute channel'),
            onTap: () {
              close();
              final notifier = ref.read(channelMutesProvider.notifier);
              isMuted
                  ? notifier.unmuteChannel(channel.id)
                  : notifier.muteChannel(channel.id);
            },
          ),
          if (!channel.isDm)
            _ChannelSheetRow(
              leading: const Icon(BuzzIcons.settings),
              title: const Text('Manage channel'),
              onTap: () async {
                final shouldClose = await showManageChannelSheet(
                  context: context,
                  channel: currentChannel,
                  canEditDetails:
                      canManageLifecycle && !currentChannel.isArchived,
                  onChannelUpdated: (updated) =>
                      displayedChannel.value = updated,
                );
                if (shouldClose == true && context.mounted) {
                  Navigator.of(context).pop(true);
                }
              },
            ),
        ],
      ),
      SheetActionSection(
        children: [
          _ChannelSheetRow(
            leading: const Icon(BuzzIcons.copy),
            title: const Text('Copy channel name'),
            onTap: () {
              close();
              copyToClipboard(
                context,
                currentChannel.name,
                message: 'Channel name copied',
              );
            },
          ),
          _ChannelSheetRow(
            leading: const Icon(BuzzIcons.hash),
            title: const Text('Copy channel ID'),
            onTap: () {
              close();
              copyToClipboard(
                context,
                channel.id,
                message: 'Channel ID copied',
              );
            },
          ),
        ],
      ),
      if (!channel.isDm)
        SheetActionSection(
          children: [
            if (channel.isMember && !channel.isArchived)
              _ActionTile(
                icon: BuzzIcons.logOut,
                label: 'Leave channel',
                destructive: true,
                onTap: () => _confirmAndRun(
                  context,
                  ref,
                  title: 'Leave #${currentChannel.name}?',
                  body: 'You’ll stop receiving messages from this channel.',
                  confirmLabel: 'Leave',
                  action: () =>
                      ref.read(channelActionsProvider).leaveChannel(channel.id),
                ),
              ),
            if (lifecycleCapabilitiesLoading)
              const _ChannelSheetRow(
                enabled: false,
                leading: BuzzLoadingIndicator(
                  size: 20,
                  semanticLabel: 'Loading channel actions',
                ),
                title: Text('Loading channel actions…'),
              )
            else if (lifecycleCapabilitiesUnavailable)
              const _ChannelSheetRow(
                enabled: false,
                leading: Icon(BuzzIcons.triangleAlert),
                title: Text('Channel actions unavailable'),
              )
            else ...[
              if (canArchive)
                _ActionTile(
                  icon: BuzzIcons.archive,
                  label: 'Archive channel',
                  onTap: () => _confirmAndRun(
                    context,
                    ref,
                    title: 'Archive #${currentChannel.name}?',
                    body: 'The channel will become read-only.',
                    confirmLabel: 'Archive',
                    action: () => ref
                        .read(channelActionsProvider)
                        .archiveChannel(channel.id),
                  ),
                ),
              if (canUnarchive)
                _ActionTile(
                  icon: BuzzIcons.archiveRestore,
                  label: 'Unarchive channel',
                  onTap: () => _confirmAndRun(
                    context,
                    ref,
                    title: 'Unarchive #${currentChannel.name}?',
                    body: 'The channel will become active again.',
                    confirmLabel: 'Unarchive',
                    action: () => ref
                        .read(channelActionsProvider)
                        .unarchiveChannel(channel.id),
                  ),
                ),
              if (canDelete)
                _ActionTile(
                  icon: BuzzIcons.trash2,
                  label: 'Delete channel',
                  destructive: true,
                  onTap: () => _confirmAndRun(
                    context,
                    ref,
                    title: 'Delete #${currentChannel.name}?',
                    body:
                        'This permanently deletes the channel and cannot be undone.',
                    confirmLabel: 'Delete',
                    action: () => ref
                        .read(channelActionsProvider)
                        .deleteChannel(channel.id),
                  ),
                ),
            ],
          ],
        ),
    ];
    if (nativePresentation) return _NativeChannelActions(groups: groups);
    return SafeArea(
      top: false,
      child: IconTheme.merge(
        data: const IconThemeData(size: 22),
        child: SingleChildScrollView(
          padding: const EdgeInsets.fromLTRB(
            Grid.gutter,
            0,
            Grid.gutter,
            Grid.xs,
          ),
          child: Column(mainAxisSize: MainAxisSize.min, children: groups),
        ),
      ),
    );
  }
}

class _ChannelQuickActionsRow extends StatelessWidget {
  const _ChannelQuickActionsRow({
    required this.isStarred,
    required this.isUnread,
    required this.onToggleStar,
    required this.onToggleRead,
  });

  final bool isStarred;
  final bool isUnread;
  final VoidCallback onToggleStar;
  final VoidCallback onToggleRead;

  @override
  Widget build(BuildContext context) => Row(
    children: [
      Expanded(
        child: _ChannelQuickAction(
          icon: BuzzIcons.star,
          iconWidget: TablerStarIcon(
            color: context.colors.onSurface,
            filled: isStarred,
          ),
          label: isStarred ? 'Unstar' : 'Star',
          onTap: onToggleStar,
        ),
      ),
      const SizedBox(width: Grid.twelve),
      Expanded(
        child: _ChannelQuickAction(
          icon: isUnread ? BuzzIcons.checkCheck : BuzzIcons.circleDot,
          label: isUnread ? 'Mark Read' : 'Mark Unread',
          onTap: onToggleRead,
        ),
      ),
    ],
  );
}

class _ChannelQuickAction extends StatelessWidget {
  const _ChannelQuickAction({
    required this.icon,
    this.iconWidget,
    required this.label,
    required this.onTap,
  });

  final IconData icon;
  final Widget? iconWidget;
  final String label;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => GestureDetector(
    onTap: () {
      unawaited(HapticFeedback.lightImpact());
      onTap();
    },
    behavior: HitTestBehavior.opaque,
    child: Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Container(
          height: 68 + (Grid.xxs * 2),
          decoration: BoxDecoration(
            color: context.colors.surfaceContainerHighest,
            borderRadius: BorderRadius.circular(Radii.dialog),
          ),
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              iconWidget ??
                  Icon(icon, size: 22, color: context.colors.onSurface),
              const SizedBox(height: Grid.xxs),
              Text(
                label,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: context.textTheme.labelMedium?.copyWith(
                  color: context.colors.onSurface,
                ),
              ),
            ],
          ),
        ),
      ],
    ),
  );
}

class _ActionTile extends StatelessWidget {
  const _ActionTile({
    required this.icon,
    required this.label,
    required this.onTap,
    this.destructive = false,
  });

  final IconData icon;
  final String label;
  final FutureOr<void> Function()? onTap;
  final bool destructive;

  @override
  Widget build(BuildContext context) => ListTile(
    leading: Icon(icon, color: destructive ? context.colors.error : null),
    title: Text(
      label,
      style: destructive ? TextStyle(color: context.colors.error) : null,
    ),
    enabled: onTap != null,
    onTap: onTap == null
        ? null
        : () {
            unawaited(HapticFeedback.lightImpact());
            onTap!();
          },
  );
}

Future<void> _confirmAndRun(
  BuildContext sheetContext,
  WidgetRef ref, {
  required String title,
  required String body,
  required String confirmLabel,
  required Future<void> Function() action,
}) async {
  // The action can take a relay round trip. Close only the sheet or page that
  // offered it, and only if it is still on top: popping whatever is on top
  // would close Home if the user dismissed this route meanwhile, and routes
  // opened over it may still depend on its context.
  final ownerNavigator = Navigator.of(sheetContext);
  final ownerRoute = ModalRoute.of(sheetContext);
  final pageContext = Navigator.of(sheetContext, rootNavigator: true).context;
  final messenger = ScaffoldMessenger.maybeOf(sheetContext);
  final confirmed = await showBuzzDialog<bool>(
    context: pageContext,
    builder: (dialogContext) => AlertDialog(
      title: Text(title),
      content: Text(body),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(dialogContext).pop(false),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: () => Navigator.of(dialogContext).pop(true),
          child: Text(confirmLabel),
        ),
      ],
    ),
  );
  if (confirmed != true || !sheetContext.mounted) return;
  Completer<bool?>? detachedResult;
  try {
    // Resolve provider-backed dependencies before disposing their owner.
    final pending = action();
    if (ownerRoute is _NativeChannelSheetRoute && ownerNavigator.mounted) {
      detachedResult = Completer<bool?>();
      ownerRoute.lifecycleResult = detachedResult.future;
      ownerNavigator.removeRoute(ownerRoute);
    }
    await pending;
    detachedResult?.complete(true);
    if (ownerNavigator.mounted && ownerRoute?.isCurrent == true) {
      ownerNavigator.pop(true);
    }
  } catch (error) {
    detachedResult?.complete(null);
    if (messenger == null || !messenger.mounted) return;
    messenger.showFeedbackSnackBar(
      SnackBar(
        content: Text(
          'Couldn’t ${confirmLabel.toLowerCase()} channel. Try again.',
        ),
      ),
      kind: ToastKind.error,
    );
  }
}

Future<void> _showMoveSectionSheet(
  BuildContext context, {
  required Channel channel,
  required String? sectionId,
}) async {
  // The actions sheet that opened this picker is already closing, and its
  // WidgetRef is disposed with it. Use the page's container throughout.
  final container = ProviderScope.containerOf(context, listen: false);
  // A notification can switch communities while the picker or name dialog is
  // open. Both close then, and only the community that opened them is edited.
  final config = container.read(relayConfigProvider);
  bool inScope() {
    final current = container.read(relayConfigProvider);
    return current.storedOrigin == config.storedOrigin &&
        current.nsec == config.nsec;
  }

  void edit(void Function(ChannelSectionsNotifier sections) change) {
    if (inScope()) change(container.read(channelSectionsProvider.notifier));
  }

  final sections = [...container.read(channelSectionsProvider).store.sections]
    ..sort((a, b) => a.order.compareTo(b.order));
  BuildContext? sheet;
  void close() {
    final open = sheet;
    if (open != null) _closeOwnRoute(open);
  }

  // Desktop's unified layout can lock legacy filing while this is open.
  final lock = container.listen(
    channelSectionsProvider.select((state) => state.editable),
    (_, editable) {
      if (!editable) close();
    },
  );
  final scope = container.listen(relayConfigProvider, (_, _) {
    if (!inScope()) close();
  });
  await showBuzzModalBottomSheet<void>(
    context: context,
    showDragHandle: true,
    builder: (sheetContext) {
      // A lock or switch that landed before this first frame found no sheet
      // to close.
      if (sheet == null &&
          (!container.read(channelSectionsProvider).editable || !inScope())) {
        WidgetsBinding.instance.addPostFrameCallback(
          (_) => _closeOwnRoute(sheetContext),
        );
      }
      sheet = sheetContext;
      return SafeArea(
        child: IconTheme.merge(
          data: const IconThemeData(size: 22),
          child: SingleChildScrollView(
            padding: const EdgeInsets.fromLTRB(
              Grid.gutter,
              0,
              Grid.gutter,
              Grid.xs,
            ),
            child: SheetActionSection(
              children: [
                for (final section in sections)
                  ListTile(
                    leading: const Icon(BuzzIcons.folder),
                    title: Text(section.name),
                    trailing: sectionId == section.id
                        ? Icon(
                            BuzzIcons.check,
                            color: sheetContext.colors.primary,
                          )
                        : null,
                    onTap: () {
                      Navigator.of(sheetContext).pop();
                      edit((s) => s.assignChannel(channel.id, section.id));
                    },
                  ),
                ListTile(
                  leading: const Icon(BuzzIcons.folderPlus),
                  title: const Text('New section…'),
                  onTap: () async {
                    Navigator.of(sheetContext).pop();
                    final name = await _showSectionNameDialog(
                      context,
                      inScope: inScope,
                    );
                    if (name == null || name.isEmpty) return;
                    edit((s) => s.createSection(name, channelId: channel.id));
                  },
                ),
                if (sectionId != null)
                  ListTile(
                    leading: const Icon(BuzzIcons.folderMinus),
                    title: const Text('Remove from section'),
                    onTap: () {
                      Navigator.of(sheetContext).pop();
                      edit((s) => s.unassignChannel(channel.id));
                    },
                  ),
              ],
            ),
          ),
        ),
      );
    },
  ).whenComplete(() {
    lock.close();
    scope.close();
  });
}

/// Closes the route that [context] belongs to, once. Later calls, such as a
/// second community switch, find it already leaving and never pop the page
/// underneath.
void _closeOwnRoute(BuildContext context) {
  if (!context.mounted) return;
  final route = ModalRoute.of(context);
  if (route == null || !route.isActive) return;
  final navigator = Navigator.of(context);
  if (route.isCurrent) {
    navigator.pop();
  } else {
    navigator.removeRoute(route);
  }
}

Future<String?> _showSectionNameDialog(
  BuildContext context, {
  required bool Function() inScope,
}) => showBuzzDialog<String>(
  context: context,
  builder: (_) => _SectionNameDialog(inScope: inScope),
);

/// Owns its controller so the field outlives the dialog's exit animation.
class _SectionNameDialog extends HookConsumerWidget {
  const _SectionNameDialog({required this.inScope});

  /// False once the community that opened the dialog is no longer active.
  final bool Function() inScope;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = useTextEditingController();
    ref.listen(relayConfigProvider, (_, _) {
      if (!inScope()) _closeOwnRoute(context);
    });
    // A switch that landed before this first frame found no dialog to close.
    useEffect(() {
      if (!inScope()) {
        WidgetsBinding.instance.addPostFrameCallback(
          (_) => _closeOwnRoute(context),
        );
      }
      return null;
    }, const []);
    return AlertDialog(
      title: const Text('New Section'),
      content: TextField(controller: controller, autofocus: true),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop(controller.text.trim()),
          child: const Text('Create'),
        ),
      ],
    );
  }
}
