import '../../shared/widgets/agent_artwork_panel.dart';
import 'profile_sheet_layout.dart';
import 'dart:async';

import 'package:flutter/material.dart';

import '../../shared/widgets/profile_action_chip.dart';
import '../../shared/widgets/app_list_card.dart';

import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:hooks_riverpod/misc.dart' show ProviderListenable;
import 'package:buzz/shared/theme/buzz_icons.dart';

import '../../shared/animated_avatar.dart';
import '../../shared/widgets/buzz_toast.dart';
import '../../shared/identity_names/identity_names.dart';
import '../../shared/identity_names/identity_names_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/utils/string_utils.dart';
import '../../shared/widgets/avatar_shape.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/verified_profiles/verified_profile_directory.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../../shared/widgets/progressive_animated_avatar.dart';
import '../channels/channel.dart';
import '../channels/channel_detail_page.dart';
import '../channels/channel_management_provider.dart';
import '../channels/message_content.dart';
import 'presence_cache_provider.dart';
import 'set_status_sheet.dart';
import 'profile_edit_page.dart';
import 'profile_details.dart';
import '../../shared/profile/user_cache_provider.dart';
import 'user_status_cache_provider.dart';

/// Show a user profile bottom sheet for the given [pubkey].
///
/// Completes after the profile and any conversation opened from it close.
///
/// The sheet names [pubkey] as the opening surface did: within the live
/// comparison context [names] (for example a channel's members). Without it,
/// the identity is compared only with itself.
Future<void> showUserProfileSheet(
  BuildContext context,
  String pubkey, {
  ProviderListenable<IdentityNames>? names,
  WidgetBuilder? contextualActions,
  ProfileDetails details = const ProfileDetails(),
}) async {
  final rootNavigator = Navigator.of(context, rootNavigator: true);
  ProfileLead? selectedLead;
  var editRequested = false;
  final channel = await showBuzzModalBottomSheet<Channel>(
    context: context,
    isScrollControlled: true,
    useSafeArea: true,
    showDragHandle: false,
    backgroundColor: utilitySurfaceThemeData(
      Theme.of(context),
    ).scaffoldBackgroundColor,
    showCloseButton: false,
    showRailCloseButton: true,
    detentCorners: true,
    builder: (sheetContext) => UserProfileSheet(
      pubkey: pubkey,
      sheetControls: true,
      details: details,
      onLead: (lead) {
        selectedLead = lead;
        Navigator.of(sheetContext).pop();
      },
      onEditProfile: () {
        editRequested = true;
        Navigator.of(sheetContext).pop();
      },
      names: names,
      contextualActions: contextualActions,
    ),
  );
  if (selectedLead case final lead?) {
    if (context.mounted) await _openLead(context, lead);
    return;
  }
  if (editRequested && rootNavigator.mounted) {
    await rootNavigator.push(
      MaterialPageRoute<void>(builder: (_) => const ProfileEditPage()),
    );
    return;
  }
  if (channel == null || !context.mounted) return;
  await Navigator.of(context).push(
    MaterialPageRoute<void>(
      builder: (_) => ChannelDetailPage(
        channel: channel,
        placement: ChannelRoutePlacement.ordinary,
      ),
    ),
  );
}

Future<void> _openLead(BuildContext context, ProfileLead lead) =>
    lead.pubkey != null
    ? showUserProfileSheet(context, lead.pubkey!)
    : showPreviewLead(context, lead);

class UserProfileSheet extends HookConsumerWidget {
  final String pubkey;
  final bool sheetControls;
  final VoidCallback? onEditProfile;
  final ValueChanged<ProfileLead>? onLead;
  final ProfileDetails details;

  /// Opens a conversation from an embedded profile without popping its pane.
  final ValueChanged<Channel>? onOpenChannel;

  /// Uses the available pane height instead of the modal sheet height cap.
  final bool embedded;

  /// Insets the profile controls while artwork fills the surrounding pane.
  final EdgeInsets contentPadding;

  /// Agent artwork treatment within the profile sheet.
  final bool agentPresentation;
  final String? initialAvatarUrl;
  final String? agentSubtitle;
  final Widget? agentSubtitleIcon;

  /// The opening surface's comparison context. The sheet watches it, so its
  /// title follows profile and context changes while it is open. The caller
  /// owns the context, so this sheet depends on no other feature's state.
  final ProviderListenable<IdentityNames>? names;

  /// Optional actions supplied by the opening surface, below the profile tiles.
  final WidgetBuilder? contextualActions;

