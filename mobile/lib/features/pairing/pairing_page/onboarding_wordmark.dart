import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

/// The website's animated noise/displacement treatment over the shared artwork.
class OnboardingWordmark extends StatefulWidget {
  /// Optional tint for dark appearances; null keeps the original ink.
  final Color? color;

  /// Creates the animated welcome wordmark.
  const OnboardingWordmark({super.key, this.color});

  @override
  State<OnboardingWordmark> createState() => _OnboardingWordmarkState();
}

class _OnboardingWordmarkState extends State<OnboardingWordmark>
    with SingleTickerProviderStateMixin, WidgetsBindingObserver {
  late final _animation = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 400),
  );
  ui.Image? _image;
  ui.FragmentShader? _shader;
  bool _loading = false;
  bool _motionEnabled = false;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _motionEnabled =
        !MediaQuery.disableAnimationsOf(context) &&
        TickerMode.valuesOf(context).enabled;
    if (_motionEnabled && !_loading) {
      _loading = true;
      _loadArtwork();
    }
    _syncAnimation();
  }

  Future<void> _loadArtwork() async {
    ui.Image? image;
    ui.FragmentShader? shader;
    try {
      final program = await ui.FragmentProgram.fromAsset(
        'shaders/onboarding_wordmark.frag',
      );
      final bytes = await rootBundle.load('assets/images/buzz-wordmark.png');
      final codec = await ui.instantiateImageCodec(
        bytes.buffer.asUint8List(bytes.offsetInBytes, bytes.lengthInBytes),
      );
      try {
        image = (await codec.getNextFrame()).image;
      } finally {
        codec.dispose();
      }
      shader = program.fragmentShader()..setImageSampler(0, image);
      if (!mounted) {
        image.dispose();
        shader.dispose();
        return;
      }
      setState(() {
        _image = image;
        _shader = shader;
      });
      _syncAnimation();
    } catch (error) {
      image?.dispose();
      shader?.dispose();
      // Keep the static artwork usable if a renderer cannot load the effect.
      debugPrint('Could not load onboarding wordmark effect: $error');
    }
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) => _syncAnimation();

  void _syncAnimation() {
    final lifecycle = WidgetsBinding.instance.lifecycleState;
    final active = lifecycle == null || lifecycle == AppLifecycleState.resumed;
    if (_motionEnabled && active && _shader != null) {
      if (!_animation.isAnimating) _animation.repeat();
    } else {
      _animation.stop();
    }
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _animation.dispose();
    _shader?.dispose();
    _image?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Semantics(
      label: 'Buzz',
      image: true,
      child: ExcludeSemantics(
        child: RepaintBoundary(
          child: _shader != null && _motionEnabled
              ? CustomPaint(
                  key: const Key('pairing-buzz-animated-texture'),
                  painter: _WordmarkPainter(
                    _shader!,
                    _animation,
                    widget.color ?? const Color(0xFF231E1E),
                  ),
                )
              : Image.asset(
                  'assets/images/buzz-wordmark.png',
                  fit: BoxFit.contain,
                  color: widget.color,
                  colorBlendMode: BlendMode.srcIn,
                  filterQuality: FilterQuality.medium,
                ),
        ),
      ),
    );
  }
}

class _WordmarkPainter extends CustomPainter {
  final ui.FragmentShader shader;
  final Animation<double> animation;
  final Color color;

  _WordmarkPainter(this.shader, this.animation, this.color)
    : super(repaint: animation);

  @override
  void paint(Canvas canvas, Size size) {
    shader
      ..setFloat(0, size.width)
      ..setFloat(1, size.height)
      ..setFloat(2, animation.value)
      ..setFloat(3, color.r)
      ..setFloat(4, color.g)
      ..setFloat(5, color.b)
      ..setFloat(6, color.a);
    canvas.drawRect(Offset.zero & size, Paint()..shader = shader);
  }

  @override
  bool shouldRepaint(_WordmarkPainter oldDelegate) =>
      shader != oldDelegate.shader || color != oldDelegate.color;
}
