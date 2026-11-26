import '../channels_page/filters.dart' show markMessagesRow;
import '../../../shared/widgets/pin_shortcut_grid.dart';

import 'package:flutter_hooks/flutter_hooks.dart';
import '../../../shared/widgets/adaptive_conversation_pane.dart';
import '../../../shared/widgets/avatar_shape.dart';
import '../../profile/presence_cache_provider.dart';

import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/profile/user_cache_provider.dart';
import '../../../shared/theme/buzz_icons.dart';
import '../../../shared/theme/theme.dart';
import '../../../shared/widgets/avatar_image.dart';
import '../channel.dart';
import '../channels_provider.dart';
import '../../../shared/widgets/dm_header_cluster.dart';
import '../../../shared/widgets/dm_header_portrait.dart';
import '../dm_channel_labels.dart';

class HomePins extends StatelessWidget {
  const HomePins({
    required this.channels,
    required this.label,
    required this.onOpen,
    required this.onLongPress,
    required this.viewer,
    super.key,
  });
  final List<Channel> channels;
  final String Function(Channel) label;
  final ValueChanged<Channel> onOpen;
  final ValueChanged<Channel> onLongPress;
  final String? viewer;

  @override
  Widget build(BuildContext context) => PinShortcutGrid<Channel>(
    items: channels,
    idOf: (channel) => channel.id,
    label: label,
    onOpen: onOpen,
    onLongPress: onLongPress,
    artwork: (channel) => _PinArtwork(channel: channel, viewer: viewer),
    decorate: markMessagesRow,
  );
}

class _PinArtwork extends HookConsumerWidget {
  const _PinArtwork({required this.channel, required this.viewer});
  final Channel channel;
  final String? viewer;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final peers = channel.participantPubkeys
        .where((pk) => pk.toLowerCase() != viewer?.toLowerCase())
        .toList();
    final peer = channel.isDm && peers.length == 1
        ? peers.single.toLowerCase()
        : null;
    useEffect(() {
      if (peer == null) return null;
      return ref.read(presenceCacheProvider.notifier).track([peer]);
    }, [peer]);
    final status = peer == null
        ? null
        : ref.watch(presentedPresenceProvider.select((map) => map[peer]));
    final profile = peer == null
        ? null
        : ref.watch(userCacheProvider.select((profiles) => profiles[peer]));
    if (peer != null && profile == null) {
      ref.read(userCacheProvider.notifier).preload([peer]);
    }
    final members = channel.isDm
        ? null
        : ref.watch(pinMemberSnapshotProvider(channel.id));
    final portraits =
        (channel.isDm
                ? peers
                : members != null && members.isNotEmpty
                ? members.map((m) => m.pubkey)
                : channel.participantPubkeys)
            .map((key) => key.toLowerCase())
            .where((key) => key.isNotEmpty)
            .toSet()
            .take(4)
            .toList();
    final artwork = peer == null && portraits.isNotEmpty
        ? FittedBox(
            child: DmHeaderCluster(
              count: portraits.length,
              builder: (i, size) {
                return Consumer(
                  builder: (context, ref, _) {
                    final key = portraits[i];
                    final person = ref.watch(
                      userCacheProvider.select((users) => users[key]),
                    );
                    if (person == null) {
                      ref.read(userCacheProvider.notifier).preload([key]);
                    }
                    return DmHeaderPortrait(
                      imageUrl: person?.avatarUrl,
                      size: size,
                      standalone: portraits.length == 1,
                      isAgent:
                          person?.isAgent == true ||
                          (members?.any(
                                (m) => m.pubkey.toLowerCase() == key && m.isBot,
                              ) ??
                              false),
                      initial: person?.initial ?? key[0].toUpperCase(),
                    );
                  },
                );
              },
            ),
          )
        : AvatarImage(
            imageUrl: profile?.avatarUrl,
            radius: DmHeaderCluster.diameter / 2,
            backgroundColor: context.colors.surfaceContainerHighest,
            isAgent: profile?.isAgent == true,
            fallback: channel.isDm
                ? Text(
                    peers.length > 1
                        ? '${peers.length}'
                        : dmAvatarInitial(channel, currentPubkey: viewer),
                    style: context.textTheme.titleMedium,
                  )
                : Icon(
                    channel.isStream || channel.isForum
                        ? channel.visibility == 'private'
                              ? BuzzIcons.lock
                              : BuzzIcons.hash
                        : BuzzIcons.messageCircle,
                    size: 24,
                    color: context.colors.onSurfaceVariant,
                  ),
          );
    final selected =
        ConversationPaneScope.maybeOf(context)?.selectedId == channel.id;
    final agent = profile?.isAgent == true;
    return Center(
      child: SizedBox.square(
        dimension: DmHeaderCluster.diameter,
        child: Stack(
          clipBehavior: Clip.none,
          alignment: Alignment.center,
          children: [
            if (selected)
              Positioned.fill(
                left: -4,
                top: -4,
                right: -4,
                bottom: -4,
                child: ClipPath(
                  clipper: AvatarRingClipper(isAgent: agent, strokeWidth: 3),
                  child: ColoredBox(color: context.colors.primaryContainer),
                ),
              ),
            if (status == null)
              artwork
            else
              ClipPath(clipper: _PinStatusClipper(agent), child: artwork),
            if (status != null)
              Positioned(
                right: DmHeaderCluster.diameter * .025,
                bottom: DmHeaderCluster.diameter * .025,
                child: Container(
                  width: DmHeaderCluster.diameter / 4,
                  height: DmHeaderCluster.diameter / 4,
                  decoration: ShapeDecoration(
                    shape: avatarShape(isAgent: agent),
                    color: switch (status) {
                      'online' => context.appColors.success,
                      'away' => context.appColors.warning,
                      _ => context.colors.outline,
                    },
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

/// Desktop's 40-point avatar-status masks, including the tangent joins.
class _PinStatusClipper extends CustomClipper<Path> {
  const _PinStatusClipper(this.agent);
  final bool agent;
  @override
  Path getClip(Size size) {
    final path = agent
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
    return path.transform(
      Matrix4.diagonal3Values(size.width / 40, size.height / 40, 1).storage,
    );
  }

  @override
  bool shouldReclip(_PinStatusClipper old) => old.agent != agent;
}