  /// Optional action opposite the sheet close control.
  final Widget? headerLeading;

  /// Agent-specific content supplied by the owning surface.
  final WidgetBuilder? agentIdentity;
  final WidgetBuilder? agentSettings;

  /// Space before the primary profile actions.
  final double primaryActionsSpacing;

  /// Space between primary actions and caller-supplied actions.
  final double contextualActionsSpacing;

  const UserProfileSheet({
    super.key,
    required this.pubkey,
    this.sheetControls = false,
    this.onEditProfile,
    this.onLead,
    this.details = const ProfileDetails(),
    this.onOpenChannel,
    this.embedded = false,
    this.contentPadding = EdgeInsets.zero,
    this.agentPresentation = false,
    this.initialAvatarUrl,
    this.agentSubtitle,
    this.agentSubtitleIcon,
    this.names,
    this.contextualActions,
    this.headerLeading,
    this.agentIdentity,
    this.agentSettings,
    this.primaryActionsSpacing = Grid.xs + Grid.half,
    this.contextualActionsSpacing = Grid.xs,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final pk = pubkey.toLowerCase();
    final currentPubkey = ref.watch(currentPubkeyProvider);

    // Watch cached profile, presence, and user status.
    final profile =
        ref.watch(userCacheProvider.select((cache) => cache[pk])) ??
        ref.read(userCacheProvider.notifier).get(pk);
    final presenceMap = ref.watch(presentedPresenceProvider);
    final presence = presenceMap[pk];
    final statusCache = ref.watch(userStatusCacheProvider);
    final userStatus = statusCache[pk];

    // Fetch about from the user's kind:0 profile event.
    final aboutFuture = useMemoized(
      () => ref
          .read(relaySessionProvider.notifier)
          .fetchHistory(NostrFilters.profile(pk))
          .then((events) {
            if (events.isEmpty) return '';
            return ProfileData.fromEvent(events.first).about ?? '';
          })
          .catchError((_) => ''),
      [pk],
    );
    final aboutSnapshot = useFuture(aboutFuture);
    final about = aboutSnapshot.data ?? profile?.about ?? '';

    // Ensure presence and status are tracked.
    useEffect(() {
      final release = ref.read(presenceCacheProvider.notifier).track([pk]);
      ref.read(userStatusCacheProvider.notifier).track([pk]);
      ref.read(userCacheProvider.notifier).preload([pk]);
      return release;
    }, [pk]);
    const preview = bool.fromEnvironment('BUZZ_PROFILE_PREVIEW');
    final photoOpen = useState(false);
    final photoTransform = useTransformationController(keys: [photoOpen.value]);
    final swipeStart = useRef<Offset?>(null);
    final pointers = useMemoized(() => <int>{}, [photoOpen.value]);
    final portraitKey = useMemoized(GlobalKey.new);

    final isAgent = agentPresentation || profile?.isAgent == true;
    final detailsToShow = !preview
        ? details
        : ProfileDetails(
            personal: const [(label: 'Email', value: 'viewer@example.org')],
            team: const [(label: 'Team', value: 'Product design')],
            tenure: const (
              label: String.fromEnvironment(
                'BUZZ_PROFILE_TENURE_LABEL',
                defaultValue: 'Time at organization',
              ),
              value: '2 years',
            ),
            lead: previewDesignLead,
            city: 'London, United Kingdom',
            timeZone: 'Europe/London',
            coordinates: (latitude: 51.51, longitude: -0.13),
          );
    final copied = useState(false);
    final isOpeningDirectMessage = useState(false);

    // Canonical npub for the copy action; null when [pubkey] is not a valid
    // identity, in which case the copy tile is disabled — an invalid key is
    // never placed on the clipboard.
    final npub = fullNpub(pubkey);

    // Keep the opening surface's comparison context and qualified identity.
    final opener = names;
    final displayName =
        (opener != null ? ref.watch(opener) : watchIdentityNames(ref, {pk}))
            .labelFor(pk);
    final verified = ref.watch(
      verifiedProfilesProvider.select((all) => all[pk]),
    );
    final titleName = agentPresentation
        ? displayName
        : verified?.fullName ?? displayName;
    final secondaryName = titleName != displayName ? displayName : null;
    final avatarUrl = profile?.avatarUrl ?? initialAvatarUrl;
    final nip05 = profile?.nip05Handle;
    final initial =
        profile?.initial ?? (pubkey.isNotEmpty ? pubkey[0].toUpperCase() : '?');

    final copyGeneration = useRef(0);

    Future<void> copyPublicKey() async {
      if (npub == null) return;
      await Clipboard.setData(ClipboardData(text: npub));
      if (!context.mounted) return;
      final generation = ++copyGeneration.value;
      copied.value = true;
      showBuzzToast(context, 'Public key copied', kind: ToastKind.success);
      unawaited(
        Future<void>.delayed(const Duration(seconds: 2), () {
          if (context.mounted && copyGeneration.value == generation) {
            copied.value = false;
          }
        }),
      );
    }

    Future<void> openDirectMessage() async {
      if (isOpeningDirectMessage.value) return;
      isOpeningDirectMessage.value = true;
      try {
        final channel = await ref
            .read(channelActionsProvider)
            .openDm(pubkeys: [pk], waitForChannelList: false);
        if (!context.mounted) return;
        if (onOpenChannel != null) {
          onOpenChannel!(channel);
        } else {
          Navigator.of(context).pop(channel);
        }
      } catch (_) {
        // Silently fail — user tapped but DM open failed.
        if (context.mounted) isOpeningDirectMessage.value = false;
      }
    }

    final presenceLabel = switch (presence) {
      'online' => 'Online',
      'away' => 'Away',
      'offline' => 'Offline',
      _ => 'Unknown',
    };
    final hasBadge = ['online', 'away', 'offline'].contains(presence);
    final status = [
      if (userStatus != null) userStatus.emoji,
      if (userStatus != null) userStatus.text,
    ].where((part) => part.trim().isNotEmpty).join(' ');
    return ProfileSheetLayout(
      avatarUrl: avatarUrl,
      agent: isAgent,
      embedded: embedded,
      sheetControls: sheetControls,
      contentPadding: contentPadding,
      status: status.isEmpty ? null : _ThoughtBubble(text: status),
      hero: Column(
        children: [
          SizedBox.square(
            key: portraitKey,
            dimension: profilePortraitSize,
            child: Stack(
              clipBehavior: Clip.none,
              children: [
                ClipPath(
                  clipper: hasBadge ? _StatusNotch(isAgent) : null,
                  child: _ProfileAvatar(
                    avatarUrl: avatarUrl,
                    initial: initial,
                    isAgent: isAgent,
                    onEnlarge: avatarUrl == null
                        ? null
                        : () => photoOpen.value = true,
                  ),
                ),
                if (!hasBadge)
                  Semantics(
                    label: 'Presence: Unknown',
                    child: const SizedBox.shrink(),
                  ),
                if (hasBadge)
                  Positioned(
                    right: 2,
                    bottom: 2,
                    child: Semantics(
                      label: 'Presence: $presenceLabel',
                      child: Container(
                        width: 20,
                        height: 20,
                        decoration: ShapeDecoration(
                          shape: avatarShape(isAgent: isAgent),
                          color: presence == 'online'
                              ? context.appColors.success
                              : presence == 'away'
                              ? context.appColors.warning
                              : context.colors.outline,
                        ),
                      ),
                    ),
                  ),
                if (status.isNotEmpty) ...[
                  for (final dot in [
                    (left: 5.0, top: -6.0, size: 13.0),
                    (left: 15.0, top: 7.0, size: 7.0),
                  ])
                    Positioned(
                      left: dot.left,
                      top: dot.top,
                      child: IgnorePointer(
                        child: DecoratedBox(
                          decoration: BoxDecoration(
                            color: context.colors.surface,
                            shape: BoxShape.circle,
                          ),
                          child: SizedBox.square(dimension: dot.size),
                        ),
                      ),
                    ),
                ],
              ],
            ),
          ),
          const SizedBox(height: 14),
          Text(
            titleName,
            textAlign: TextAlign.center,
            style: context.textTheme.titleMedium,
          ),
          if (secondaryName != null)
            Text(secondaryName, style: context.textTheme.bodySmall),
          if (isAgent && about.isNotEmpty)
            Text(about, textAlign: TextAlign.center),
          if (!isAgent && detailsToShow.city != null)
            Padding(
              padding: const EdgeInsets.only(top: 2),
              child: Text(detailsToShow.city!),
            ),
          if (!isAgent && detailsToShow.timeZone != null)
            Padding(
              padding: const EdgeInsets.only(top: 4),
              child: ProfileHeaderClock(zoneName: detailsToShow.timeZone!),
            ),
          const SizedBox(height: Grid.xxs),
          Wrap(
            alignment: WrapAlignment.center,
            spacing: Grid.xxs,
            runSpacing: Grid.xxs,
            crossAxisAlignment: WrapCrossAlignment.center,
            children: [
              if (pk == currentPubkey && !isAgent) ...[
                ProfileActionChip(
                  label: 'Edit profile',
                  onPressed:
                      onEditProfile ??
                      () => Navigator.of(context, rootNavigator: true).push(
                        MaterialPageRoute<void>(
                          builder: (_) => const ProfileEditPage(),
                        ),
                      ),
                ),
                ProfileActionChip(
                  label: status.isEmpty ? 'Set a status' : 'Update status',
                  onPressed: () =>
                      showSetStatusSheet(context, currentStatus: userStatus),
                ),
              ] else if (pk != currentPubkey)
                ProfileActionChip(
                  label: isOpeningDirectMessage.value ? 'Opening…' : 'Message',
                  onPressed: isOpeningDirectMessage.value
                      ? null
                      : openDirectMessage,
                ),
              if (agentPresentation && contextualActions != null)
                Builder(builder: contextualActions!),
            ],
          ),
        ],
      ),
      body: Padding(
        padding: const EdgeInsets.fromLTRB(16, 8, 16, 16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (isAgent) ...[
              if (agentIdentity != null) Builder(builder: agentIdentity!),
              AppListCard(
                outlined: true,
                label: 'Public key',
                horizontalPadding: 0,
                children: [
                  Semantics(
                    button: true,
                    enabled: npub != null,
                    label: copied.value
                        ? 'Copied public key'
                        : 'Copy public key',
                    value: npub,
                    liveRegion: true,
                    child: Tooltip(
                      message: 'Copy public key',
                      excludeFromSemantics: true,
                      child: InkWell(
                        onTap: npub == null ? null : copyPublicKey,
                        child: Padding(
                          padding: const EdgeInsets.all(Grid.xs),
                          child: ExcludeSemantics(
                            child: Row(
                              children: [
                                Expanded(
                                  child: Text(
                                    npub ?? 'Public key unavailable',
                                    style: context.textTheme.bodySmall,
                                  ),
                                ),
                                const SizedBox(width: Grid.xs),
                                AnimatedSwitcher(
                                  duration:
                                      MediaQuery.disableAnimationsOf(context)
                                      ? Duration.zero
                                      : const Duration(milliseconds: 180),
                                  switchInCurve: Curves.easeOutCubic,
                                  switchOutCurve: Curves.easeInCubic,
                                  transitionBuilder: (child, animation) =>
                                      ScaleTransition(
                                        scale: animation,
                                        child: child,
                                      ),
                                  child: Icon(
                                    copied.value
                                        ? BuzzIcons.check
                                        : BuzzIcons.copy,
                                    key: ValueKey(copied.value),
                                    size: 18,
                                  ),
                                ),
                              ],
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                ],
              ),
              if (agentSettings != null) Builder(builder: agentSettings!),
            ] else
              ProfileDetailsCards(
                details: detailsToShow,
                about: preview && about.trim().isEmpty
                    ? 'Designing tools for collaboration.'
                    : about,
                username:
                    verified?.username ?? nip05 ?? (preview ? 'viewer' : null),
                avatarUrl: avatarUrl,
                publicKey: npub == null ? 'Unavailable' : shortPubkey(npub),
                onLead: onLead ?? (lead) => _openLead(context, lead),
                onCopyKey: npub == null ? null : copyPublicKey,
                copied: copied.value,
              ),
            if (!agentPresentation && contextualActions != null)
              Builder(builder: contextualActions!),
          ],
        ),
      ),
      portraitKey: portraitKey,
      photoOpen: photoOpen.value && avatarUrl != null,
      onPhotoBack: () => photoOpen.value = false,
      enlargedPhoto: avatarUrl == null
          ? null
          : AvatarClip(
              key: const ValueKey('enlarged-profile-portrait'),
              isAgent: isAgent,
              child: Listener(
                onPointerDown: (event) {
                  pointers.add(event.pointer);
                  swipeStart.value =
                      pointers.length == 1 &&
                          photoTransform.value.getMaxScaleOnAxis() <= 1.001
                      ? event.position
                      : null;
                },
                onPointerCancel: (event) {
                  pointers.remove(event.pointer);
                  swipeStart.value = null;
                },
                onPointerUp: (event) {
                  pointers.remove(event.pointer);
                  final start = swipeStart.value;
                  swipeStart.value = null;
                  if (start == null) return;
                  final delta = event.position - start;
                  if (delta.dy >= 40 && delta.dy > delta.dx.abs()) {
                    photoOpen.value = false;
                  }
                },
                child: InteractiveViewer(
                  transformationController: photoTransform,
                  child: Stack(
                    fit: StackFit.expand,
                    children: [
                      FittedBox(
                        child: SizedBox.square(
                          dimension: profilePortraitSize,
                          child: AvatarImageContent(
                            imageUrl:
                                parseAnimatedAvatarUrl(avatarUrl)?.posterUrl ??
                                avatarUrl,
                            preferThumbnail: true,
                            fallback: _AvatarFallback(initial: initial),
                          ),
                        ),
                      ),
                      _ProfileAvatar(
                        avatarUrl: avatarUrl,
                        initial: initial,
                        isAgent: isAgent,
                        transparentFallback: true,
                      ),
                    ],
                  ),
                ),
              ),
            ),
    );
  }
}

class _ProfileAvatar extends HookWidget {
  final String? avatarUrl;
  final String initial;
  final bool transparentFallback;
  final bool isAgent;

  final VoidCallback? onEnlarge;
  const _ProfileAvatar({
    this.onEnlarge,
    this.transparentFallback = false,
    required this.avatarUrl,
    required this.initial,
    required this.isAgent,
  });

  @override
  Widget build(BuildContext context) {
    final animatedAvatar = parseAnimatedAvatarUrl(avatarUrl);
    final stoppedAnimationUrl = useState<String?>(null);
    final isPlaying =
        animatedAvatar != null &&
        stoppedAnimationUrl.value != animatedAvatar.animationUrl;

    return AspectRatio(
      aspectRatio: 1,
      child: Semantics(
        button: animatedAvatar != null || onEnlarge != null,
        label: animatedAvatar != null
            ? (isPlaying ? 'Pause profile animation' : 'Play profile animation')
            : (onEnlarge != null ? 'Enlarge profile photo' : null),
        onLongPressHint: animatedAvatar != null && onEnlarge != null
            ? 'enlarge profile photo'
            : null,
        child: InkWell(
          key: const ValueKey('selected-profile-avatar'),
          onLongPress: animatedAvatar == null ? null : onEnlarge,
          onTap: animatedAvatar == null
              ? onEnlarge
              : () => stoppedAnimationUrl.value =
                    stoppedAnimationUrl.value == animatedAvatar.animationUrl
                    ? null
                    : animatedAvatar.animationUrl,
          child: ExcludeSemantics(
            child: LayoutBuilder(
              builder: (context, constraints) {
                final fallback = transparentFallback
                    ? const SizedBox.shrink()
                    : _AvatarFallback(initial: initial);
                final avatar = isPlaying
                    ? ProgressiveAnimatedAvatar(
                        key: ValueKey(animatedAvatar.animationUrl),
                        descriptor: animatedAvatar,
                        fallback: fallback,
                      )
                    : AvatarImageContent(
                        imageUrl: animatedAvatar?.posterUrl ?? avatarUrl,
                        preferThumbnail: true,
                        fallback: fallback,
                      );
                return AvatarClip(
                  isAgent: isAgent,
                  child: !isAgent || !usesInsetAgentArtwork(avatarUrl)
                      ? avatar
                      : Transform.scale(
                          scale: animatedAvatarArtworkScale,
                          child: avatar,
                        ),
                );
              },
            ),
          ),
        ),
      ),
    );
  }
}

class _AvatarFallback extends StatelessWidget {
  final String initial;

  const _AvatarFallback({required this.initial});

  @override
  Widget build(BuildContext context) {
    return Container(
      color: context.controlSurface,
      alignment: Alignment.center,
      child: Text(
        initial,
        style: context.textTheme.displayLarge?.copyWith(
          color: context.colors.onPrimaryContainer,
          fontWeight: FontWeight.w600,
        ),
      ),
    );
  }
}

class _ThoughtBubble extends StatelessWidget {
  const _ThoughtBubble({required this.text});
  final String text;

  @override
  Widget build(BuildContext context) {
    final color = context.colors.surface;
    return Padding(
      padding: const EdgeInsets.only(top: 12),
      child: Stack(
        clipBehavior: Clip.none,
        alignment: Alignment.center,
        children: [
          DecoratedBox(
            decoration: BoxDecoration(
              color: color,
              borderRadius: BorderRadius.circular(26),
              boxShadow: const [
                BoxShadow(
                  color: Color(0x18000000),
                  blurRadius: 8,
                  offset: Offset(0, 2),
                ),
              ],
            ),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 8),
              child: _StatusTicker(text: text),
            ),
          ),
        ],
      ),
    );
  }
}

class _StatusTicker extends HookWidget {
  const _StatusTicker({required this.text});
  final String text;

  @override
  Widget build(BuildContext context) {
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final line = text.replaceAll(RegExp(r'\s+'), ' ').trim();
    final scroll = useScrollController();
    final copy = useMemoized(GlobalKey.new);
    final distance = useState(0.0);
    final animation = useAnimationController();
    useEffect(() {
      if (distance.value == 0 || reducedMotion) return null;
      final travel = distance.value;
      void move() {
        if (scroll.hasClients) scroll.jumpTo(animation.value * travel);
      }

      animation.addListener(move);
      animation.repeat(
        period: Duration(milliseconds: (travel / 24 * 1000).round()),
      );
      return () {
        animation.removeListener(move);
        animation.stop();
        animation.value = 0;
        if (scroll.hasClients) scroll.jumpTo(0);
      };
    }, [distance.value, line, reducedMotion]);
    return ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: 196),
      child: LayoutBuilder(
        builder: (context, constraints) {
          if (reducedMotion) {
            return MessageContent(content: text, textAlign: TextAlign.center);
          }
          WidgetsBinding.instance.addPostFrameCallback((_) {
            final size = copy.currentContext?.size;
            if (!context.mounted || size == null) return;
            final next = size.width > constraints.maxWidth
                ? size.width + 48
                : 0.0;
            if (next != distance.value) distance.value = next;
          });
          final ticker = Semantics(
            label: text,
            excludeSemantics: true,
            child: SingleChildScrollView(
              controller: scroll,
              physics: const NeverScrollableScrollPhysics(),
              scrollDirection: Axis.horizontal,
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  MessageContent(key: copy, content: line),
                  if (distance.value > 0) ...[
                    const SizedBox(width: 48),
                    MessageContent(content: line),
                  ],
                ],
              ),
            ),
          );
          if (distance.value == 0) return ticker;
          return ShaderMask(
            blendMode: BlendMode.dstIn,
            shaderCallback: (bounds) {
              final edge = (8 / bounds.width).clamp(0.0, 0.5);
              return LinearGradient(
                colors: const [
                  Colors.transparent,
                  Colors.white,
                  Colors.white,
                  Colors.transparent,
                ],
                stops: [0, edge, 1 - edge, 1],
              ).createShader(bounds);
            },
            child: ticker,
          );
        },
      ),
    );
  }
}

