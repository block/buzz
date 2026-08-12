part of '../invite_create_page.dart';

class _PersonInviteSection extends HookConsumerWidget {
  const _PersonInviteSection({required this.role});

  final CommunityMemberRole role;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final searchController = useTextEditingController();
    final searchFocusNode = useFocusNode();
    final isSearchEditing = useState(false);
    final candidatePubkey = useState<String?>(null);
    final selectedRole = useState(CommunityMemberRole.member);
    final isSubmitting = useState(false);
    final submitError = useState<String?>(null);

    final membership = ref.watch(communityMembershipProvider).asData?.value;
    final existingMemberPubkeys = {
      ...?membership?.pubkeys.map((pubkey) => pubkey.toLowerCase()),
    };
    final currentPubkey = ref.watch(myPubkeyProvider)?.toLowerCase();

    useEffect(() {
      void handleFocusChange() {
        isSearchEditing.value = searchFocusNode.hasFocus;
      }

      searchFocusNode.addListener(handleFocusChange);
      return () => searchFocusNode.removeListener(handleFocusChange);
    }, [searchFocusNode]);

    void selectCandidate(String pubkey) {
      final normalized = pubkey.toLowerCase();
      if (normalized == currentPubkey) {
        candidatePubkey.value = null;
        submitError.value = 'You cannot invite yourself.';
        return;
      }
      if (existingMemberPubkeys.contains(normalized)) {
        candidatePubkey.value = null;
        submitError.value = 'This person is already in the community.';
        return;
      }
      candidatePubkey.value = normalized;
      submitError.value = null;
    }

    void handleInput(String value) {
      submitError.value = null;
      final pubkey = parseCommunityInvitePubkey(value);
      if (pubkey == null) {
        candidatePubkey.value = null;
      } else {
        selectCandidate(pubkey);
      }
    }

    Future<void> submit(CommunityInviteDirectoryUser invitee) async {
      if (isSubmitting.value) return;
      isSubmitting.value = true;
      submitError.value = null;
      try {
        await ref
            .read(communityInviteActionsProvider)
            .inviteMembers(
              pubkeys: [invitee.pubkey],
              role: role == CommunityMemberRole.owner
                  ? selectedRole.value
                  : CommunityMemberRole.member,
            );
        if (!context.mounted) return;
        candidatePubkey.value = null;
        searchController.clear();
        ref.invalidate(communityMembershipProvider);
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('Invited to the community')),
        );
      } catch (error) {
        if (context.mounted) {
          submitError.value = _inviteErrorMessage(error);
        }
      } finally {
        if (context.mounted) {
          isSubmitting.value = false;
        }
      }
    }

    final pubkey = candidatePubkey.value;
    final profileAsync = pubkey == null
        ? null
        : ref.watch(communityInviteProfileProvider(pubkey));

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
          child: SizedBox(
            key: const Key('community-invite-recipient-field'),
            width: double.infinity,
            height: buzzSearchIdleFieldHeight,
            child: BuzzSearchField(
              fieldKey: const Key('community-invite-search'),
              controller: searchController,
              focusNode: searchFocusNode,
              hintText: 'Search npub',
              iconColor: navigationPrimaryForeground(context),
              inputColor: navigationPrimaryForeground(context),
              placeholderColor: navigationSecondaryForeground(context),
              surfaceColor: navigationSearchSurface(context),
              isEditing: isSearchEditing.value,
              reduceMotion: MediaQuery.disableAnimationsOf(context),
              motionDuration: const Duration(milliseconds: 160),
              autocorrect: false,
              enableSuggestions: false,
              enabled: !isSubmitting.value,
              textInputAction: TextInputAction.done,
              onTap: searchFocusNode.requestFocus,
              onChanged: handleInput,
              onSubmitted: (value) {
                final pubkey = parseCommunityInvitePubkey(value);
                if (pubkey == null && value.trim().isNotEmpty) {
                  submitError.value = 'Paste a valid npub.';
                } else if (pubkey != null) {
                  selectCandidate(pubkey);
                }
              },
            ),
          ),
        ),
        if (pubkey != null && profileAsync != null) ...[
          const SizedBox(height: Grid.xs),
          _InviteeResolutionCard(
            pubkey: pubkey,
            profileAsync: profileAsync,
            role: role,
            selectedRole: selectedRole.value,
            isSubmitting: isSubmitting.value,
            onRetry: () =>
                ref.invalidate(communityInviteProfileProvider(pubkey)),
            onRoleSelected: (value) => selectedRole.value = value,
            onInvite: submit,
          ),
        ],
        if (submitError.value case final error?) ...[
          const SizedBox(height: Grid.xxs),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
            child: Text(
              error,
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.error,
              ),
            ),
          ),
        ],
      ],
    );
  }
}

