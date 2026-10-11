part of '../channels_page.dart';

class _ChannelsSkeleton extends StatelessWidget {
  final List<Channel>? channels;
  final double topInset;

  const _ChannelsSkeleton({required this.channels, required this.topInset});

  @override
  Widget build(BuildContext context) {
    final visibleChannels =
        channels
            ?.where((channel) => channel.isMember && !channel.isArchived)
            .take(8)
            .toList() ??
        const <Channel>[];
    final widths = visibleChannels
        .map(
          (channel) => min(
            240,
            max(
              88,
              24 +
                  (resolveDmChannelDisplayLabel(
                        channel,
                        currentPubkey: null,
                      ).length *
                      8),
            ),
          ).toDouble(),
        )
        .toList();
    const fallbackWidths = <double>[136, 184, 112, 160, 208, 128];
    if (widths.isEmpty) widths.addAll(fallbackWidths);

    return ListView.builder(
      cacheExtent: 0,
      physics: const NeverScrollableScrollPhysics(),
      padding: EdgeInsets.fromLTRB(
        Grid.gutter,
        topInset + Grid.twelve,
        Grid.gutter,
        80,
      ),
      itemBuilder: (_, section) => Padding(
        padding: const EdgeInsets.only(bottom: Grid.xs),
        child: _ChannelSkeletonSection(
          widths: List.generate(
            4,
            (row) => widths[(section * 4 + row) % widths.length],
          ),
        ),
      ),
    );
  }
}

class _ChannelSkeletonSection extends StatelessWidget {
  final List<double> widths;

  const _ChannelSkeletonSection({required this.widths});

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        const Padding(
          padding: EdgeInsets.symmetric(vertical: Grid.half),
          child: Row(
            children: [
              SizedBox(
                width: _kChannelLeadingWidth,
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: SkeletonBar(width: 18, height: 18),
                ),
              ),
              SizedBox(width: Grid.xxs),
              SkeletonBar(
                key: Key('channels-skeleton-section-label'),
                width: 88,
                height: 16,
              ),
            ],
          ),
        ),
        for (var index = 0; index < widths.length; index++)
          Padding(
            padding: const EdgeInsets.symmetric(
              vertical: _kChannelRowVerticalPadding,
            ),
            child: Row(
              children: [
                SizedBox(
                  width: _kChannelLeadingWidth,
                  child: Align(
                    alignment: Alignment.centerLeft,
                    child: SkeletonBar(
                      width: _kChannelIconSize,
                      height: _kChannelIconSize,
                      borderRadius: BorderRadius.circular(
                        index.isEven ? Radii.xs : Radii.full,
                      ),
                    ),
                  ),
                ),
                const SizedBox(width: _kChannelLabelGap),
                SkeletonBar(
                  key: Key('channels-skeleton-row-label-$index'),
                  width: widths[index],
                  height: 16,
                ),
              ],
            ),
          ),
      ],
    );
  }
}
