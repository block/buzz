import UIKit

final class NavigationTitleView: UIVisualEffectView, UIGestureRecognizerDelegate {
  var onActivate: (() -> Void)?
  private let titleLabel = UILabel()
  private let subtitleLabel = UILabel()
  private let contentStack = UIStackView()
  private let textStack = UIStackView()
  private var contentLeading: NSLayoutConstraint!
  private let subtitleStack = UIStackView()
  private let maximumWidth: CGFloat?
  private(set) var portraitSlot: UIView?

  init(title: String?, subtitle: String, color: UIColor, maximumWidth: CGFloat? = nil) {
    self.maximumWidth = maximumWidth
    if #available(iOS 26.0, *) {
      super.init(effect: UIGlassEffect(style: .regular))
    } else {
      super.init(effect: UIBlurEffect(style: .systemMaterial))
    }
    clipsToBounds = true
    layer.cornerCurve = .continuous
    titleLabel.text = title
    titleLabel.textColor = color
    subtitleStack.isHidden = subtitle.isEmpty
    subtitleLabel.text = subtitle
    subtitleLabel.textColor = .secondaryLabel
    for label in [titleLabel, subtitleLabel] {
      label.textAlignment = .center
      label.lineBreakMode = .byTruncatingTail
      label.adjustsFontForContentSizeCategory = false
      label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
    }
    subtitleStack.axis = .horizontal
    subtitleStack.alignment = .center
    subtitleStack.spacing = 6
    subtitleStack.addArrangedSubview(subtitleLabel)
    textStack.axis = .vertical
    textStack.alignment = .center
    textStack.addArrangedSubview(titleLabel)
    textStack.addArrangedSubview(subtitleStack)
    contentStack.axis = .horizontal
    contentStack.alignment = .center
    contentStack.spacing = 8
    contentStack.addArrangedSubview(textStack)
    contentStack.translatesAutoresizingMaskIntoConstraints = false
    contentView.addSubview(contentStack)
    contentLeading = contentStack.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 12)
    NSLayoutConstraint.activate([
      contentLeading,
      contentStack.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -12),
      contentStack.centerYAnchor.constraint(equalTo: contentView.centerYAnchor),
      titleLabel.widthAnchor.constraint(lessThanOrEqualTo: contentView.widthAnchor, constant: -24),
      subtitleStack.widthAnchor.constraint(lessThanOrEqualTo: contentView.widthAnchor, constant: -24),
    ])
    translatesAutoresizingMaskIntoConstraints = false
    if let maximumWidth {
      widthAnchor.constraint(lessThanOrEqualToConstant: max(0, maximumWidth)).isActive = true
    }
    updateFonts()
    // Flutter creates platform views with a zero frame. Keep a nonzero
    // intrinsic width and let UINavigationBar compress it between its items.
    setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
    isAccessibilityElement = true
    accessibilityTraits = .button
    let tap = UITapGestureRecognizer(target: self, action: #selector(activate))
    tap.delegate = self
    addGestureRecognizer(tap)
  }

  /// Flutter paints the live portrait in this native capsule's reserved slot.
  func reservePortraitSlot() {
    let slot = UIView()
    contentStack.insertArrangedSubview(slot, at: 0)
    NSLayoutConstraint.activate([
      slot.widthAnchor.constraint(equalToConstant: 40),
      slot.heightAnchor.constraint(equalToConstant: 40),
    ])
    portraitSlot = slot
    contentLeading.constant = 2
    textStack.alignment = .leading
    for label in [titleLabel, subtitleLabel] { label.textAlignment = .natural }
    invalidateIntrinsicContentSize()
  }

  func setAvatar(_ image: UIImage?, presence: UIColor?) {
    // Bar-item images carry alignment insets; the title owns its 32pt layout.
    let avatar = UIImageView(image: image?.withAlignmentRectInsets(.zero))
    avatar.accessibilityIdentifier = "dm-navigation-avatar"
    avatar.contentMode = .scaleAspectFit
    let avatarContainer = UIView()
    avatar.translatesAutoresizingMaskIntoConstraints = false
    avatarContainer.addSubview(avatar)
    contentStack.insertArrangedSubview(avatarContainer, at: 0)
    NSLayoutConstraint.activate([
      avatarContainer.widthAnchor.constraint(equalToConstant: 32),
      avatarContainer.heightAnchor.constraint(equalToConstant: 32),
      avatar.leadingAnchor.constraint(equalTo: avatarContainer.leadingAnchor),
      avatar.trailingAnchor.constraint(equalTo: avatarContainer.trailingAnchor),
      avatar.topAnchor.constraint(equalTo: avatarContainer.topAnchor),
      avatar.bottomAnchor.constraint(equalTo: avatarContainer.bottomAnchor),
    ])
    textStack.alignment = .leading
    for label in [titleLabel, subtitleLabel] { label.textAlignment = .natural }
    if let presence {
      let badge = UIView()
      badge.backgroundColor = presence
      badge.layer.cornerRadius = 4
      badge.layer.borderWidth = 1.5
      badge.layer.borderColor = UIColor.systemBackground.cgColor
      badge.accessibilityIdentifier = "dm-navigation-presence"
      badge.translatesAutoresizingMaskIntoConstraints = false
      avatarContainer.addSubview(badge)
      NSLayoutConstraint.activate([
        badge.widthAnchor.constraint(equalToConstant: 8),
        badge.heightAnchor.constraint(equalToConstant: 8),
        badge.trailingAnchor.constraint(equalTo: avatarContainer.trailingAnchor),
        badge.bottomAnchor.constraint(equalTo: avatarContainer.bottomAnchor),
      ])
    }
    invalidateIntrinsicContentSize()
  }

  func setSubtitlePresence(_ color: UIColor) {
    let dot = UIView()
    dot.backgroundColor = color
    dot.layer.cornerRadius = 3
    dot.isAccessibilityElement = false
    dot.accessibilityIdentifier = "dm-navigation-status-dot"
    subtitleStack.insertArrangedSubview(dot, at: 0)
    NSLayoutConstraint.activate([
      dot.widthAnchor.constraint(equalToConstant: 6),
      dot.heightAnchor.constraint(equalToConstant: 6),
    ])
    invalidateIntrinsicContentSize()
  }

  func setEphemeralStatus(_ label: String) {
    // The subtitle carries the visible temporary/expiry text. Keep the full
    // retention explanation on the title's single VoiceOver element too.
    accessibilityLabel = [accessibilityLabel, label].compactMap { $0 }.joined(separator: ", ")
  }

  func attachRefreshBar(_ bar: NavigationRefreshBar) {
    bar.removeFromSuperview()
    bar.translatesAutoresizingMaskIntoConstraints = false
    contentView.addSubview(bar)
    NSLayoutConstraint.activate([
      bar.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
      bar.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
      bar.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
      bar.heightAnchor.constraint(equalToConstant: 2),
    ])
  }

  func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
    acceptsTitleTouch(in: touch.view)
  }

  // UIKit can wrap titleView in a UIControl. Only controls inside our title
  // should consume the title tap.
  func acceptsTitleTouch(in touchedView: UIView?) -> Bool {
    var touched = touchedView
    while let view = touched, view !== self {
      if view is UIControl { return false }
      touched = view.superview
    }
    return true
  }

  required init?(coder: NSCoder) { return nil }

  @objc private func activate() { onActivate?() }

  override func accessibilityActivate() -> Bool {
    guard isUserInteractionEnabled, let onActivate else { return false }
    onActivate()
    return true
  }

  override var intrinsicContentSize: CGSize {
    let contentSize = contentStack.systemLayoutSizeFitting(UIView.layoutFittingCompressedSize)
    return CGSize(width: min(contentSize.width + (portraitSlot == nil ? 24 : 14), maximumWidth ?? .greatestFiniteMagnitude),
                  height: max(44, contentSize.height))
  }

  override func traitCollectionDidChange(_ previousTraitCollection: UITraitCollection?) {
    super.traitCollectionDidChange(previousTraitCollection)
    if previousTraitCollection?.preferredContentSizeCategory != traitCollection.preferredContentSizeCategory {
      updateFonts()
      invalidateIntrinsicContentSize()
      setNeedsLayout()
    }
  }

  override func sizeThatFits(_ size: CGSize) -> CGSize {
    let desired = intrinsicContentSize
    return CGSize(width: min(size.width, desired.width), height: desired.height)
  }

  private func updateFonts() {
    // Compact navigation remains a 44pt toolbar. Scale within that budget;
    // the complete title/subtitle remains available as one VoiceOver label.
    titleLabel.font = UIFontMetrics(forTextStyle: .headline).scaledFont(
      for: .systemFont(ofSize: 17, weight: .semibold), maximumPointSize: 20,
      compatibleWith: traitCollection)
    subtitleLabel.font = UIFontMetrics(forTextStyle: .caption1).scaledFont(
      for: .systemFont(ofSize: 12), maximumPointSize: 14,
      compatibleWith: traitCollection)
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    layer.cornerRadius = bounds.height / 2
  }
}


