part of '../share_sheet.dart';

/// A row of what's being shared: photo, GIF and video thumbnails, and file
/// names.
class _AttachmentPreviews extends StatelessWidget {
  const _AttachmentPreviews({required this.attachments});

  final List<ShareAttachment> attachments;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: 72 + Grid.xs,
      child: ListView.separated(
        scrollDirection: Axis.horizontal,
        padding: const EdgeInsets.only(
          left: Grid.xs,
          right: Grid.xs,
          top: Grid.xs,
        ),
        itemCount: attachments.length,
        separatorBuilder: (_, _) => const SizedBox(width: Grid.xxs),
        itemBuilder: (context, index) =>
            _AttachmentPreview(attachment: attachments[index]),
      ),
    );
  }
}

class _AttachmentPreview extends StatelessWidget {
  const _AttachmentPreview({required this.attachment});

  final ShareAttachment attachment;

  @override
  Widget build(BuildContext context) {
    final image = switch (attachment.kind) {
      ShareAttachmentKind.image || ShareAttachmentKind.gif => attachment.path,
      ShareAttachmentKind.video => attachment.poster,
      ShareAttachmentKind.file => null,
    };
    final label = switch (attachment.kind) {
      ShareAttachmentKind.image => 'Shared photo',
      ShareAttachmentKind.gif => 'Shared GIF',
      ShareAttachmentKind.video => 'Shared video',
      ShareAttachmentKind.file => attachment.filename ?? 'Shared file',
    };
    if (image == null && attachment.kind == ShareAttachmentKind.file) {
      return Semantics(
        label: label,
        child: SizedBox(
          width: 72,
          child: ExcludeSemantics(
            child: Column(
              children: [
                const Icon(BuzzIcons.file, size: 40),
                Text(
                  label,
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  textAlign: TextAlign.center,
                  style: Theme.of(context).textTheme.labelSmall,
                ),
              ],
            ),
          ),
        ),
      );
    }
    return Semantics(
      label: label,
      image: true,
      child: ClipRRect(
        borderRadius: BorderRadius.circular(8),
        child: SizedBox.square(
          dimension: 72,
          child: Stack(
            fit: StackFit.expand,
            children: [
              if (image != null)
                Image.file(
                  File(image),
                  fit: BoxFit.cover,
                  cacheWidth: 216,
                  excludeFromSemantics: true,
                )
              else
                ColoredBox(
                  color: Theme.of(context).colorScheme.surfaceContainerHighest,
                ),
              if (attachment.kind == ShareAttachmentKind.video)
                const Center(
                  child: Icon(BuzzIcons.play, color: Colors.white, size: 28),
                ),
            ],
          ),
        ),
      ),
    );
  }
}
