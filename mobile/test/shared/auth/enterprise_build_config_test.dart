import 'dart:convert';

import 'package:buzz/shared/auth/enterprise_identity.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test(
    'compiled Dart define reaches the production native config parser',
    () async {
      // Run once normally (OSS), and once with --dart-define-from-file containing
      // synthetic seven-field JSON. An ordinary shell environment is not enough.
      const expectedManaged = bool.fromEnvironment('EXPECT_MANAGED');
      expect(enterpriseEnabled, expectedManaged);
      FlutterSecureStorage.setMockInitialValues({});
      final owner =
          EnterpriseIdentity(); // Actual default compile-time consumer.
      await owner.restore();
      expect(owner.authenticated, false);
      if (expectedManaged) {
        final config =
            jsonDecode(enterpriseBuildConfig) as Map<String, dynamic>;
        expect(config.length, 7);
        expect(config['redirectUri'], 'buzz://enterprise-login');
        expect(config.containsKey('environment'), false);
      } else {
        expect(enterpriseBuildConfig, isEmpty);
      }
    },
  );
}
