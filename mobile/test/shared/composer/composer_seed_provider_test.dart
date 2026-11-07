import 'package:buzz/shared/composer/composer_seed_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  test('a planted seed is taken exactly once, per channel', () {
    final container = ProviderContainer();
    addTearDown(container.dispose);
    final notifier = container.read(composerSeedProvider.notifier);
    const image = ComposerSeedFile(path: '/tmp/a.png', mimeType: 'image/png');
    const doc = ComposerSeedFile(
      path: '/tmp/b.pdf',
      mimeType: 'application/pdf',
    );

    notifier.plant('c1', const [image]);
    notifier.plant('c2', const [doc]);

    expect(notifier.take('c1'), const [image]);
    expect(notifier.take('c1'), isEmpty);
    expect(notifier.take('c2'), const [doc]);
    expect(container.read(composerSeedProvider), isEmpty);
  });

  test('empty plants are ignored and kinds derive from mime type', () {
    final container = ProviderContainer();
    addTearDown(container.dispose);
    container.read(composerSeedProvider.notifier).plant('c1', const []);
    expect(container.read(composerSeedProvider), isEmpty);

    expect(
      const ComposerSeedFile(path: 'x', mimeType: 'image/jpeg').isImage,
      isTrue,
    );
    expect(
      const ComposerSeedFile(path: 'x', mimeType: 'video/mp4').isVideo,
      isTrue,
    );
    expect(const ComposerSeedFile(path: 'x').isImage, isFalse);
  });
}
