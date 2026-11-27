import 'dart:async';
import 'dart:ui' as ui;
import 'package:flutter/rendering.dart';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:intl/intl.dart';
import 'package:timezone/data/latest.dart' as zones;
import 'package:timezone/timezone.dart' as tz;
import 'package:url_launcher/url_launcher.dart';

import '../../shared/theme/theme.dart';
import '../../shared/theme/buzz_icons.dart';
import 'package:flutter/services.dart';
import '../../shared/widgets/buzz_toast.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/agent_artwork_panel.dart' show teamPortraitSize;
import '../../shared/widgets/modal_presentation.dart';
import 'profile_sheet_layout.dart';
import 'profile_country_codes.dart';
import '../../shared/widgets/native_profile_sheet.dart';
import '../../shared/widgets/app_list.dart';
import '../../shared/widgets/app_list_card.dart';

part 'profile_local_sky.dart';

// Midpoint in zoom levels between the global and regional previews.
const _mapLongitudeSpan = 164.0;

final _profileBodyTextStyle = messageBodyTextStyle.copyWith(height: 22 / 17);

typedef ProfileField = ({String label, String value});

typedef ProfileLead = ({String name, String? avatarUrl, String? pubkey});
const ProfileLead previewDesignLead = (
  name: 'Alex Example',
  pubkey: null,
  avatarUrl: 'https://i.pravatar.cc/400?img=12',
);

Future<void> showPreviewLead(BuildContext context, ProfileLead lead) =>
    showBuzzModalBottomSheet<void>(
      context: context,
      backgroundColor: utilitySurfaceThemeData(
        Theme.of(context),
      ).scaffoldBackgroundColor,
      showCloseButton: false,
      showRailCloseButton: true,
      detentCorners: true,
      useSafeArea: true,
      isScrollControlled: true,
      builder: (_) => previewLeadContent(lead),
    );

Widget previewLeadContent(
  ProfileLead lead, {
  Widget? leading,
  bool initiallyExpanded = false,
}) => ProfileSheetLayout(
  leading: leading,
  initiallyExpanded: initiallyExpanded,
  avatarUrl: lead.avatarUrl,
  hero: Column(
    children: [
      SizedBox.square(
        dimension: 80,
        child: ClipOval(
          child: AvatarImageContent(
            imageUrl: lead.avatarUrl,
            fallback: const Icon(BuzzIcons.user),
          ),
        ),
      ),
      const SizedBox(height: 12),
      Builder(
        builder: (context) =>
            Text(lead.name, style: Theme.of(context).textTheme.titleMedium),
      ),
    ],
  ),
  body:
      lead == previewDesignLead &&
          const bool.fromEnvironment('BUZZ_PROFILE_PREVIEW')
      ? const Padding(
          padding: EdgeInsets.all(16),
          child: ProfileDetailsCards(
            details: ProfileDetails(
              team: [(label: 'Team', value: 'Product design')],
            ),
            avatarUrl: null,
            about: 'Design lead',
            username: null,
          ),
        )
      : const SizedBox.shrink(),
);

class ProfileDetails {
  const ProfileDetails({
    this.talkToMeAbout,
    this.personal = const [],
    this.team = const [],
    this.tenure,
    this.city,
    this.timeZone,
    this.coordinates,
    this.lead,
    this.teammates = const [],
  });
  final String? talkToMeAbout;
  final List<ProfileField> personal;
  final List<ProfileField> team;
  final ProfileField? tenure;
  final ProfileLead? lead;
  final List<ProfileLead> teammates;
  final String? city;
  final String? timeZone;
  final ({double latitude, double longitude})? coordinates;
}

