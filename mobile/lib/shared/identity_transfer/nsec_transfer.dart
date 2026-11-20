import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:path_provider/path_provider.dart';
import 'package:share_plus/share_plus.dart';

import '../hosted/account_identity_store.dart';
import '../community/community.dart';
import '../security/sensitive_action_authorizer.dart';

/// Manual transfer boundaries. Exceptions contain no key material.
abstract final class NsecTransfer {
  /// Validates a pasted nsec and returns only its public identity.
  static String preview(String text) {
    if (text.length > 1024) throw AccountIdentityFailure.invalidKey;
    return AccountIdentityStore.publicKeyForNsec(text.trim());
  }

  /// Reads a single small UTF-8 nsec file. Cancellation returns null.
  static Future<String?> pickFile() async {
    final text = await const MethodChannel(
      'buzz/identity_file',
    ).invokeMethod<String>('import');
    if (text == null) return null;
    preview(text);
    return text.trim();
  }

  /// Authorizes the selected community identity immediately before export.
  static Future<bool> exportCommunity({
    required Community community,
    required Future<DeviceAuthResult> Function(bool biometricOnly) authorize,
    required bool Function() isCurrent,
    required Future<bool> Function(String nsec) share,
  }) async {
    final nsec = community.nsec;
    if (nsec == null || preview(nsec) != community.pubkey) {
      throw AccountIdentityFailure.mismatch;
    }
    final result = await authorize(
      community.sensitiveActionPolicy == SensitiveActionPolicy.enabled,
    );
    if (result != DeviceAuthResult.success || !isCurrent()) return false;
    return share(nsec);
  }

  /// Shares a plaintext backup, removing our temporary file on completion.
  /// The OS share target owns any copy the user chooses to save or send.
  static Future<bool> shareFile(String nsec, Rect origin) async {
    preview(nsec);
    if (defaultTargetPlatform == TargetPlatform.android) {
      return await const MethodChannel('buzz/identity_file').invokeMethod<bool>(
            'export',
            {'text': '${nsec.trim()}\n', 'name': 'buzz-identity.nsec'},
          ) ??
          false;
    }
    // iOS must apply protection when the secret is first written, not after it.
    final File file;
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      final path = await const MethodChannel(
        'buzz/identity_file',
      ).invokeMethod<String>('prepareExport', {'text': '${nsec.trim()}\n'});
      if (path == null) throw PlatformException(code: 'export_failed');
      file = File(path);
    } else {
      final directory = await (await getTemporaryDirectory()).createTemp(
        'buzz-identity-',
      );
      file = File('${directory.path}/buzz-identity.nsec');
    }
    try {
      if (defaultTargetPlatform != TargetPlatform.iOS) {
        await file.writeAsString('${nsec.trim()}\n', flush: true);
      }
      final result = await SharePlus.instance.share(
        ShareParams(
          files: [XFile(file.path, mimeType: 'text/plain')],
          sharePositionOrigin: origin,
        ),
      );
      return result.status == ShareResultStatus.success;
    } finally {
      await file.parent.delete(recursive: true);
    }
  }
}
