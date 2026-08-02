import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:intl/intl.dart' as intl;

import 'app_localizations_en.dart';
import 'app_localizations_pt.dart';

// ignore_for_file: type=lint

/// Callers can lookup localized strings with an instance of AppLocalizations
/// returned by `AppLocalizations.of(context)`.
///
/// Applications need to include `AppLocalizations.delegate()` in their app's
/// `localizationDelegates` list, and the locales they support in the app's
/// `supportedLocales` list. For example:
///
/// ```dart
/// import 'l10n/app_localizations.dart';
///
/// return MaterialApp(
///   localizationsDelegates: AppLocalizations.localizationsDelegates,
///   supportedLocales: AppLocalizations.supportedLocales,
///   home: MyApplicationHome(),
/// );
/// ```
///
/// ## Update pubspec.yaml
///
/// Please make sure to update your pubspec.yaml to include the following
/// packages:
///
/// ```yaml
/// dependencies:
///   # Internationalization support.
///   flutter_localizations:
///     sdk: flutter
///   intl: any # Use the pinned version from flutter_localizations
///
///   # Rest of dependencies
/// ```
///
/// ## iOS Applications
///
/// iOS applications define key application metadata, including supported
/// locales, in an Info.plist file that is built into the application bundle.
/// To configure the locales supported by your app, you’ll need to edit this
/// file.
///
/// First, open your project’s ios/Runner.xcworkspace Xcode workspace file.
/// Then, in the Project Navigator, open the Info.plist file under the Runner
/// project’s Runner folder.
///
/// Next, select the Information Property List item, select Add Item from the
/// Editor menu, then select Localizations from the pop-up menu.
///
/// Select and expand the newly-created Localizations item then, for each
/// locale your application supports, add a new item and select the locale
/// you wish to add from the pop-up menu in the Value field. This list should
/// be consistent with the languages listed in the AppLocalizations.supportedLocales
/// property.
abstract class AppLocalizations {
  AppLocalizations(String locale)
    : localeName = intl.Intl.canonicalizedLocale(locale.toString());

  final String localeName;

  static AppLocalizations of(BuildContext context) {
    return Localizations.of<AppLocalizations>(context, AppLocalizations)!;
  }

  static const LocalizationsDelegate<AppLocalizations> delegate =
      _AppLocalizationsDelegate();

  /// A list of this localizations delegate along with the default localizations
  /// delegates.
  ///
  /// Returns a list of localizations delegates containing this delegate along with
  /// GlobalMaterialLocalizations.delegate, GlobalCupertinoLocalizations.delegate,
  /// and GlobalWidgetsLocalizations.delegate.
  ///
  /// Additional delegates can be added by appending to this list in
  /// MaterialApp. This list does not have to be used at all if a custom list
  /// of delegates is preferred or required.
  static const List<LocalizationsDelegate<dynamic>> localizationsDelegates =
      <LocalizationsDelegate<dynamic>>[
        delegate,
        GlobalMaterialLocalizations.delegate,
        GlobalCupertinoLocalizations.delegate,
        GlobalWidgetsLocalizations.delegate,
      ];

  /// A list of this localizations delegate's supported locales.
  static const List<Locale> supportedLocales = <Locale>[
    Locale('en'),
    Locale('pt'),
    Locale('pt', 'BR'),
  ];

  /// No description provided for @appTitle.
  ///
  /// In en, this message translates to:
  /// **'Buzz'**
  String get appTitle;

  /// No description provided for @settingsTitle.
  ///
  /// In en, this message translates to:
  /// **'Settings'**
  String get settingsTitle;

  /// No description provided for @styleSection.
  ///
  /// In en, this message translates to:
  /// **'Style'**
  String get styleSection;

  /// No description provided for @appearance.
  ///
  /// In en, this message translates to:
  /// **'Appearance'**
  String get appearance;

  /// No description provided for @theme.
  ///
  /// In en, this message translates to:
  /// **'Theme'**
  String get theme;

  /// No description provided for @accentColor.
  ///
  /// In en, this message translates to:
  /// **'Accent color'**
  String get accentColor;

  /// No description provided for @system.
  ///
  /// In en, this message translates to:
  /// **'System'**
  String get system;

  /// No description provided for @light.
  ///
  /// In en, this message translates to:
  /// **'Light'**
  String get light;

  /// No description provided for @dark.
  ///
  /// In en, this message translates to:
  /// **'Dark'**
  String get dark;

  /// No description provided for @language.
  ///
  /// In en, this message translates to:
  /// **'Language'**
  String get language;

  /// No description provided for @languageAndRegion.
  ///
  /// In en, this message translates to:
  /// **'Language and region'**
  String get languageAndRegion;

  /// No description provided for @languageDescription.
  ///
  /// In en, this message translates to:
  /// **'Choose the language used by Buzz on this device.'**
  String get languageDescription;

  /// No description provided for @portugueseBrazil.
  ///
  /// In en, this message translates to:
  /// **'Português (Brasil)'**
  String get portugueseBrazil;

  /// No description provided for @englishUnitedStates.
  ///
  /// In en, this message translates to:
  /// **'English (United States)'**
  String get englishUnitedStates;

