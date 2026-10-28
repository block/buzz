import Flutter
import UIKit

final class IosMediaCarouselFactory: NSObject, FlutterPlatformViewFactory {
  private let messenger: FlutterBinaryMessenger
  init(messenger: FlutterBinaryMessenger) { self.messenger = messenger }
  func createArgsCodec() -> FlutterMessageCodec & NSObjectProtocol {
    FlutterStandardMessageCodec.sharedInstance()
  }
  func create(withFrame frame: CGRect, viewIdentifier viewId: Int64, arguments args: Any?) -> FlutterPlatformView {
    IosMediaCarousel(frame: frame, id: viewId, arguments: args, messenger: messenger)
  }
}

private final class CarouselPhotoCell: UICollectionViewCell {
  let image = UIImageView()
  let status = UILabel()
  var request: String?
  var cancelRequest: (() -> Void)?
  func cancelImage() {
    cancelRequest?()
    cancelRequest = nil
    request = nil
  }
  override init(frame: CGRect) {
    super.init(frame: frame)
    image.contentMode = .scaleAspectFill
    contentView.clipsToBounds = true
    contentView.layer.cornerCurve = .continuous
    contentView.backgroundColor = .secondarySystemBackground
    status.textAlignment = .center
    status.numberOfLines = 2
    status.font = .preferredFont(forTextStyle: .caption1)
    status.textColor = .secondaryLabel
    contentView.addSubview(image)
    contentView.addSubview(status)
    isAccessibilityElement = true
    accessibilityTraits = .button
  }
  required init?(coder: NSCoder) { nil }
  override func layoutSubviews() {
    super.layoutSubviews()
    image.frame = contentView.bounds
    status.frame = contentView.bounds.insetBy(dx: 12, dy: 12)
  }
  override func prepareForReuse() {
    super.prepareForReuse()
    cancelImage()
    image.image = nil
    status.text = nil
  }
}

/// A native collection view owns drag tracking, deceleration and snapping.
/// Thumbnail bytes come from Flutter's authenticated, community-scoped cache.
final class IosMediaCarousel: NSObject, FlutterPlatformView, UICollectionViewDataSource, UICollectionViewDelegateFlowLayout {
  private let collection: UICollectionView
  private let layout = UICollectionViewFlowLayout()
  private let channel: FlutterMethodChannel
  private var labels: [String] = []
  private var stride: CGFloat = 1
  private var gutter: CGFloat = 0
  private var gap: CGFloat = 8
  private var trailingPadding: CGFloat = 16
  private var radius: CGFloat = 16
  private var rtl = false
  private var ready = false
  private var applying = false
  private var selected = 0

  init(frame: CGRect, id: Int64, arguments: Any?, messenger: FlutterBinaryMessenger) {
    layout.scrollDirection = .horizontal
    collection = UICollectionView(frame: frame, collectionViewLayout: layout)
    channel = FlutterMethodChannel(name: "buzz/ios_media_carousel/\(id)", binaryMessenger: messenger)
    super.init()
    collection.backgroundColor = .clear
    collection.showsHorizontalScrollIndicator = false
    collection.alwaysBounceHorizontal = true
    collection.decelerationRate = .fast
    collection.contentInsetAdjustmentBehavior = .never
    collection.dataSource = self
    collection.delegate = self
    collection.register(CarouselPhotoCell.self, forCellWithReuseIdentifier: "photo")
    apply(arguments as? [String: Any] ?? [:])
    channel.setMethodCallHandler { [weak self] call, result in
      guard let self else { result(nil); return }
      if call.method == "imageFrame", let args = call.arguments as? [String: Any],
         let id = args["id"] as? String {
        for case let cell as CarouselPhotoCell in self.collection.visibleCells where cell.request == id {
          let image = (args["bytes"] as? FlutterStandardTypedData).flatMap { UIImage(data: $0.data) }
          cell.image.image = image
          cell.status.text = image == nil ? "Image unavailable\nTap to open" : nil
        }
        result(nil)
        return
      }
      guard call.method == "configure", let args = call.arguments as? [String: Any] else {
        result(FlutterMethodNotImplemented)
        return
      }
      let wasReady = self.ready
      self.ready = true
      self.apply(args)
      if !wasReady { self.collection.reloadData() }
      result(nil)
    }
  }
  func view() -> UIView { collection }