class _InviteeResolutionCard extends StatelessWidget {
  const _InviteeResolutionCard({
    required this.pubkey,
    required this.profileAsync,
    required this.role,
    required this.selectedRole,
    required this.isSubmitting,
    required this.onRetry,
    required this.onRoleSelected,
    required this.onInvite,
  });

  final String pubkey;
  final AsyncValue<CommunityInviteDirectoryUser?> profileAsync;
  final CommunityMemberRole role;
  final CommunityMemberRole selectedRole;
  final bool isSubmitting;
  final VoidCallback onRetry;
  final ValueChanged<CommunityMemberRole> onRoleSelected;
  final ValueChanged<CommunityInviteDirectoryUser> onInvite;

  @override
  Widget build(BuildContext context) {
    return profileAsync.when(
      loading: () => AppListCard(
        children: [
          AppListRowRaw(
            key: Key('community-invite-resolving-$pubkey'),
            leading: const SizedBox.square(
              dimension: 40,
              child: Center(
                child: BuzzLoadingIndicator(
                  size: 20,
                  semanticLabel: 'Resolving profile',
                ),
              ),
            ),
            title: const Text('Resolving profile'),
            subtitle: Text(shortCommunityInviteNpub(pubkey)),
          ),
        ],
      ),
      error: (_, _) => AppListCard(
        children: [
          AppListRow(
            key: Key('community-invite-resolution-error-$pubkey'),
            title: 'Could not resolve profile',
            subtitle: shortCommunityInviteNpub(pubkey),
            trailing: TextButton(
              onPressed: onRetry,
              child: const Text('Retry'),
            ),
          ),
        ],
      ),
      data: (invitee) {
        if (invitee == null) {
          return AppListCard(
            children: [
              AppListRow(
                key: Key('community-invite-unresolved-$pubkey'),
                title: 'Invalid npub',
                subtitle: shortCommunityInviteNpub(pubkey),
              ),
            ],
          );
        }
        return AppListCard(
          key: const Key('community-invite-person-card'),
          dividerIndent: Grid.xs,
          children: [
            AppListRow(
              key: Key('community-invite-resolved-$pubkey'),
              title: shortCommunityInviteNpub(invitee.pubkey),
              trailing: FilledButton(
                key: const Key('community-invite-submit'),
                onPressed: isSubmitting ? null : () => onInvite(invitee),
                child: isSubmitting
                    ? const BuzzLoadingIndicator(
                        size: 16,
                        semanticLabel: 'Inviting person',
                      )
                    : const Text('Invite'),
              ),
            ),
            if (role == CommunityMemberRole.owner)
              AppListRow(
                key: const Key('community-invite-role'),
                title: 'Role',
                value: switch (selectedRole) {
                  CommunityMemberRole.member => 'Member',
                  CommunityMemberRole.admin => 'Admin',
                  CommunityMemberRole.owner => 'Owner',
                },
                trailing: const _InviteRowChevron(),
                onTap: isSubmitting
                    ? null
                    : () => _showInviteOptionSheet<CommunityMemberRole>(
                        context: context,
                        title: 'Role',
                        value: selectedRole,
                        options: const [
                          CommunityInviteOption(
                            label: 'Member',
                            value: CommunityMemberRole.member,
                          ),
                          CommunityInviteOption(
                            label: 'Admin',
                            value: CommunityMemberRole.admin,
                          ),
                        ],
                        onSelected: onRoleSelected,
                      ),
              ),
          ],
        );
      },
    );
  }
}
