import 'package:flutter/material.dart';

import '../theme/theme.dart';

AppBar androidPageHeader(
  BuildContext context, {
  required String title,
  required Widget leading,
  List<Widget> actions = const [],
}) => AppBar(
  primary: false,
  automaticallyImplyLeading: false,
  leadingWidth: Grid.gutter + Grid.xl,
  leading: Align(alignment: AlignmentDirectional.centerEnd, child: leading),
  title: Text(title),
  actions: actions,
  actionsPadding: const EdgeInsetsDirectional.only(end: Grid.gutter),
);
