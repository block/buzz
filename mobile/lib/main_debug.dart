import 'app.dart';
import 'main.dart' as common;
import 'shared/push/debug_push_lease_bootstrap.dart';

void main() => common.runBuzzApp(const DebugPushLeaseBootstrap(child: App()));
