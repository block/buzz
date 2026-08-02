// ignore: unused_import
import 'package:intl/intl.dart' as intl;
import 'app_localizations.dart';

// ignore_for_file: type=lint

/// The translations for English (`en`).
class AppLocalizationsEn extends AppLocalizations {
  AppLocalizationsEn([String locale = 'en']) : super(locale);

  @override
  String get appTitle => 'Buzz';

  @override
  String get settingsTitle => 'Settings';

  @override
  String get styleSection => 'Style';

  @override
  String get appearance => 'Appearance';

  @override
  String get theme => 'Theme';

  @override
  String get accentColor => 'Accent color';

  @override
  String get system => 'System';

  @override
  String get light => 'Light';

  @override
  String get dark => 'Dark';

  @override
  String get language => 'Language';

  @override
  String get languageAndRegion => 'Language and region';

  @override
  String get languageDescription =>
      'Choose the language used by Buzz on this device.';

  @override
  String get portugueseBrazil => 'Português (Brasil)';

  @override
  String get englishUnitedStates => 'English (United States)';

  @override
  String get startingBuzz => 'Starting Buzz';

  @override
  String get connection => 'Connection';

  @override
  String get connectedTo => 'Connected to';

  @override
  String get identityPubkey => 'Identity (pubkey)';

  @override
  String get pubkeyCopied => 'Pubkey copied';

  @override
  String get removeCommunity => 'Remove community';

  @override
  String get removeCommunityTitle => 'Remove Community';

  @override
  String get removeCommunityDescription =>
      'This will disconnect this community. You will need to scan a new pairing code to reconnect.';

  @override
  String get cancel => 'Cancel';

  @override
  String get remove => 'Remove';

  @override
  String get addCommunity => 'Add Community';

  @override
  String get verifySecurityCode => 'Verify Security Code';

  @override
  String get waitingDesktop => 'Waiting for desktop to confirm...';

  @override
  String get codeQuestion => 'Does your desktop app show this code?';

  @override
  String get transferWarning =>
      'You are about to transfer your Buzz identity to this device. Only confirm if you initiated this pairing from your desktop.';

  @override
  String get connecting => 'Connecting';

  @override
  String get confirmedWaiting => 'Confirmed — waiting for desktop';

  @override
  String get codesMatch => 'Codes Match';

  @override
  String get welcomeBuzz => 'Welcome to Buzz';

  @override
  String get pairingInstructions =>
      'Scan the QR code from your desktop app or paste a pairing code to connect.';

  @override
  String get openingScanner => 'Opening scanner';

  @override
  String get scanQr => 'Scan a QR code';

  @override
  String get hidePairingCode => 'Hide pairing code';

  @override
  String get usePairingCode => 'Use pairing code';

  @override
  String get connect => 'Connect';
}
