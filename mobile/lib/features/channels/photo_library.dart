import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:image_picker/image_picker.dart';
import 'package:photo_manager/photo_manager.dart';

const _recentPhotoCount = 30;
const _photoPermissionRequest = PermissionRequestOption(
  androidPermission: AndroidPermission(
    type: RequestType.image,
    mediaLocation: false,
  ),
);

/// A recent photo exposed to Buzz's compact in-composer gallery.
@immutable
class RecentPhoto {
  /// The platform photo-library identifier.
  final String id;

  /// Thumbnail bytes sized for the compact picker grid.
  final Uint8List thumbnailBytes;

  /// Creates a recent photo value.
  const RecentPhoto({required this.id, required this.thumbnailBytes});
}

/// Reads recent photos and resolves selected library assets to uploadable files.
abstract interface class PhotoLibrary {
  /// Requests access when needed and returns the newest visible photos.
  Future<List<RecentPhoto>> loadRecentPhotos();

  /// Resolves selected photos in the supplied selection order.
  Future<List<XFile>> resolveSelectedPhotos(List<RecentPhoto> photos);
}

/// Indicates that the compact gallery cannot read the device photo library.
class PhotoLibraryAccessException implements Exception {
  /// Creates a photo-library access error.
  const PhotoLibraryAccessException();

  @override
  String toString() => 'Photo access is turned off for Buzz.';
}

/// Provides the device photo library. Tests can override this with fixtures.
final photoLibraryProvider = Provider<PhotoLibrary>(
  (ref) => const DevicePhotoLibrary(),
);

/// The device-backed implementation of [PhotoLibrary].
class DevicePhotoLibrary implements PhotoLibrary {
  /// Creates the device photo library.
  const DevicePhotoLibrary();

  @override
  Future<List<RecentPhoto>> loadRecentPhotos() async {
    final permission = await PhotoManager.requestPermissionExtend(
      requestOption: _photoPermissionRequest,
    );
    if (!permission.hasAccess) {
      throw const PhotoLibraryAccessException();
    }

    final assets = await PhotoManager.getAssetListPaged(
      page: 0,
      pageCount: _recentPhotoCount,
      type: RequestType.image,
      filterOption: FilterOptionGroup(
        imageOption: const FilterOption(needTitle: true),
        orders: const [
          OrderOption(type: OrderOptionType.createDate, asc: false),
        ],
      ),
    );

    final loaded = await Future.wait([
      for (final asset in assets) _loadRecentPhoto(asset),
    ]);
    return [for (final photo in loaded) ?photo];
  }

  Future<RecentPhoto?> _loadRecentPhoto(AssetEntity asset) async {
    final bytes = await asset.thumbnailDataWithSize(
      const ThumbnailSize.square(256),
      quality: 84,
    );
    if (bytes == null || bytes.isEmpty) return null;
    return RecentPhoto(id: asset.id, thumbnailBytes: bytes);
  }

  @override
  Future<List<XFile>> resolveSelectedPhotos(List<RecentPhoto> photos) async {
    final resolved = <XFile>[];
    for (final photo in photos) {
      final asset = await AssetEntity.fromId(photo.id);
      final file = await asset?.file;
      if (file == null) {
        throw const PhotoLibraryAccessException();
      }
      resolved.add(XFile(file.path, name: asset?.title));
    }
    return resolved;
  }
}
