import 'dart:ui';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme.dart';

part 'skeleton_bar.dart';
part 'skeleton_mask.dart';
part 'skeleton_reveal.dart';
part 'skeleton_shimmer.dart';

const _shimmerDuration = Duration(milliseconds: 2000);
const _skeletonOpacity = 0.1;
const _shimmerMinOpacity = 0.5;
const _revealDuration = Duration(milliseconds: 400);
const _revealBlur = 2.0;
