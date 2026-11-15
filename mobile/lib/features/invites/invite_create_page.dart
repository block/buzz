import 'package:buzz/shared/widgets/feedback_snackbar.dart';
import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:duo_kit/duo_kit.dart';
import '../../shared/widgets/duo_toolbar.dart';
import '../../shared/widgets/ios_navigation_bar.dart';
import '../../shared/widgets/adaptive_conversation_pane.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';

import '../../shared/clipboard_utils.dart';
import '../../shared/community/community_membership_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/utils/string_utils.dart';
import '../../shared/widgets/app_list.dart';
import '../../shared/widgets/app_list_card.dart';
import '../../shared/widgets/buzz_action_tile.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/buzz_search_field.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/widgets/modal_presentation.dart';
import 'invite_create_provider.dart';

part 'invite_create_page/invite_link_section.dart';
part 'invite_create_page/person_invite_section.dart';

/// A page for authorized community members to create and send invitations.
class CommunityInvitePage extends HookConsumerWidget {
  /// Creates the community invitation page.
  const CommunityInvitePage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final roleAsync = ref.watch(currentCommunityRoleProvider);
    final bodyKey = useMemoized(GlobalKey.new);
    final optionsNavigator = useMemoized(() => GlobalKey<NavigatorState>());
    final scope = DuoToolbarScope.maybeOf(context);
    final strip = scope?.strip ?? FoldGeometry.of(context).strip;
    final split =
        defaultTargetPlatform == TargetPlatform.iOS &&
        strip != null &&
        usesConversationPanes(context, strip: strip);
    final form = roleAsync.when(
      loading: () => const Center(
        child: BuzzLoadingIndicator(
          size: 48,
          semanticLabel: 'Checking community permissions',
        ),
      ),
      error: (_, _) => _InvitePermissionError(
        onRetry: () => ref.invalidate(communityMembershipProvider),
      ),
      data: (role) => canManageCommunityInvites(role)
          ? _CommunityInviteBody(role: role!, split: split)
          : const _InvitePermissionDenied(),
    );
    final body = KeyedSubtree(
      key: bodyKey,
      child: LayoutBuilder(
        builder: (context, constraints) => MediaQuery(
          data: MediaQuery.of(context).copyWith(
            size: constraints.biggest,
            padding: split
                ? MediaQuery.paddingOf(context).copyWith(left: 0, right: 0)
                : null,
            viewPadding: split
                ? MediaQuery.viewPaddingOf(context).copyWith(left: 0, right: 0)
                : null,
          ),
          child: NavigatorPopHandler<void>(
            onPopWithResult: (_) => optionsNavigator.currentState!.pop(),
            child: Navigator(
              key: optionsNavigator,
              pages: [MaterialPage<void>(child: form)],
              onDidRemovePage: (_) {},
            ),
          ),
        ),
      ),
    );
    if (!split) {
      return FrostedScaffold(
        useUtilitySurfaceTheme: true,
        appBar: const FrostedAppBar(
          centerTitle: true,
          nativeLargeTitle: true,
          title: Text('Invite to community'),
        ),
        body: body,
      );
    }
    return Theme(
      data: utilitySurfaceThemeData(Theme.of(context)),
      child: DuoToolbarScope(
        strip: strip,
        active: scope?.active ?? true,
        child: Scaffold(
          body: Padding(
            padding: EdgeInsets.only(
              left: strip.side == FoldStripSide.left ? strip.width : 0,
              right: strip.side == FoldStripSide.right ? strip.width : 0,
            ),
            child: Row(
              children: [
                SizedBox(
                  width: 375.0.clamp(
                    0.0,
                    (MediaQuery.sizeOf(context).width - strip.width - 1) / 2,
                  ),
                  child: DuoToolbarPortal(
                    strip: strip,
                    active:
                        (scope?.active ?? true) &&
                        (ModalRoute.isCurrentOf(context) ?? true),
                    actions: [
                      if (Navigator.canPop(context))
                        IosNavigationAction(
                          label: 'Back',
                          symbol: 'chevron.backward',
                          onPressed: () {
                            final local = optionsNavigator.currentState;
                            if (local?.canPop() ?? false) {
                              local!.pop();
                            } else {
                              Navigator.of(context).maybePop();
                            }
                          },
                        ),
                    ],
                    child: const SizedBox.expand(
                      key: ValueKey('invite-title-pane'),
                      child: IosNavigationBar(
                        title: 'Invite to community',
                        standaloneLargeTitle: true,
                      ),
                    ),
                  ),
                ),
                VerticalDivider(
                  width: 1,
                  thickness: 1,
                  color: context.colors.outlineVariant,
                ),
                Expanded(
                  child: SafeArea(
                    left: false,
                    right: false,
                    child: SizedBox.expand(
                      key: const ValueKey('invite-controls-pane'),
                      child: body,
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

class _CommunityInviteBody extends StatelessWidget {
  const _CommunityInviteBody({required this.role, this.split = false});

  final bool split;

  final CommunityMemberRole role;

  @override
  Widget build(BuildContext context) {
    return ListView(
      padding: EdgeInsets.fromLTRB(
        0,
        split
            ? Grid.sm
            : frostedAppBarHeight(context, nativeLargeTitle: true) + Grid.xs,
        0,
        Grid.lg,
      ),
      children: [
        const _InviteLinkSection(),
        const SizedBox(height: Grid.sm),
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
          child: Row(
            children: [
              Expanded(child: Divider(color: context.colors.outlineVariant)),
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: Grid.xs),
                child: Text(
                  'Or use npub',
                  style: context.textTheme.bodySmall?.copyWith(
                    color: context.colors.onSurfaceVariant,
                  ),
                ),
              ),
              Expanded(child: Divider(color: context.colors.outlineVariant)),
            ],
          ),
        ),
        const SizedBox(height: Grid.sm),
        _PersonInviteSection(role: role),
      ],
    );
  }
}

class _InvitePermissionError extends StatelessWidget {
  const _InvitePermissionError({required this.onRetry});

  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) {
    return _InviteMessage(
      icon: BuzzIcons.wifiOff,
      title: 'Could not check permissions',
      body: 'Reconnect to this community and try again.',
      action: TextButton(onPressed: onRetry, child: const Text('Retry')),
    );
  }
}

class _InvitePermissionDenied extends StatelessWidget {
  const _InvitePermissionDenied();

  @override
  Widget build(BuildContext context) {
    return const _InviteMessage(
      icon: BuzzIcons.shieldAlert,
      title: 'Invite access required',
      body: 'Only community owners and admins can invite people.',
    );
  }
}

class _InviteMessage extends StatelessWidget {
  const _InviteMessage({
    required this.icon,
    required this.title,
    required this.body,
    this.action,
  });

  final IconData icon;
  final String title;
  final String body;
  final Widget? action;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(Grid.gutter),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(icon, size: 36, color: context.colors.onSurfaceVariant),
            const SizedBox(height: Grid.xs),
            Text(title, style: context.textTheme.titleMedium),
            const SizedBox(height: Grid.xxs),
            Text(
              body,
              textAlign: TextAlign.center,
              style: context.textTheme.bodyMedium?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
            if (action != null) ...[const SizedBox(height: Grid.xs), action!],
          ],
        ),
      ),
    );
  }
}

String _inviteErrorMessage(Object error) =>
    error.toString().replaceFirst('Exception: ', '');
