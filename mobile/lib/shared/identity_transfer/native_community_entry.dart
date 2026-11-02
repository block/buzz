import 'package:flutter/services.dart';

import '../hosted/account_identity_store.dart';
import '../hosted/device_identity_source.dart';
import '../utils/string_utils.dart';
import 'nsec_transfer.dart';

/// UIKit presentation with identity operations kept in the Dart service layer.
abstract final class NativeCommunityEntry {
  static const _channel = MethodChannel('buzz/community_entry');
  static bool _presenting = false;
  static int _session = 0;

  /// Returns null when native presentation is unavailable.
  static Future<bool?> show({
    required String name,
    required bool dark,
    required int primaryArgb,
    required List<DeviceIdentityChoice> choices,
    Future<Map<String, dynamic>> Function()? confirmInvite,
    required Future<void> Function(
      String? pubkey,
      String? nsec,
      bool create,
      bool Function() isCurrent,
    )
    connect,
  }) async {
    if (_presenting) return false;
    _presenting = true;
    final session = ++_session;
    var active = true;
    var generation = 0;
    var busy = false;
    String? previewed;
    _channel.setMethodCallHandler((call) async {
      final args = Map<String, dynamic>.from(call.arguments as Map);
      if (!active || args['session'] != session) return {'cancelled': true};
      if (call.method == 'cancel') {
        generation++;
        busy = false;
        previewed = null;
        return null;
      }
      if (busy) return {'cancelled': true};
      final attempt = ++generation;
      bool current() => active && generation == attempt;
      busy = true;
      try {
        switch (call.method) {
          case 'preview':
          case 'file':
            final text = call.method == 'file'
                ? await NsecTransfer.pickFile()
                : args['nsec'] as String?;
            if (!current() || text == null) return {'cancelled': true};
            previewed = NsecTransfer.preview(text);
            return {
              'npub': fullNpub(previewed!),
              if (call.method == 'file') 'nsec': text,
            };
          case 'join':
            if (confirmInvite != null) return await confirmInvite();
            final nsec = args['nsec'] as String?;
            final pubkey = args['pubkey'] as String?;
            final create = args['create'] == true;
            if (nsec != null) {
              if (previewed == null ||
                  NsecTransfer.preview(nsec) != previewed) {
                throw AccountIdentityFailure.invalidKey;
              }
            } else if (create
                ? choices.isNotEmpty
                : !choices.any((choice) => choice.pubkey == pubkey)) {
              return {'error': 'Choose an identity to continue.'};
            }
            await connect(pubkey, nsec, create, current);
            return current() ? {'joined': true} : {'cancelled': true};
          default:
            throw MissingPluginException();
        }
      } catch (error) {
        if (!current()) return {'cancelled': true};
        return {
          'error': switch (error) {
            AccountIdentityFailure.invalidKey =>
              'Enter a valid nsec private key.',
            AccountIdentityFailure.occupied ||
            AccountIdentityFailure.mismatch =>
              'This device already has a different identity. It has not been replaced.',
            _ => 'Could not finish. Try again. Your saved keys have been kept.',
          },
        };
      } finally {
        if (current()) busy = false;
      }
    });
    try {
      return await _channel.invokeMethod<bool>('present', {
            'session': session,
            'name': name,
            'dark': dark,
            'primaryArgb': primaryArgb,
            'invite': confirmInvite != null,
            'choices': [
              for (final choice in choices)
                {
                  'pubkey': choice.pubkey,
                  'label': choice.label,
                  'npub': fullNpub(choice.pubkey) ?? choice.pubkey,
                },
            ],
          }) ??
          false;
    } on MissingPluginException {
      return null;
    } on PlatformException catch (error) {
      // Scene transitions can temporarily leave UIKit without a presenter.
      if (error.code == 'presentation_failed') return null;
      rethrow;
    } finally {
      active = false;
      generation++;
      previewed = null;
      _channel.setMethodCallHandler(null);
      _presenting = false;
    }
  }
}