  private func apply(_ args: [String: Any]) {
    applying = true
    defer { applying = false }
    let oldTrailingPadding = trailingPadding
    let oldStride = stride
    let oldGutter = gutter
    let oldRTL = rtl
    let oldLabels = labels
    labels = args["labels"] as? [String] ?? []
    stride = max(1, (args["width"] as? Double ?? 1) * 0.9)
    gutter = args["gutter"] as? Double ?? 0
    gap = args["gap"] as? Double ?? 8
    trailingPadding = args["trailingPadding"] as? Double ?? 16
    radius = args["radius"] as? Double ?? 16
    rtl = args["rtl"] as? Bool ?? false
    collection.overrideUserInterfaceStyle = args["brightness"] as? String == "dark" ? .dark : .light
    // Reflect the strip and each cell, keeping logical indices in reading order.
    collection.transform = rtl ? CGAffineTransform(scaleX: -1, y: 1) : .identity
    collection.contentInset = UIEdgeInsets(top: 0, left: gutter, bottom: 0, right: gutter)
    layout.minimumLineSpacing = gap
    // The strip overflows the message gutter; restore that full inset at its end.
    layout.sectionInset = UIEdgeInsets(top: 0, left: 0, bottom: 0, right: trailingPadding)
    layout.invalidateLayout()
    for cell in collection.visibleCells {
      cell.contentView.layer.cornerRadius = radius
    }
    if oldTrailingPadding != trailingPadding || oldStride != stride || oldGutter != gutter || oldLabels != labels || oldRTL != rtl {
      selected = min(selected, max(0, labels.count - 1))
      for case let cell as CarouselPhotoCell in collection.visibleCells { cell.cancelImage() }
      collection.reloadData()
      collection.layoutIfNeeded()
      collection.contentOffset.x = clampedOffset(CGFloat(selected) * stride - gutter)
    }
  }

  func collectionView(_ collectionView: UICollectionView, numberOfItemsInSection section: Int) -> Int {
    labels.count
  }
  func collectionView(_ collectionView: UICollectionView, layout collectionViewLayout: UICollectionViewLayout, sizeForItemAt indexPath: IndexPath) -> CGSize {
    CGSize(width: max(1, stride - gap), height: max(1, collectionView.bounds.height))
  }
  func collectionView(_ collectionView: UICollectionView, cellForItemAt indexPath: IndexPath) -> UICollectionViewCell {
    guard let cell = collectionView.dequeueReusableCell(withReuseIdentifier: "photo", for: indexPath) as? CarouselPhotoCell else {
      return UICollectionViewCell()
    }
    cell.transform = rtl ? CGAffineTransform(scaleX: -1, y: 1) : .identity
    cell.contentView.layer.cornerRadius = radius
    cell.accessibilityLabel = "Open \(labels[indexPath.item])"
    cell.accessibilityValue = "\(indexPath.item + 1) of \(labels.count)"
    cell.status.text = "Loading image"
    return cell
  }
  func collectionView(_ collectionView: UICollectionView, willDisplay cell: UICollectionViewCell, forItemAt indexPath: IndexPath) {
    guard ready, let cell = cell as? CarouselPhotoCell else { return }
    cell.cancelImage()
    let id = UUID().uuidString
    cell.request = id
    cell.cancelRequest = { [weak self] in
      self?.channel.invokeMethod("cancelImage", arguments: id)
    }
    channel.invokeMethod("subscribeImage", arguments: ["id": id, "index": indexPath.item])
  }
  func collectionView(_ collectionView: UICollectionView, didEndDisplaying cell: UICollectionViewCell, forItemAt indexPath: IndexPath) {
    (cell as? CarouselPhotoCell)?.cancelImage()
  }
  func collectionView(_ collectionView: UICollectionView, didSelectItemAt indexPath: IndexPath) {
    channel.invokeMethod("open", arguments: indexPath.item)
  }
  func scrollViewDidScroll(_ scrollView: UIScrollView) {
    guard ready, !applying, !labels.isEmpty else { return }
    let offset = max(0, scrollView.contentOffset.x + gutter)
    channel.invokeMethod("scroll", arguments: offset)
    let index = min(labels.count - 1, max(0, Int((offset / stride).rounded())))
    if index != selected {
      selected = index
      channel.invokeMethod("page", arguments: index)
    }
  }
  func scrollViewWillEndDragging(_ scrollView: UIScrollView, withVelocity velocity: CGPoint, targetContentOffset: UnsafeMutablePointer<CGPoint>) {
    let page = ((targetContentOffset.pointee.x + gutter) / stride).rounded()
    targetContentOffset.pointee.x = clampedOffset(page * stride - gutter)
  }
  private func clampedOffset(_ offset: CGFloat) -> CGFloat {
    let maximum = max(-gutter, collection.contentSize.width - collection.bounds.width + gutter)
    return min(maximum, max(-gutter, offset))
  }
  deinit {
    for case let cell as CarouselPhotoCell in collection.visibleCells {
      if let id = cell.request { channel.invokeMethod("cancelImage", arguments: id) }
    }
    channel.setMethodCallHandler(nil)
  }
}