class ProfileDetailsCards extends StatelessWidget {
  const ProfileDetailsCards({
    super.key,
    required this.details,
    required this.avatarUrl,
    required this.about,
    required this.username,
    this.horizontalInset = 0,
    this.ldap,
    this.publicKey,
    this.onCopyKey,
    this.onLead,
    this.copied = false,
  });
  final ProfileDetails details;
  final String? avatarUrl;
  final String about;
  final double horizontalInset;
  final String? username, ldap;
  final String? publicKey;
  final VoidCallback? onCopyKey;
  final ValueChanged<ProfileLead>? onLead;
  final bool copied;

  @override
  Widget build(BuildContext context) {
    final description = about.trim().isNotEmpty
        ? about.trim()
        : details.talkToMeAbout?.trim() ?? '';
    if (Theme.of(context).platform == TargetPlatform.iOS) {
      final personal = {
        for (final field in details.personal)
          if (field.value.trim().isNotEmpty) field.label: field,
      };
      final email = personal.remove('Email');
      final handle = ldap?.trim() ?? '';
      final identity = [
        ?personal.remove('Pronouns'),
        ?personal.remove('Pronunciation'),
        ...personal.values,
      ];
      final hasContact = email != null || handle.isNotEmpty;
      final hasAbout = description.isNotEmpty || identity.isNotEmpty;
      final hasRoster = details.lead != null || details.teammates.isNotEmpty;
      final teamName = details.team
          .where((f) => f.label == 'Team' && f.value.trim().isNotEmpty)
          .firstOrNull
          ?.value;
      final teamFacts = [
        if (teamName != null) (label: 'Group', value: teamName),
        ...details.team.where(
          (f) =>
              f.label != 'Team' &&
              f.label != 'Org' &&
              f.value.trim().isNotEmpty,
        ),
        ?details.tenure,
      ];
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children:
            [
                  if (hasAbout) ...[
                    const ProfileSectionLabel(
                      label: 'About',
                      title: true,
                      opaqueTitle: true,
                    ),
                    const SizedBox(height: 8),
                    if (description.isNotEmpty)
                      ProfileSectionLabel(
                        label: description,
                        messageTypography: true,
                        body: true,
                        textStyle: _profileBodyTextStyle,
                      ),
                    if (description.isNotEmpty && identity.isNotEmpty)
                      const SizedBox(height: 16),
                    if (identity.isNotEmpty) _FactRows(fields: identity),
                    const SizedBox(height: 30),
                  ],
                  if (hasContact) ...[
                    const ProfileSectionLabel(
                      label: 'Contact',
                      title: true,
                      opaqueTitle: true,
                    ),
                    const SizedBox(height: 12),
                    if (email != null)
                      _ArtworkFact(
                        label: email.label,
                        value: email.value,
                        onTap: () async {
                          await Clipboard.setData(
                            ClipboardData(text: email.value),
                          );
                          if (context.mounted) {
                            showBuzzToast(context, 'Email copied');
                          }
                        },
                      ),
                    if (email != null && handle.isNotEmpty)
                      const SizedBox(height: 16),
                    if (handle.isNotEmpty)
                      _ArtworkFact(
                        label: 'LDAP',
                        value: handle.startsWith('@') ? handle : '@$handle',
                      ),
                    const SizedBox(height: 30),
                  ],
                  if (hasRoster || teamFacts.isNotEmpty) ...[
                    const ProfileSectionLabel(
                      label: 'Team',
                      title: true,
                      opaqueTitle: true,
                    ),
                    const SizedBox(height: Grid.twelve),
                    if (teamFacts.isNotEmpty) _FactRows(fields: teamFacts),
                    if (hasRoster && teamFacts.isNotEmpty)
                      const SizedBox(height: 30),
                    if (hasRoster)
                      _TeamStrip(
                        key: NativeProfileSheet.maybeOf(context)?.rosterKey,
                        horizontalInset: horizontalInset,
                        details: details,
                        onOpen: onLead,
                      ),
                    const SizedBox(height: 32),
                  ],
                  const ProfileSectionLabel(
                    label: 'Office Location',
                    title: true,
                    opaqueTitle: true,
                  ),
                  const SizedBox(height: 8),
                  _LocationCard(
                    details: details,
                    avatarUrl: avatarUrl,
                    flat: true,
                  ),
                  const SizedBox(height: 16),
                ]
                .map(
                  (child) => child is _TeamStrip || horizontalInset == 0
                      ? child
                      : Padding(
                          padding: EdgeInsets.symmetric(
                            horizontal: horizontalInset,
                          ),
                          child: child,
                        ),
                )
                .toList(),
      );
    }
    final sections = {
      'Personal info': [
        if (description.isNotEmpty) (label: 'About', value: description),
        if (username != null)
          (
            label: 'Username',
            value: username!.startsWith('@') ? username! : '@${username!}',
          ),
        ...details.personal,
      ],
      'Team': details.team,
    };
    return Column(
      children: [
        for (final section in sections.entries)
          if (section.value.isNotEmpty ||
              (section.key == 'Team' &&
                  (details.lead != null || details.tenure != null)))
            AppListCard(
              outlined: true,
              iconColumnWidth: Grid.md,
              horizontalPadding: 0,
              labelOverArtwork: true,
              label: section.key,

              children: [
                for (final field in section.value)
                  _ProfileRow(
                    label: field.label,
                    value: field.value,
                    icon: switch (field.label) {
                      'Username' => BuzzIcons.atSign,
                      'Email' => BuzzIcons.mail,
                      'Team' => BuzzIcons.users,
                      _ => BuzzIcons.userCircle,
                    },
                    onCopy: field.label == 'Email'
                        ? () async {
                            await Clipboard.setData(
                              ClipboardData(text: field.value),
                            );
                            if (context.mounted) {
                              showBuzzToast(context, 'Email copied');
                            }
                          }
                        : null,
                  ),
                if (section.key == 'Team')
                  if (details.lead case final lead?)
                    _ProfileRow(
                      icon: BuzzIcons.user,
                      label: 'Led by',
                      value: lead.name,
                      portrait: ClipOval(
                        child: AvatarImageContent(
                          imageUrl: lead.avatarUrl,
                          preferThumbnail: true,
                          fallback: Center(
                            child: Text(lead.name.characters.first),
                          ),
                        ),
                      ),
                      onOpen: onLead == null ? null : () => onLead!(lead),
                    ),
                if (section.key == 'Team' && details.tenure != null)
                  _ProfileRow(
                    icon: BuzzIcons.clock,
                    label: details.tenure!.label,
                    value: details.tenure!.value,
                  ),
              ],
            ),
        _LocationCard(details: details, avatarUrl: avatarUrl),
        if (publicKey != null)
          AppListCard(
            outlined: true,
            iconColumnWidth: Grid.md,
            horizontalPadding: 0,
            children: [
              _ProfileRow(
                label: 'Public key',
                value: publicKey!,
                icon: BuzzIcons.hash,
                onCopy: onCopyKey,
                copied: copied,
              ),
            ],
          ),
      ],
    );
  }
}