// The two tangent joins come from desktop's 40-point avatar-status-cutout.svg.
class _StatusNotch extends CustomClipper<Path> {
  const _StatusNotch(this.isAgent);
  final bool isAgent;
  @override
  Path getClip(Size size) {
    final outline = isAgent
        ? (Path()
            ..moveTo(39.909, 24.629)
            ..cubicTo(39.837, 26.228, 37.956, 26.169, 36.360, 26.064)
            ..cubicTo(35.660, 26.018, 34.877, 26, 34, 26)
            ..cubicTo(27.12, 26, 26, 27.12, 26, 34)
            ..cubicTo(26, 34.877, 26.018, 35.660, 26.064, 36.360)
            ..cubicTo(26.169, 37.956, 26.228, 39.837, 24.629, 39.909)
            ..cubicTo(23.216, 39.973, 21.676, 40, 20, 40)
            ..cubicTo(2.8, 40, 0, 37.2, 0, 20)
            ..cubicTo(0, 2.8, 2.8, 0, 20, 0)
            ..cubicTo(37.2, 0, 40, 2.8, 40, 20)
            ..cubicTo(40, 21.676, 39.973, 23.216, 39.909, 24.629)
            ..close())
        : (Path()
            ..moveTo(39.271, 25.349)
            ..cubicTo(38.844, 26.891, 36.913, 26.379, 35.335, 26.112)
            ..arcToPoint(
              const Offset(26.112, 35.335),
              radius: const Radius.circular(8),
              clockwise: false,
            )
            ..cubicTo(26.379, 36.913, 26.891, 38.844, 25.349, 39.271)
            ..arcToPoint(
              const Offset(39.271, 25.349),
              radius: const Radius.circular(20),
              largeArc: true,
            )
            ..close());
    return outline.transform(
      Matrix4.diagonal3Values(size.width / 40, size.height / 40, 1).storage,
    );
  }

  @override
  bool shouldReclip(_StatusNotch oldClipper) => oldClipper.isAgent != isAgent;
}
