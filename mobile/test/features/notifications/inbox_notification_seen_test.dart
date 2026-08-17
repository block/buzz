import 'package:buzz/shared/notifications/inbox_notification_seen.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  group('inbox notification seen storage', () {
    test('read/write round-trips per pubkey', () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();

      expect(readStoredInboxNotificationSeenIds(prefs, 'AbC'), isEmpty);

      await writeStoredInboxNotificationSeenIds(prefs, 'AbC', ['a', 'b']);
      expect(readStoredInboxNotificationSeenIds(prefs, 'abc'), ['a', 'b']);
    });

    test('capSeenInboxNotificationIds keeps newest entries', () {
      final ids = {
        for (var i = 0; i < inboxNotificationSeenMaxItems + 5; i++) 'id-$i',
      };
      final capped = capSeenInboxNotificationIds(ids);
      expect(capped.length, inboxNotificationSeenMaxItems);
      expect(
        capped.contains('id-${inboxNotificationSeenMaxItems + 4}'),
        isTrue,
      );
      expect(capped.contains('id-0'), isFalse);
    });
  });
}
