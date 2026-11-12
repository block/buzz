part of '../compose_bar.dart';

class _PendingVideoThumbnail extends HookConsumerWidget {
  const _PendingVideoThumbnail({required this.file});
  final XFile file;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final service = ref.watch(mediaUploadServiceProvider);
    final poster = useFuture(
      useMemoized(() => service.prepareVideoPoster(file.path), [
        file.path,
        service,
      ]),
    );
    return Stack(
      fit: StackFit.expand,
      children: [
        const ColoredBox(color: Colors.black),
        if (poster.data case final bytes? when bytes.isNotEmpty)
          Image.memory(bytes, fit: BoxFit.cover, excludeFromSemantics: true),
        const Center(
          child: Icon(BuzzIcons.video, color: Colors.white, size: 24),
        ),
      ],
    );
  }
}