  /// No description provided for @startingBuzz.
  ///
  /// In en, this message translates to:
  /// **'Starting Buzz'**
  String get startingBuzz;

  /// No description provided for @connection.
  ///
  /// In en, this message translates to:
  /// **'Connection'**
  String get connection;

  /// No description provided for @connectedTo.
  ///
  /// In en, this message translates to:
  /// **'Connected to'**
  String get connectedTo;

  /// No description provided for @identityPubkey.
  ///
  /// In en, this message translates to:
  /// **'Identity (pubkey)'**
  String get identityPubkey;

  /// No description provided for @pubkeyCopied.
  ///
  /// In en, this message translates to:
  /// **'Pubkey copied'**
  String get pubkeyCopied;

  /// No description provided for @removeCommunity.
  ///
  /// In en, this message translates to:
  /// **'Remove community'**
  String get removeCommunity;

  /// No description provided for @removeCommunityTitle.
  ///
  /// In en, this message translates to:
  /// **'Remove Community'**
  String get removeCommunityTitle;

  /// No description provided for @removeCommunityDescription.
  ///
  /// In en, this message translates to:
  /// **'This will disconnect this community. You will need to scan a new pairing code to reconnect.'**
  String get removeCommunityDescription;

  /// No description provided for @cancel.
  ///
  /// In en, this message translates to:
  /// **'Cancel'**
  String get cancel;

  /// No description provided for @remove.
  ///
  /// In en, this message translates to:
  /// **'Remove'**
  String get remove;

  /// No description provided for @addCommunity.
  ///
  /// In en, this message translates to:
  /// **'Add Community'**
  String get addCommunity;

  /// No description provided for @verifySecurityCode.
  ///
  /// In en, this message translates to:
  /// **'Verify Security Code'**
  String get verifySecurityCode;

  /// No description provided for @waitingDesktop.
  ///
  /// In en, this message translates to:
  /// **'Waiting for desktop to confirm...'**
  String get waitingDesktop;

  /// No description provided for @codeQuestion.
  ///
  /// In en, this message translates to:
  /// **'Does your desktop app show this code?'**
  String get codeQuestion;

  /// No description provided for @transferWarning.
  ///
  /// In en, this message translates to:
  /// **'You are about to transfer your Buzz identity to this device. Only confirm if you initiated this pairing from your desktop.'**
  String get transferWarning;

  /// No description provided for @connecting.
  ///
  /// In en, this message translates to:
  /// **'Connecting'**
  String get connecting;

  /// No description provided for @confirmedWaiting.
  ///
  /// In en, this message translates to:
  /// **'Confirmed — waiting for desktop'**
  String get confirmedWaiting;

  /// No description provided for @codesMatch.
  ///
  /// In en, this message translates to:
  /// **'Codes Match'**
  String get codesMatch;

  /// No description provided for @welcomeBuzz.
  ///
  /// In en, this message translates to:
  /// **'Welcome to Buzz'**
  String get welcomeBuzz;

  /// No description provided for @pairingInstructions.
  ///
  /// In en, this message translates to:
  /// **'Scan the QR code from your desktop app or paste a pairing code to connect.'**
  String get pairingInstructions;

  /// No description provided for @openingScanner.
  ///
  /// In en, this message translates to:
  /// **'Opening scanner'**
  String get openingScanner;

  /// No description provided for @scanQr.
  ///
  /// In en, this message translates to:
  /// **'Scan a QR code'**
  String get scanQr;

  /// No description provided for @hidePairingCode.
  ///
  /// In en, this message translates to:
  /// **'Hide pairing code'**
  String get hidePairingCode;

  /// No description provided for @usePairingCode.
  ///
  /// In en, this message translates to:
  /// **'Use pairing code'**
  String get usePairingCode;

  /// No description provided for @connect.
  ///
  /// In en, this message translates to:
  /// **'Connect'**
  String get connect;
}

class _AppLocalizationsDelegate
    extends LocalizationsDelegate<AppLocalizations> {
  const _AppLocalizationsDelegate();

  @override
  Future<AppLocalizations> load(Locale locale) {
    return SynchronousFuture<AppLocalizations>(lookupAppLocalizations(locale));
  }

  @override
  bool isSupported(Locale locale) =>
      <String>['en', 'pt'].contains(locale.languageCode);

  @override
  bool shouldReload(_AppLocalizationsDelegate old) => false;
}

AppLocalizations lookupAppLocalizations(Locale locale) {
  // Lookup logic when language+country codes are specified.
  switch (locale.languageCode) {
    case 'pt':
      {
        switch (locale.countryCode) {
          case 'BR':
            return AppLocalizationsPtBr();
        }
        break;
      }
  }

  // Lookup logic when only language code is specified.
  switch (locale.languageCode) {
    case 'en':
      return AppLocalizationsEn();
    case 'pt':
      return AppLocalizationsPt();
  }

  throw FlutterError(
    'AppLocalizations.delegate failed to load unsupported locale "$locale". This is likely '
    'an issue with the localizations generation tool. Please file an issue '
    'on GitHub with a reproducible sample app and the gen-l10n configuration '
    'that was used.',
  );
}