class _TeamStrip extends StatelessWidget {
  const _TeamStrip({
    super.key,
    required this.details,
    required this.onOpen,
    required this.horizontalInset,
  });
  final double horizontalInset;
  final ProfileDetails details;
  final ValueChanged<ProfileLead>? onOpen;

  @override
  Widget build(BuildContext context) {
    final peers = [...details.teammates]
      ..sort((a, b) => a.name.toLowerCase().compareTo(b.name.toLowerCase()));
    return SingleChildScrollView(
      padding: EdgeInsets.symmetric(horizontal: horizontalInset),
      scrollDirection: Axis.horizontal,
      child: Row(
        spacing: Grid.twelve,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          for (final person in [?details.lead, ...peers])
            Builder(
              builder: (context) {
                final isLead = person == details.lead;
                final onTap =
                    onOpen != null && (isLead || person.pubkey != null)
                    ? () => onOpen!(person)
                    : null;
                return Semantics(
                  label: isLead ? '${person.name}, Lead' : person.name,
                  button: onTap != null,
                  onTap: onTap,
                  excludeSemantics: true,
                  child: InkWell(
                    onTap: onTap,
                    child: SizedBox(
                      width: 116,
                      child: Column(
                        children: [
                          ClipOval(
                            child: SizedBox.square(
                              dimension: teamPortraitSize,
                              child: AvatarImageContent(
                                imageUrl: person.avatarUrl,
                                preferThumbnail: true,
                                fallback: const Icon(BuzzIcons.user),
                              ),
                            ),
                          ),
                          const SizedBox(height: Grid.xxs),
                          Text(
                            person.name,
                            textAlign: TextAlign.center,
                            style: context.textTheme.bodyMedium,
                          ),
                          if (isLead)
                            Text(
                              'Lead',
                              style: context.textTheme.bodySmall?.copyWith(
                                color: context.colors.onSurfaceVariant,
                              ),
                            ),
                        ],
                      ),
                    ),
                  ),
                );
              },
            ),
        ],
      ),
    );
  }
}

