import 'package:buzz/shared/widgets/feedback_snackbar.dart';
import 'dart:async';

import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/deeplink/deep_link.dart';
import '../../shared/deeplink/pending_deep_link_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/share_inbox/share_inbox_provider.dart';
import '../../shared/share_inbox/shared_payload.dart';
import '../invites/invite_join_provider.dart';
import '../invites/invite_join_sheet.dart';
import 'channel.dart';
import 'channel_detail_page.dart';
import 'channels_provider.dart';
import 'share_target_sheet.dart';
import 'share_targets_sync.dart';

/// Routes pending `buzz://message` deep links into the channel view.
///
/// Wraps the authenticated home subtree. Whenever a parsed link is parked in
/// [pendingDeepLinkProvider] and the channel list is available, this pushes
/// the target [ChannelDetailPage] on the enclosing [Navigator]. Links are
/// held (not dropped) while channels are still loading, so cold-start links
/// dispatch as soon as the first channel fetch completes.
typedef DeepLinkDestinationBuilder =
    Widget Function(Channel channel, BuzzDeepLink link);

class DeepLinkDispatcher extends ConsumerStatefulWidget {
  final Widget child;
  final DeepLinkDestinationBuilder? destinationBuilder;
  final bool dispatchMessageLinks;

  /// App composition handles account onboarding without feature imports here.
  /// Return true only after the destination has retained the link durably.
  final Future<bool> Function(BuildContext, BuzzDeepLink)? onboardingHandler;

  const DeepLinkDispatcher({
    super.key,
    required this.child,
    this.destinationBuilder,
    this.dispatchMessageLinks = true,
    this.onboardingHandler,
  });

  @override
  ConsumerState<DeepLinkDispatcher> createState() => _DeepLinkDispatcherState();
}

