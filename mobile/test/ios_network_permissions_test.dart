import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

void main() {
  test('iOS permits self-hosted relays on the local network', () {
    final plist = File('ios/Runner/Info.plist').readAsStringSync();

    expect(plist, contains('<key>NSLocalNetworkUsageDescription</key>'));
    expect(plist, contains('<key>NSAllowsLocalNetworking</key>'));
  });
}