class _FactRows extends StatelessWidget {
  const _FactRows({required this.fields});
  final List<ProfileField> fields;

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    spacing: Grid.xs,
    children: [
      for (final field in fields)
        _ArtworkFact(label: field.label, value: field.value),
    ],
  );
}

class _ArtworkFact extends StatelessWidget {
  const _ArtworkFact({required this.label, required this.value, this.onTap});
  final String label, value;
  final VoidCallback? onTap;
  @override
  Widget build(BuildContext context) {
    final content = Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        ProfileSectionLabel(label: label, opaqueTitle: true),
        const SizedBox(height: 4),
        Row(
          children: [
            Flexible(
              fit: onTap == null ? FlexFit.tight : FlexFit.loose,
              child: ProfileSectionLabel(
                label: value,
                fitTextWidth: onTap != null,
                messageTypography: true,
                body: true,
                textStyle: _profileBodyTextStyle,
              ),
            ),
            if (onTap != null)
              Padding(
                padding: const EdgeInsetsDirectional.only(start: Grid.half),
                child: Transform.translate(
                  offset: Offset(0, 2 / MediaQuery.devicePixelRatioOf(context)),
                  child: const Icon(BuzzIcons.copy, size: 14),
                ),
              ),
          ],
        ),
      ],
    );
    return onTap == null
        ? content
        : Semantics(
            button: true,
            child: InkWell(onTap: onTap, child: content),
          );
  }
}

class _ProfileRow extends StatelessWidget {
  const _ProfileRow({
    required this.icon,
    required this.label,
    required this.value,
    this.onCopy,
    this.copied = false,
    this.onOpen,
    this.portrait,
  });
  final Widget? portrait;
  final IconData icon;
  final String label, value;
  final VoidCallback? onCopy, onOpen;
  final bool copied;
  @override
  Widget build(BuildContext context) {
    if (portrait case final avatar?) {
      return AppListRowRaw(
        leading: SizedBox.square(dimension: Grid.md, child: avatar),
        title: Text(label, style: context.textTheme.bodyLarge),
        subtitle: Text(
          value,
          style: context.textTheme.bodySmall?.copyWith(
            color: context.colors.onSurfaceVariant,
          ),
        ),
        trailing: onOpen == null
            ? null
            : const Icon(BuzzIcons.chevronRight, size: 18),
        onTap: onOpen,
      );
    }
    return AppListRow(
      icon: icon,
      title: label,
      subtitle: value.isEmpty ? null : value,
      trailing: onCopy == null
          ? null
          : Icon(copied ? BuzzIcons.check : BuzzIcons.copy, size: 16),
      onTap: onCopy ?? onOpen,
    );
  }
}

String? _displayLocation(String? raw) {
  final parts = raw
      ?.split(',')
      .map(
        (part) => part
            .replaceAll(RegExp(r'\s*\(remote\)\s*$', caseSensitive: false), '')
            .trim(),
      )
      .where((part) => part.isNotEmpty && part.toLowerCase() != 'remote');
  final location = parts?.join(', ');
  return location == null || location.isEmpty ? null : location;
}

