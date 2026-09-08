import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/clipboard_utils.dart';
import '../../shared/community/community_membership_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
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
class CommunityInvitePage extends ConsumerWidget {
  /// Creates the community invitation page.
  const CommunityInvitePage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final roleAsync = ref.watch(currentCommunityRoleProvider);
    return FrostedScaffold(
      useUtilitySurfaceTheme: true,
      appBar: const FrostedAppBar(
        centerTitle: true,
        title: Text('Invite to community'),
      ),
      body: roleAsync.when(
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
            ? _CommunityInviteBody(role: role!)
            : const _InvitePermissionDenied(),
      ),
    );
  }
}

class _CommunityInviteBody extends StatelessWidget {
  const _CommunityInviteBody({required this.role});

  final CommunityMemberRole role;

  @override
  Widget build(BuildContext context) {
    return ListView(
      padding: EdgeInsets.fromLTRB(
        0,
        frostedAppBarHeight(context) + Grid.xs,
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
      icon: LucideIcons.wifiOff,
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
      icon: LucideIcons.shieldAlert,
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
