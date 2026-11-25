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
import '../../shared/widgets/modal_presentation.dart';
import 'profile_sheet_layout.dart';
import '../../shared/widgets/app_list.dart';
import '../../shared/widgets/app_list_card.dart';

// Midpoint in zoom levels between the global and regional previews.
const _mapLongitudeSpan = 164.0;

typedef ProfileField = ({String label, String value});

typedef ProfileLead = ({String name, String? avatarUrl, String? pubkey});
final ProfileLead previewDesignLead = (
  name: 'Alex Example',
  pubkey: null,
  avatarUrl: Uri.dataFromString(
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 80 80"><rect width="80" height="80" fill="#c8dcd2"/><path d="M8 84Q12 52 40 52T73 84" fill="#315452"/><ellipse cx="40" cy="36" rx="17" ry="21" fill="#b77755"/><path d="M22 36Q16 8 40 10Q61 9 59 32Q45 30 36 21Q32 30 22 36" fill="#382d2b"/></svg>',
    mimeType: 'image/svg+xml',
  ).toString(),
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
      builder: (_) => ProfileSheetLayout(
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
            Text(lead.name, style: Theme.of(context).textTheme.titleMedium),
          ],
        ),
        body: const bool.fromEnvironment('BUZZ_PROFILE_PREVIEW')
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
      ),
    );

class ProfileDetails {
  const ProfileDetails({
    this.personal = const [],
    this.team = const [],
    this.tenure,
    this.city,
    this.timeZone,
    this.coordinates,
    this.lead,
  });
  final List<ProfileField> personal;
  final List<ProfileField> team;
  final ProfileField? tenure;
  final ProfileLead? lead;
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
    this.publicKey,
    this.onCopyKey,
    this.onLead,
    this.copied = false,
  });
  final ProfileDetails details;
  final String? avatarUrl;
  final String about;
  final String? username;
  final String? publicKey;
  final VoidCallback? onCopyKey;
  final ValueChanged<ProfileLead>? onLead;
  final bool copied;

  @override
  Widget build(BuildContext context) {
    final sections = {
      'Personal info': [
        if (about.trim().isNotEmpty) (label: 'About', value: about),
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

class ProfileHeaderClock extends HookWidget {
  const ProfileHeaderClock({super.key, required this.zoneName});
  final String zoneName;
  @override
  Widget build(BuildContext context) {
    final now = _useClock();
    final zone = _locations[zoneName];
    if (zone == null) return const SizedBox.shrink();
    final local = tz.TZDateTime.from(now, zone);
    return Text(
      DateFormat('h:mm a', 'en_US').format(local),
      style: context.textTheme.bodyMedium?.copyWith(
        color: context.colors.onSurfaceVariant,
      ),
    );
  }
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
  return tz.timeZoneDatabase.locations;
})();

class _LocationCard extends HookWidget {
  const _LocationCard({required this.details, required this.avatarUrl});
  final ProfileDetails details;
  final String? avatarUrl;

  @override
  Widget build(BuildContext context) {
    final pinKey = useMemoized(() => GlobalKey());
    final globeId = useRef<int?>(null);
    final now = _useClock();
    final zone = _locations[details.timeZone];
    final local = zone == null ? null : tz.TZDateTime.from(now, zone);
    final city = details.city;
    final coords = details.coordinates;
    final useGlobe = !kIsWeb && defaultTargetPlatform == TargetPlatform.iOS;
    final colors = context.colors;
    final dark = colors.brightness == Brightness.dark;
    final water = colors.surfaceContainerLowest;
    final land = Color.alphaBlend(
      colors.onSurfaceVariant.withValues(alpha: dark ? .35 : .28),
      water,
    );
    return AppListCard(
      outlined: true,
      iconColumnWidth: Grid.md,
      horizontalPadding: 0,
      labelOverArtwork: true,
      label: 'Location',

      children: [
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
            aspectRatio: 2.6,
            child: ColoredBox(
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
                aspectRatio: 160 / 60,
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
                                viewType: 'buzz/profile_globe',
                                onPlatformViewCreated: (id) =>
                                    globeId.value = id,
                                creationParamsCodec:
                                    const StandardMessageCodec(),
                                creationParams: {
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
                            left: width / 2 - 22,
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
                                    color: dark
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
                                        pinKey.currentContext
                                                ?.findRenderObject()
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
      ],
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