// Representative centres for named directory regions without supplied coordinates.
const _directoryCentres = {
  'san francisco, us': (latitude: 37.7749, longitude: -122.4194),
  'denver metro, us': (latitude: 39.7392, longitude: -104.9903),
  'nyc metro, us': (latitude: 40.7128, longitude: -74.0060),
  'southern ca, us': (latitude: 34.0, longitude: -117.5),
};

class ProfileHeaderClock extends HookWidget {
  const ProfileHeaderClock({super.key, this.city, this.zoneName});
  final String? city;
  final String? zoneName;
  @override
  Widget build(BuildContext context) {
    final now = _useClock();
    final zone = _locations[zoneName];
    final label = [
      ?_displayLocation(city),
      if (zone != null)
        DateFormat('h:mm a', 'en_US').format(tz.TZDateTime.from(now, zone)),
    ].join(' · ');
    if (label.isEmpty) return const SizedBox.shrink();
    final ios = Theme.of(context).platform == TargetPlatform.iOS;
    return Text(
      label,
      textAlign: TextAlign.center,
      style: context.textTheme.bodyMedium?.copyWith(
        color: ios ? context.colors.onSurface : context.colors.onSurfaceVariant,
      ),
    );
  }
}

String _relativeClockLabel(Duration difference) {
  final minutes = difference.inMinutes.abs();
  if (minutes == 0) return '';
  final hours = minutes ~/ 60;
  final remainder = minutes % 60;
  final amount = [
    if (hours > 0) '$hours ${hours == 1 ? "hour" : "hours"}',
    if (remainder > 0) '$remainder min',
  ].join(' ');
  return '$amount ${difference.isNegative ? "behind" : "ahead"}';
}

DateTime _useClock() {
  final now = useState(DateTime.now().toUtc());
  useEffect(() {
    final timer = Timer.periodic(
      const Duration(minutes: 1),
      (_) => now.value = DateTime.now().toUtc(),
    );
    return timer.cancel;
  }, const []);
  return now.value;
}

final _locations = (() {
  zones.initializeTimeZones();
  final zonesByName = tz.timeZoneDatabase.locations;
  return {
    ...zonesByName,
    'US/Eastern': zonesByName['America/New_York']!,
    'US/Central': zonesByName['America/Chicago']!,
    'US/Mountain': zonesByName['America/Denver']!,
    'US/Pacific': zonesByName['America/Los_Angeles']!,
  };
})();

class _LocationCard extends HookWidget {
  const _LocationCard({
    required this.details,
    required this.avatarUrl,
    this.flat = false,
  });
  final bool flat;
  final ProfileDetails details;
  final String? avatarUrl;