/// The unobstructed navigation row beside an inner display's status cluster.
/// Window geometry remains stable while individual routes slide on/off screen.
func duoPortraitHeader(in window: UIWindow?) -> CGRect? {
  #if canImport(UIKit, _version: 9127.0.85)
  if #available(iOS 27.1, *), let window,
     window.bounds.width >= 600, window.bounds.height > window.bounds.width {
    let bounds = window.bounds
    let regions = window.reservedRegions(kind: .occlusion).filter {
      $0.isActive && $0.frame.minY < bounds.height / 3
        && $0.frame.maxY < bounds.height / 3
        && $0.frame.maxX >= bounds.maxX - 1 && $0.frame.minX > bounds.midX
    }
    guard let region = regions.min(by: { $0.frame.minY < $1.frame.minY }) else { return nil }
    let status = window.windowScene?.statusBarManager?.statusBarFrame ?? .zero
    let centerY = !status.isEmpty && region.frame.contains(CGPoint(x: status.midX, y: status.midY))
      ? status.midY : region.frame.inset(by: region.margins).midY
    let row = CGRect(x: 0, y: max(0, centerY - 28), width: region.frame.minX, height: 56)
    return row
  }
  #endif
  return nil
}

func portraitControlRow(_ row: CGRect, in bounds: CGRect) -> CGRect? {
  let clipped = row.intersection(bounds)
  guard !clipped.isNull, !clipped.isEmpty,
        clipped.midY.isFinite, clipped.maxX.isFinite else { return nil }
  return clipped
}
