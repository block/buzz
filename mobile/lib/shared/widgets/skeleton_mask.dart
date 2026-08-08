part of 'skeleton.dart';

class _SkeletonMask extends InheritedWidget {
  const _SkeletonMask({required super.child});

  static bool isActive(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<_SkeletonMask>() != null;

  @override
  bool updateShouldNotify(_SkeletonMask oldWidget) => false;
}