  @override
  Widget build(BuildContext context) {
    final pinKey = useMemoized(() => GlobalKey());
    final globeId = useRef<int?>(null);
    final now = _useClock();
    final zone = _locations[details.timeZone];
    final local = zone == null ? null : tz.TZDateTime.from(now, zone);
    final city = _displayLocation(details.city);
    final coords =
        details.coordinates ?? _directoryCentres[city?.toLowerCase()];
    final useGlobe = !kIsWeb && defaultTargetPlatform == TargetPlatform.iOS;
    final colors = context.colors;
    final dark = colors.brightness == Brightness.dark;
    final water = colors.surfaceContainerLowest;
    final land = Color.alphaBlend(
      colors.onSurfaceVariant.withValues(alpha: dark ? .35 : .28),
      water,
    );
    final children = <Widget>[
      if (!flat)
        _ProfileRow(
          icon: Icons.public,
          label:
              city ?? (local != null ? 'Local time' : 'Location unavailable'),
          value: local == null
              ? ''
              : '${TimeOfDay.fromDateTime(local).format(context)} ${local.timeZoneName}',
          onOpen: city == null
              ? null
              : () => launchUrl(Uri.https('maps.apple.com', '/', {'q': city})),
        ),
      if (coords == null)
        AspectRatio(
          aspectRatio: flat ? 1 : 2.6,
          child: local != null
              ? _LocalSky(local: local)
              : ColoredBox(
                  color: water,
                  child: SvgPicture.asset(
                    'assets/images/profile-world.svg',
                    colorFilter: ColorFilter.mode(land, BlendMode.srcIn),
                  ),
                ),
        ),
      if (coords != null && city != null)
        Semantics(
          label: 'Map highlighting $city',
          image: true,
          child: ClipRect(
            child: AspectRatio(
              aspectRatio: flat ? 1 : 160 / 60,
              child: LayoutBuilder(
                builder: (context, box) {
                  final width = box.maxWidth;
                  final height = box.maxHeight;
                  final scale = width / _mapLongitudeSpan;
                  final centerLat = coords.latitude;
                  return ColoredBox(
                    color: water,
                    child: Stack(
                      children: [
                        if (useGlobe)
                          Positioned.fill(
                            child: UiKitView(
                              key: ValueKey((coords, dark)),
                              hitTestBehavior:
                                  PlatformViewHitTestBehavior.transparent,
                              viewType: 'buzz/profile_globe',
                              // This view draws beneath Flutter tap targets. Let the native sheet pan.
                              gestureBlockingPolicy:
                                  UiKitViewGestureBlockingPolicy
                                      .doNotBlockGesture,
                              onPlatformViewCreated: (id) => globeId.value = id,
                              creationParamsCodec: const StandardMessageCodec(),
                              creationParams: {
                                'officePreview': flat,
                                'latitude': coords.latitude,
                                'longitude': coords.longitude,
                                'dark': dark,
                              },
                            ),
                          )
                        else ...[
                          for (final wrap in [-360, 0, 360])
                            Positioned(
                              left:
                                  width / 2 +
                                  (wrap - coords.longitude - 180) * scale,
                              top: height / 2 + (centerLat - 90) * scale,
                              width: 360 * scale,
                              height: 180 * scale,
                              child: SvgPicture.asset(
                                'assets/images/profile-world.svg',
                                colorFilter: ColorFilter.mode(
                                  land,
                                  BlendMode.srcIn,
                                ),
                              ),
                            ),
                          Positioned.fill(
                            child: CustomPaint(
                              painter: _NightShade(
                                now,
                                coords.longitude,
                                centerLat.toDouble(),
                                mask: colors.scrim.withValues(
                                  alpha: dark ? .60 : .16,
                                ),
                                stroke: colors.onSurface.withValues(
                                  alpha: dark ? .52 : .24,
                                ),
                              ),
                            ),
                          ),
                        ],
                        Positioned(
                          left: width * (flat ? .68 : .5) - 22,
                          top: height / 2 - (useGlobe ? 26 : 52),
                          child: IgnorePointer(
                            ignoring: useGlobe,
                            child: GestureDetector(
                              onTap: () => launchUrl(
                                Uri.https('maps.apple.com', '/', {'q': city}),
                              ),
                              child: RepaintBoundary(
                                key: pinKey,
                                child: PhysicalShape(
                                  clipper: const _MapPin(),
                                  color: useGlobe
                                      ? Colors.white
                                      : dark
                                      ? colors.surface
                                      : colors.surfaceContainerLow,
                                  elevation: dark ? 3 : 1,
                                  shadowColor: Colors.black54,
                                  child: SizedBox(
                                    width: 44,
                                    height: 52,
                                    child: Align(
                                      alignment: Alignment.topCenter,
                                      child: Padding(
                                        padding: const EdgeInsets.all(4),
                                        child: ClipOval(
                                          child: SizedBox.square(
                                            dimension: 36,
                                            child: AvatarImageContent(
                                              imageUrl: avatarUrl,
                                              preferThumbnail: true,
                                              fallback: const Icon(
                                                Icons.location_on,
                                              ),
                                            ),
                                          ),
                                        ),
                                      ),
                                    ),
                                  ),
                                ),
                              ),
                            ),
                          ),
                        ),
                        if (useGlobe)
                          Positioned.fill(
                            child: Semantics(
                              button: true,
                              label: 'Open interactive map',
                              child: GestureDetector(
                                behavior: HitTestBehavior.opaque,
                                onTap: () async {
                                  final id = globeId.value;
                                  if (id == null) return;
                                  final boundary =
                                      pinKey.currentContext?.findRenderObject()
                                          as RenderRepaintBoundary?;
                                  final image = await boundary?.toImage(
                                    pixelRatio: MediaQuery.devicePixelRatioOf(
                                      context,
                                    ),
                                  );
                                  try {
                                    final data = await image?.toByteData(
                                      format: ui.ImageByteFormat.png,
                                    );
                                    if (context.mounted &&
                                        globeId.value == id) {
                                      await MethodChannel(
                                        'buzz/profile_globe/$id',
                                      ).invokeMethod<void>(
                                        'open',
                                        data?.buffer.asUint8List(),
                                      );
                                    }
                                  } finally {
                                    image?.dispose();
                                  }
                                },
                              ),
                            ),
                          ),
                      ],
                    ),
                  );
                },
              ),
            ),
          ),
        ),
    ];
    if (flat) {
      final differenceLabel = local == null
          ? ''
          : _relativeClockLabel(
              local.timeZoneOffset - now.toLocal().timeZoneOffset,
            );
      final location = city?.trim() ?? '';
      final comma = location.lastIndexOf(',');
      final place = comma > 0 ? location.substring(0, comma).trim() : location;
      final country = comma > 0 ? location.substring(comma + 1).trim() : '';
      final clock = local == null
          ? ''
          : DateFormat('h:mm a', 'en_US').format(local);
      final caption = [
        clock,
        if (differenceLabel.isNotEmpty) differenceLabel,
      ].join(' · ');
      final remoteOnly =
          details.city
                  ?.split(',')
                  .any((part) => part.trim().toLowerCase() == 'remote') ==
              true &&
          (location.isEmpty || isProfileCountry(location));
      if (coords == null && remoteOnly) {
        final zoneLabel = switch (details.timeZone) {
          'US/Eastern' ||
          'America/New_York' ||
          'America/Detroit' ||
          'America/Toronto' => 'Eastern Time',
          'US/Central' || 'America/Chicago' => 'Central Time',
          'US/Mountain' || 'America/Denver' => 'Mountain Time',
          'US/Pacific' ||
          'America/Los_Angeles' ||
          'America/Vancouver' => 'Pacific Time',
          _ => local?.timeZoneName,
        };
        return _FactRows(
          fields: [
            (
              label: zoneLabel == null ? 'Location' : 'Time zone',
              value: [
                ?zoneLabel,
                if (location.isNotEmpty) location,
                'Remote',
              ].join(' · '),
            ),
            if (local != null) (label: 'Local time', value: caption),
          ],
        );
      }
      final sky = coords == null && local != null;
      final captionColor =
          sky && _skyColors(local).last.computeLuminance() > .179
          ? Colors.black
          : Colors.white;
      return ClipRSuperellipse(
        borderRadius: BorderRadius.circular(22),
        child: Stack(
          alignment: Alignment.bottomLeft,
          children: [
            Positioned.fill(
              child: children.isEmpty
                  ? ColoredBox(color: water)
                  : children.first,
            ),
            if (!sky)
              Positioned.fill(
                child: IgnorePointer(
                  child: DecoratedBox(
                    decoration: const BoxDecoration(
                      gradient: RadialGradient(
                        center: Alignment.bottomLeft,
                        radius: 1.4,
                        colors: [
                          Color(0xAA000000),
                          Color(0x65000000),
                          Colors.transparent,
                        ],
                        stops: [0, .5, 1],
                      ),
                    ),
                  ),
                ),
              ),
            ConstrainedBox(
              constraints: const BoxConstraints(minHeight: 148),
              child: Padding(
                padding: const EdgeInsets.fromLTRB(16, 22, 16, 16),
                child: Align(
                  alignment: Alignment.bottomLeft,
                  heightFactor: 1,
                  child: GestureDetector(
                    onTap: city == null
                        ? null
                        : () => launchUrl(
                            Uri.https('maps.apple.com', '/', {'q': city}),
                          ),
                    child: DefaultTextStyle(
                      style: context.textTheme.bodySmall!.copyWith(
                        color: captionColor,
                        height: 1.1,
                      ),
                      textAlign: TextAlign.left,
                      child: Column(
                        mainAxisSize: MainAxisSize.min,
                        spacing: 4,
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text(
                            place.isEmpty ? 'Location unavailable' : place,
                            style: context.textTheme.titleMedium?.copyWith(
                              color: captionColor,
                              height: 1.1,
                              fontWeight: FontWeight.w700,
                            ),
                          ),
                          if (country.isNotEmpty) Text(country),
                          if (caption.isNotEmpty) Text(caption),
                        ],
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ],
        ),
      );
    }
    return AppListCard(
      outlined: true,
      iconColumnWidth: Grid.md,
      horizontalPadding: 0,
      labelOverArtwork: true,
      label: 'Location',
      children: children,
    );
  }
}

// Approximate the solar terminator using the seasonal declination and UTC hour.
class _NightShade extends CustomPainter {
  const _NightShade(
    this.now,
    this.longitude,
    this.latitude, {
    required this.mask,
    required this.stroke,
  });
  final Color mask;
  final Color stroke;
  final DateTime now;
  final double longitude;
  final double latitude;

  @override
  void paint(Canvas canvas, Size size) {
    const radians = math.pi / 180;
    final day = now.difference(DateTime.utc(now.year)).inDays + 1;
    final declination =
        -23.44 * radians * math.cos((day + 10) * 2 * math.pi / 365.25);
    final midnightEdge = declination >= 0 ? size.height : 0.0;
    final scale = size.width / _mapLongitudeSpan;
    final boundaryPath = Path();
    for (int i = 0; i <= 160; i++) {
      final x = size.width * i / 160;
      final lon = longitude + (x - size.width / 2) / scale;
      final angle = (15 * (now.hour + now.minute / 60 - 12) + lon) * radians;
      final boundary =
          math.atan(-math.cos(angle) / math.tan(declination)) / radians;
      final y = size.height / 2 + (latitude - boundary) * scale;
      if (i == 0) {
        boundaryPath.moveTo(x, y);
      } else {
        boundaryPath.lineTo(x, y);
      }
    }
    final night = Path.from(boundaryPath)
      ..lineTo(size.width, midnightEdge)
      ..lineTo(0, midnightEdge)
      ..close();
    canvas.drawPath(night, Paint()..color = mask);
    canvas.drawPath(
      boundaryPath,
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1
        ..color = stroke,
    );
  }

  @override
  bool shouldRepaint(_NightShade old) =>
      old.now != now ||
      old.longitude != longitude ||
      old.latitude != latitude ||
      old.mask != mask ||
      old.stroke != stroke;
}

class _MapPin extends CustomClipper<Path> {
  const _MapPin();
  @override
  Path getClip(Size size) => Path()
    ..moveTo(22, 0)
    ..arcToPoint(const Offset(34, 40), radius: const Radius.circular(22))
    ..quadraticBezierTo(28, 44, 24, 50)
    ..quadraticBezierTo(22, 53, 20, 50)
    ..quadraticBezierTo(16, 44, 10, 40)
    ..arcToPoint(const Offset(22, 0), radius: const Radius.circular(22))
    ..close();
  @override
  bool shouldReclip(_MapPin oldClipper) => false;
}