class _DeepLinkDispatcherState extends ConsumerState<DeepLinkDispatcher> {
  bool _preparingInvite = false;
  bool _handlingOnboarding = false;
  bool _resolvingLink = false;
  bool _lookupInterrupted = false;
  ProviderSubscription<SessionState>? _session;
  bool _presentingShare = false;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      _maybeDispatch(ref.read(pendingDeepLinkProvider));
      if (widget.dispatchMessageLinks) {
        _maybePresentShare(ref.read(pendingSharedPayloadProvider));
      }
    });
  }

  @override
  void didUpdateWidget(DeepLinkDispatcher oldWidget) {
    super.didUpdateWidget(oldWidget);
    // A link or share parked while dispatch was off waits for this, not a new
    // event.
    if (widget.dispatchMessageLinks && !oldWidget.dispatchMessageLinks) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        _maybeDispatch(ref.read(pendingDeepLinkProvider));
        _maybePresentShare(ref.read(pendingSharedPayloadProvider));
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    // Re-evaluate dispatch when either a new link arrives or channels load.
    ref.listen<BuzzDeepLink?>(pendingDeepLinkProvider, (_, link) {
      _maybeDispatch(link);
    });
    if (widget.dispatchMessageLinks) {
      ref.listen<AsyncValue<List<Channel>>>(channelsProvider, (_, _) {
        _maybeDispatch(ref.read(pendingDeepLinkProvider));
        _maybePresentShare(ref.read(pendingSharedPayloadProvider));
      });
      ref.listen<SharedPayload?>(pendingSharedPayloadProvider, (_, payload) {
        _maybePresentShare(payload);
      });
      // Keep the Share Extension's channel list current while signed in.
      // Listen rather than watch: it changes as channels reorder.
      ref.listen(shareTargetsSyncProvider, (_, _) {});
    }

    return widget.child;
  }

  /// Share-sheet hand-offs carry no destination: pull the staged payload and
  /// let the user pick a channel. The link itself is consumed immediately; the
  /// payload notifier owns the rest of the flow.
  void _dispatchShareLink() {
    ref.read(pendingDeepLinkProvider.notifier).consume();
    unawaited(ref.read(pendingSharedPayloadProvider.notifier).sync());
  }

  void _maybePresentShare(SharedPayload? payload) {
    if (payload == null || _presentingShare || !mounted) return;
    _presentingShare = true;
    final navigatorContext = context;
    unawaited(() async {
      try {
        if (payload.hasTarget) {
          final handled = await _dispatchTargetedShare(payload);
          if (handled) return;
          if (!navigatorContext.mounted ||
              ref.read(pendingSharedPayloadProvider) != payload) {
            return;
          }
        }
        final channel = await showShareTargetSheet(navigatorContext, payload);
        if (channel == null) {
          // Dismissed: drop the payload so it does not resurface on resume.
          await ref
              .read(pendingSharedPayloadProvider.notifier)
              .dismiss(payload.id);
          return;
        }
        if (!navigatorContext.mounted) return;
        _pushChannel(channel, ChannelDeepLink(channelId: channel.id));
      } finally {
        _presentingShare = false;
        // A newer share that arrived while this one was presented was ignored
        // by the listener; present it now.
        if (mounted) {
          final next = ref.read(pendingSharedPayloadProvider);
          if (next != null && next != payload) _maybePresentShare(next);
        }
      }
    }());
  }

  /// A payload whose destination was picked in the extension goes straight
  /// into that channel. Returns false when the picker should take over
  /// instead (community gone, channel gone). A community switch remounts the
  /// subtree, whose dispatcher retries; channels still loading are retried by
  /// the channelsProvider listener.
  Future<bool> _dispatchTargetedShare(SharedPayload payload) async {
    final notifier = ref.read(pendingSharedPayloadProvider.notifier);
    final preparation = await notifier.prepareCommunity(payload.communityId!);
    if (!mounted || ref.read(pendingSharedPayloadProvider) != payload) {
      return true;
    }
    switch (preparation) {
      case SharedPayloadCommunityPreparation.switched:
        return true;
      case SharedPayloadCommunityPreparation.unavailable:
      case SharedPayloadCommunityPreparation.failed:
        return false;
      case SharedPayloadCommunityPreparation.ready:
        break;
    }
    final channels = ref.read(channelsProvider).asData?.value;
    if (channels == null) return true;
    final channel = channels
        .where(
          (candidate) =>
              candidate.id == payload.channelId &&
              candidate.isMember &&
              !candidate.isArchived,
        )
        .firstOrNull;
    if (channel == null) return false;
    seedComposerWithSharedPayload(ref, channel.id, payload);
    await notifier.consume(payload.id);
    if (!mounted) return true;
    _pushChannel(channel, ChannelDeepLink(channelId: channel.id));
    return true;
  }

  void _maybeDispatch(BuzzDeepLink? link) {
    if (link == null || _preparingInvite || _handlingOnboarding) return;
    if (widget.onboardingHandler != null &&
        (link is InviteDeepLink ||
            link is NostrBindDeepLink ||
            link is AddCommunityDeepLink)) {
      unawaited(_dispatchOnboarding(link));
      return;
    }
    if (link is InviteDeepLink) {
      _maybeDispatchInvite(link);
      return;
    }
    if (link is ShareDeepLink) {
      if (widget.dispatchMessageLinks) _dispatchShareLink();
      return;
    }
    if ((link is! MessageDeepLink && link is! ChannelDeepLink) ||
        !widget.dispatchMessageLinks) {
      return;
    }
    if (link is MessageDeepLink) {
      unawaited(_dispatchNotificationLink(link));
      return;
    }

    _dispatchNavigableLink(link);
  }

  Future<void> _dispatchOnboarding(BuzzDeepLink link) async {
    _handlingOnboarding = true;
    var consumed = false;
    try {
      final handled = await widget.onboardingHandler!(context, link);
      if (!mounted) return;
      if (handled) {
        if (ref.read(pendingDeepLinkProvider) == link) {
          ref.read(pendingDeepLinkProvider.notifier).consume();
          consumed = true;
        }
      } else if (link is InviteDeepLink) {
        _maybeDispatchInvite(link);
      } else if (ref.read(pendingDeepLinkProvider) == link) {
        // No fallback owns this link. It must not block later entry attempts.
        ref.read(pendingDeepLinkProvider.notifier).consume();
        consumed = true;
      }
    } catch (_) {
      if (mounted) {
        if (ref.read(pendingDeepLinkProvider) == link) {
          ref.read(pendingDeepLinkProvider.notifier).consume();
          consumed = true;
        }
        ScaffoldMessenger.maybeOf(context)?.showFeedbackSnackBar(
          const SnackBar(
            content: Text('Couldn’t open setup. Reopen the link.'),
          ),
          kind: ToastKind.error,
        );
      }
    } finally {
      _handlingOnboarding = false;
      if (mounted && (consumed || ref.read(pendingDeepLinkProvider) != link)) {
        _maybeDispatch(ref.read(pendingDeepLinkProvider));
      }
    }
  }

  Future<void> _dispatchNotificationLink(MessageDeepLink link) async {
    final preparation = await ref
        .read(pendingDeepLinkProvider.notifier)
        .prepareCommunity(link);
    if (!mounted || ref.read(pendingDeepLinkProvider) != link) return;
    switch (preparation) {
      case DeepLinkCommunityPreparation.ready:
        _dispatchNavigableLink(link);
      case DeepLinkCommunityPreparation.switched:
        // The community-scoped app subtree remounts and consumes the parked
        // target after its channels load.
        return;
      case DeepLinkCommunityPreparation.unavailable:
        ref.read(pendingDeepLinkProvider.notifier).consume();
        ScaffoldMessenger.maybeOf(context)?.showFeedbackSnackBar(
          const SnackBar(content: Text('Community unavailable')),
          kind: ToastKind.error,
        );
      case DeepLinkCommunityPreparation.failed:
        ScaffoldMessenger.maybeOf(context)?.showFeedbackSnackBar(
          const SnackBar(content: Text('Couldn’t open community')),
          kind: ToastKind.error,
        );
    }
  }

  void _dispatchNavigableLink(BuzzDeepLink link) {
    if (_resolvingLink || !widget.dispatchMessageLinks) return;
    final channelId = switch (link) {
      MessageDeepLink(:final channelId) => channelId,
      ChannelDeepLink(:final channelId) => channelId,
      _ => throw StateError('unsupported navigable deep link: $link'),
    };
    final channels = ref.read(channelsProvider).asData?.value;
    // Channels not loaded yet — keep the link parked; the channelsProvider
    // listener re-attempts once data arrives.
    if (channels == null) return;

    // Cached channels stay usable while the session reconnects.
    final listed = channels.where((c) => c.id == channelId).firstOrNull;
    if (listed != null) {
      _pushChannel(listed, link);
      ref.read(pendingDeepLinkProvider.notifier).consume();
      return;
    }
    // Keep the link parked; the session listener re-attempts on connect.
    if (_watchSession().status != SessionStatus.connected) return;
    unawaited(_fetchAndPush(link, channelId));
  }

  /// An unlisted channel's lookup needs an authenticated session. A drop
  /// cancels a running lookup; a connect retries the parked link.
  SessionState _watchSession() {
    _session ??= ref.listenManual<SessionState>(relaySessionProvider, (
      previous,
      next,
    ) {
      if (next.status != SessionStatus.connected) {
        if (_resolvingLink) _lookupInterrupted = true;
        return;
      }
      if (previous?.status != SessionStatus.connected) {
        _maybeDispatch(ref.read(pendingDeepLinkProvider));
      }
    });
    return _session!.read();
  }

  Future<void> _fetchAndPush(BuzzDeepLink link, String channelId) async {
    _resolvingLink = true;
    _lookupInterrupted = false;
    Channel? channel;
    Object? failure;
    try {
      channel = await ref
          .read(channelsProvider.notifier)
          .fetchOpenChannel(channelId);
    } catch (error) {
      failure = error;
    } finally {
      _resolvingLink = false;
    }
    // Dispatch can turn off during the lookup (hosted setup is showing).
    // Keep the link parked; didUpdateWidget re-dispatches when it turns on.
    if (!mounted || !widget.dispatchMessageLinks) return;
    if (ref.read(pendingDeepLinkProvider) != link) {
      // The guard above held back whatever replaced this link meanwhile.
      _maybeDispatch(ref.read(pendingDeepLinkProvider));
      return;
    }
    if (failure != null) {
      final connected = _watchSession().status == SessionStatus.connected;
      if (!connected || _lookupInterrupted) {
        // The session dropped and cancelled the lookup. Keep the link: retry
        // now if it is already back, else the session listener retries.
        if (connected) _maybeDispatch(link);
        return;
      }
    }
    if (channel != null) {
      _pushChannel(channel, link);
      ref.read(pendingDeepLinkProvider.notifier).consume();
      return;
    }
    ref.read(pendingDeepLinkProvider.notifier).consume();
    if (failure != null) {
      debugPrint('deep-link: channel $channelId lookup failed: $failure');
      ScaffoldMessenger.maybeOf(context)?.showFeedbackSnackBar(
        const SnackBar(content: Text('Could not open the channel')),
        kind: ToastKind.error,
      );
    } else {
      debugPrint(
        'deep-link: channel $channelId not found in workspace; dropping link',
      );
      ScaffoldMessenger.maybeOf(context)?.showFeedbackSnackBar(
        const SnackBar(content: Text('Channel not found here')),
        kind: ToastKind.error,
      );
    }
  }

  void _pushChannel(Channel channel, BuzzDeepLink link) {
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) =>
            widget.destinationBuilder?.call(channel, link) ??
            ChannelDetailPage(
              channel: channel,
              initialMessageId: link is MessageDeepLink ? link.messageId : null,
              initialThreadRootId: link is MessageDeepLink
                  ? link.threadRootId
                  : null,
            ),
      ),
    );
  }

  void _maybeDispatchInvite(InviteDeepLink link) {
    if (_preparingInvite) return;
    _preparingInvite = true;
    final navigatorContext = context;
    final messenger = ScaffoldMessenger.maybeOf(context);
    Future.microtask(() async {
      var consumed = false;
      try {
        await ref.read(inviteJoinProvider.notifier).prepare(link);
        ref.read(pendingDeepLinkProvider.notifier).consume();
        consumed = true;
        if (!navigatorContext.mounted) return;
        final inviteState = ref.read(inviteJoinProvider);
        final status = inviteState.status;
        if (status == InviteJoinStatus.confirming ||
            inviteState.isStarterSetupRecovery) {
          final sheet = showInviteJoinSheet(navigatorContext);
          if (status == InviteJoinStatus.claiming &&
              inviteState.isStarterSetupRecovery) {
            unawaited(
              ref.read(inviteJoinProvider.notifier).startStarterSetupRecovery(),
            );
          }
          final shouldFocusStarter = await sheet;
          final focusChannelId = ref.read(inviteJoinProvider).focusChannelId;
          if (shouldFocusStarter == true &&
              focusChannelId != null &&
              ref.read(pendingDeepLinkProvider) == null &&
              navigatorContext.mounted) {
            final channels = ref.read(channelsProvider).asData?.value;
            final channel = channels
                ?.where((candidate) => candidate.id == focusChannelId)
                .firstOrNull;
            if (channel != null) {
              _pushChannel(channel, ChannelDeepLink(channelId: focusChannelId));
            }
          }
        } else if (status == InviteJoinStatus.switchedExisting) {
          messenger?.showFeedbackSnackBar(
            const SnackBar(content: Text('Community switched')),
            kind: ToastKind.generic,
          );
        }
      } catch (error) {
        debugPrint('deep-link: failed to prepare invite: $error');
        if (navigatorContext.mounted) {
          messenger?.showFeedbackSnackBar(
            const SnackBar(
              content: Text('Couldn’t open invite. Reopen the link.'),
            ),
            kind: ToastKind.error,
          );
        }
      } finally {
        _preparingInvite = false;
        if (mounted && consumed) {
          _maybeDispatch(ref.read(pendingDeepLinkProvider));
        }
      }
    });
  }
}
