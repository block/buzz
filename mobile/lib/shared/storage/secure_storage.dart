import 'package:flutter_secure_storage/flutter_secure_storage.dart';

/// Android caches the first configuration per namespace. Every production
/// client must use these options: a failed read must never delete a key.
const secureStorageAndroidOptions = AndroidOptions(resetOnError: false);

/// Preserve the existing community-storage namespace and iOS accessibility.
const communitySecureStorage = FlutterSecureStorage(
  aOptions: secureStorageAndroidOptions,
);

/// Account credentials and identities remain device-only on iOS.
const accountSecureStorage = FlutterSecureStorage(
  aOptions: secureStorageAndroidOptions,
  iOptions: IOSOptions(
    accessibility: KeychainAccessibility.first_unlock_this_device,
  ),
);
