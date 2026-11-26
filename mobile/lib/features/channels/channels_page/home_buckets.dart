part of '../channels_page.dart';

Widget _homeBucket(BuildContext context, Widget child) {
  final sliver = child is _HomePinsSection
      ? SliverPadding(
          padding: EdgeInsets.only(
            left: MessagesSectionOptions.messagesGutter(context) - Grid.xs,
            right: Grid.gutter - Grid.xs,
          ),
          sliver: child,
        )
      : child;
  if (!showMessagesLayoutBounds) return sliver;
  return DecoratedSliver(
    position: DecorationPosition.foreground,
    decoration: BoxDecoration(
      border: Border.all(color: const Color(0xFFB00000), width: 2),
    ),
    sliver: sliver,
  );
}

class _MessagesAlignmentGuides extends Decoration {
  const _MessagesAlignmentGuides({required this.inset});

  final double inset;

  @override
  BoxPainter createBoxPainter([VoidCallback? onChanged]) =>
      _MessagesAlignmentPainter(inset);
}

class _MessagesAlignmentPainter extends BoxPainter {
  _MessagesAlignmentPainter(this.inset);

  final double inset;

  @override
  void paint(Canvas canvas, Offset offset, ImageConfiguration configuration) {
    final size = configuration.size!;
    void drawGuide(double leading, Color color) {
      final x =
          offset.dx +
          (configuration.textDirection == TextDirection.rtl
              ? size.width - leading
              : leading);
      canvas.drawLine(
        Offset(x, offset.dy),
        Offset(x, offset.dy + size.height),
        Paint()
          ..color = color
          ..strokeWidth = 1,
      );
    }

    drawGuide(inset + _kChannelLeadingWidth / 2, const Color(0xFF1671FF));
    drawGuide(
      inset + _kChannelLeadingWidth + _kChannelLabelGap,
      const Color(0xFF00865A),
    );
  }
}

class _HomePinsSection extends ConsumerWidget {
  const _HomePinsSection({
    required this.pins,
    required this.currentPubkey,
    required this.onOpen,
    required this.onLongPress,
  });
  final List<Channel> pins;
  final String? currentPubkey;
  final ValueChanged<Channel> onOpen;
  final ValueChanged<Channel> onLongPress;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final names = ref.watch(identityNameSourcesProvider);
    String label(Channel channel) => resolveDmChannelDisplayLabel(
      channel,
      currentPubkey: currentPubkey,
      names: names,
    );
    return SliverToBoxAdapter(
      child: HomePins(
        channels: pins,
        label: label,
        onOpen: onOpen,
        onLongPress: onLongPress,
        viewer: currentPubkey,
      ),
    );
  }
}
