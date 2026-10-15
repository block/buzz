import UIKit

/// Signal-style context presentation composed entirely of public UIKit views.
/// Flutter supplies an anchor and action IDs; it retains all mutation authority.
final class NativeMessageMenuViewController: UIViewController {
  var onPreviewReady: ((@escaping () -> Void) -> Void)?
  var onSelect: (([String: String]) -> Void)?
  private let data: [String: Any]
  private let sourceRect: CGRect
  private let preview: UIView
  private let backdrop = UIVisualEffectView()
  private let content = UIStackView()
  private let scroll = NativeMessageScrollView()
  private var contentTop: NSLayoutConstraint?
  private var closing = false
  private var previewHost: UIView?
  private var actionMenu: UIView?
  private var sourceTransform = CGAffineTransform.identity
  private var reactionTray: UIView?
  private var reactionButtons: [UIView] = []

  static func sourceRect(_ data: [String: Any]) -> CGRect? {
    guard let x = data["x"] as? Double, let y = data["y"] as? Double,
      let width = data["width"] as? Double, let height = data["height"] as? Double,
      x.isFinite, y.isFinite, width.isFinite, height.isFinite,
      width > 0, height > 0 else { return nil }
    return CGRect(x: x, y: y, width: width, height: height)
  }

  init(data: [String: Any], sourceRect: CGRect, preview: UIView) {
    self.data = data
    self.sourceRect = sourceRect
    self.preview = preview
    super.init(nibName: nil, bundle: nil)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = .clear
    view.accessibilityViewIsModal = true
    backdrop.frame = view.bounds
    backdrop.autoresizingMask = [.flexibleWidth, .flexibleHeight]
    view.addSubview(backdrop)
    let dismiss = UIButton(primaryAction: UIAction { [weak self] _ in self?.select([:]) })
    dismiss.accessibilityLabel = "Dismiss message menu"
    dismiss.frame = view.bounds
    dismiss.autoresizingMask = [.flexibleWidth, .flexibleHeight]
    view.addSubview(dismiss)

    scroll.translatesAutoresizingMaskIntoConstraints = false
    scroll.showsVerticalScrollIndicator = false
    scroll.contentInsetAdjustmentBehavior = .never
    // Keep all painted actions inside the tappable viewport. Inner padding
    // gives glass shadows space without exposing off-viewport content.
    scroll.clipsToBounds = true
    view.addSubview(scroll)
    content.axis = .vertical
    content.spacing = 12
    content.alignment = .leading
    content.translatesAutoresizingMaskIntoConstraints = false
    scroll.addSubview(content)
    scroll.passthroughView = content
    contentTop = content.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor)
    contentTop?.isActive = true
    let safe = view.safeAreaLayoutGuide
    NSLayoutConstraint.activate([
      scroll.leadingAnchor.constraint(equalTo: safe.leadingAnchor),
      scroll.trailingAnchor.constraint(equalTo: safe.trailingAnchor),
      scroll.topAnchor.constraint(equalTo: safe.topAnchor, constant: 12),
      scroll.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor, constant: -12),
      content.leadingAnchor.constraint(equalTo: scroll.contentLayoutGuide.leadingAnchor, constant: 16),
      content.trailingAnchor.constraint(equalTo: scroll.contentLayoutGuide.trailingAnchor, constant: -16),
      content.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor, constant: -48),
      content.widthAnchor.constraint(equalTo: scroll.frameLayoutGuide.widthAnchor, constant: -32),
    ])
    let reactionTray = makeReactionTray()
    self.reactionTray = reactionTray
    content.addArrangedSubview(reactionTray)
    reactionTray.widthAnchor.constraint(lessThanOrEqualTo: content.widthAnchor).isActive = true
    let trayWidth = reactionTray.widthAnchor.constraint(equalToConstant: 324)
    trayWidth.priority = .defaultHigh
    trayWidth.isActive = true
    // Preserve the actual message rendering, including media and markdown.
    preview.layer.cornerRadius = 0
    preview.layer.cornerCurve = .continuous
    preview.clipsToBounds = true
    preview.isAccessibilityElement = true
    preview.accessibilityLabel = data["previewLabel"] as? String
    let previewHost = UIView()
    self.previewHost = previewHost
    previewHost.clipsToBounds = true
    previewHost.layer.cornerRadius = 0
    previewHost.addSubview(preview)
    content.addArrangedSubview(previewHost)
    let width = min(sourceRect.width, max(1, view.bounds.width - 32))
    let height = min(sourceRect.height * width / sourceRect.width, 200)
    previewHost.widthAnchor.constraint(equalTo: content.widthAnchor).isActive = true
    previewHost.heightAnchor.constraint(equalToConstant: height).isActive = true
    preview.frame = CGRect(x: 0, y: 0, width: width, height: sourceRect.height * width / sourceRect.width)
    let actions = data["actions"] as? [[String: Any]] ?? []
    if !actions.isEmpty {
      let menu = NativeMessageSurface(radius: 16)
      actionMenu = menu
      let rows = UIStackView()
      rows.axis = .vertical
      for (index, action) in actions.enumerated() {
        if index > 0 {
          let divider = UIView()
          divider.backgroundColor = .separator
          divider.heightAnchor.constraint(equalToConstant: 1 / traitCollection.displayScale).isActive = true
          rows.addArrangedSubview(divider)
        }
        let button = UIButton(primaryAction: UIAction { [weak self] _ in
          guard let id = action["id"] as? String else { return }
          self?.select(["action": id])
        })
        let color: UIColor = action["destructive"] as? Bool == true ? .systemRed : .label
        let title = UILabel()
        title.text = action["title"] as? String
        title.font = .preferredFont(forTextStyle: .body)
        title.adjustsFontForContentSizeCategory = true
        title.numberOfLines = 0
        title.textColor = color
        let icon = UIImageView(image: UIImage(systemName: action["symbol"] as? String ?? "ellipsis",
          withConfiguration: UIImage.SymbolConfiguration(pointSize: 18, weight: .regular)))
        icon.tintColor = color
        icon.contentMode = .center
        icon.widthAnchor.constraint(equalToConstant: 24).isActive = true
        let row = UIStackView(arrangedSubviews: [title, icon])
        row.alignment = .center
        row.spacing = 20
        row.isUserInteractionEnabled = false
        row.translatesAutoresizingMaskIntoConstraints = false
        button.addSubview(row)
        NSLayoutConstraint.activate([
          row.leadingAnchor.constraint(equalTo: button.leadingAnchor, constant: 16),
          row.trailingAnchor.constraint(equalTo: button.trailingAnchor, constant: -16),
          row.topAnchor.constraint(equalTo: button.topAnchor, constant: 12),
          row.bottomAnchor.constraint(equalTo: button.bottomAnchor, constant: -12),
          button.heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
        ])
        button.accessibilityLabel = title.text
        button.configurationUpdateHandler = { button in
          button.backgroundColor = button.isHighlighted ? .tertiarySystemFill : .clear
        }
        rows.addArrangedSubview(button)
      }
      menu.embed(rows)
      content.addArrangedSubview(menu)
      menu.widthAnchor.constraint(lessThanOrEqualTo: content.widthAnchor).isActive = true
      let menuWidth = menu.widthAnchor.constraint(equalToConstant: 280)
      menuWidth.priority = .defaultHigh
      menuWidth.isActive = true
    }
  }

  private func makeReactionTray() -> UIView {
    let surface = NativeMessageSurface(radius: 30)
    let trayScroll = UIScrollView()
    trayScroll.showsHorizontalScrollIndicator = false
    let row = UIStackView()
    row.spacing = 4
    for reaction in data["reactions"] as? [[String: Any]] ?? [] {
      guard let emoji = reaction["emoji"] as? String else { continue }
      let button = UIButton(primaryAction: UIAction { [weak self] _ in
        self?.select(["action": "reaction", "emoji": emoji])
      })
      button.accessibilityLabel = reaction["label"] as? String ?? emoji
      let selected = reaction["selected"] as? Bool == true
      button.accessibilityValue = selected ? "Selected. Remove reaction" : "Add reaction"
      button.backgroundColor = selected ? .tertiarySystemFill : .clear
      button.layer.cornerRadius = 24
      let glyph = NativeMessageGlyph(data: reaction, size: 28)
      glyph.isUserInteractionEnabled = false
      button.addSubview(glyph)
      glyph.translatesAutoresizingMaskIntoConstraints = false
      NSLayoutConstraint.activate([
        glyph.centerXAnchor.constraint(equalTo: button.centerXAnchor),
        glyph.centerYAnchor.constraint(equalTo: button.centerYAnchor),
        glyph.widthAnchor.constraint(equalToConstant: 32),
        glyph.heightAnchor.constraint(equalToConstant: 32),
        button.widthAnchor.constraint(equalToConstant: 48),
        button.heightAnchor.constraint(equalToConstant: 48),
      ])
      row.addArrangedSubview(button)
      reactionButtons.append(button)
    }
    let more = UIButton(primaryAction: UIAction { [weak self] _ in self?.select(["action": "more"]) })
    more.setImage(UIImage(systemName: "plus"), for: .normal)
    more.tintColor = .secondaryLabel
    more.accessibilityLabel = "More emoji"
    more.widthAnchor.constraint(equalToConstant: 48).isActive = true
    row.addArrangedSubview(more)
    reactionButtons.append(more)
    row.translatesAutoresizingMaskIntoConstraints = false
    trayScroll.addSubview(row)
    NSLayoutConstraint.activate([
      row.leadingAnchor.constraint(equalTo: trayScroll.contentLayoutGuide.leadingAnchor, constant: 6),
      row.trailingAnchor.constraint(equalTo: trayScroll.contentLayoutGuide.trailingAnchor, constant: -6),
      row.topAnchor.constraint(equalTo: trayScroll.contentLayoutGuide.topAnchor, constant: 6),
      row.bottomAnchor.constraint(equalTo: trayScroll.contentLayoutGuide.bottomAnchor, constant: -6),
      row.heightAnchor.constraint(equalTo: trayScroll.frameLayoutGuide.heightAnchor, constant: -12),
    ])
    surface.embed(trayScroll)
    surface.heightAnchor.constraint(equalToConstant: 60).isActive = true
    return surface
  }

  override func viewDidLayoutSubviews() {
    super.viewDidLayoutSubviews()
    // The scroll view lays out its content after this callback. Measure the
    // complete stack explicitly so the first presentation cannot use a stale
    // (or zero) height and leave actions below the screen.
    let size = content.systemLayoutSizeFitting(
      CGSize(width: max(1, scroll.bounds.width - 32), height: UIView.layoutFittingCompressedSize.height),
      withHorizontalFittingPriority: .required,
      verticalFittingPriority: .fittingSizeLevel)
    let available = max(0, scroll.bounds.height - size.height - 48)
    let top = max(32, min(sourceRect.minY - scroll.frame.minY - 72, available))
    if contentTop?.constant != top {
      contentTop?.constant = top
      scroll.layoutIfNeeded()
    }
  }

  override func viewWillAppear(_ animated: Bool) {
    super.viewWillAppear(animated)
    reactionTray?.alpha = 0
    actionMenu?.alpha = 0
    previewHost?.alpha = 0
  }

  override func viewDidAppear(_ animated: Bool) {
    super.viewDidAppear(animated)
    guard !closing, previewHost != nil else { return }
    view.layoutIfNeeded()
    if let onPreviewReady {
      onPreviewReady { [weak self] in self?.animateIn() }
    } else {
      animateIn()
    }
  }

  private func animateIn() {
    guard !closing, let previewHost else { return }
    let reducedMotion = UIAccessibility.isReduceMotionEnabled
    let target = previewHost.convert(previewHost.bounds, to: view)
    sourceTransform = CGAffineTransform(
      translationX: sourceRect.minX - target.minX,
      y: sourceRect.minY - target.minY)
    if !reducedMotion {
      previewHost.transform = sourceTransform
      actionMenu?.transform = CGAffineTransform(translationX: 0, y: sourceRect.minY - target.minY)
        .scaledBy(x: 0.95, y: 0.95)
      for button in reactionButtons {
        button.alpha = 0
        button.transform = CGAffineTransform(translationX: 0, y: 24)
      }
    }
    previewHost.alpha = 1
    UIView.animate(withDuration: reducedMotion ? 0 : 0.14) {
      self.backdrop.effect = UIBlurEffect(style: .systemUltraThinMaterial)
    }
    UIView.animate(withDuration: reducedMotion ? 0 : 0.25, delay: 0,
      usingSpringWithDamping: 0.8, initialSpringVelocity: 1,
      options: [.beginFromCurrentState, .allowUserInteraction]) {
      previewHost.transform = .identity
      self.actionMenu?.transform = .identity
      self.actionMenu?.alpha = 1
    } completion: { _ in
      guard !self.closing else { return }
      UIAccessibility.post(notification: .screenChanged, argument: self.content)
    }
    UIView.animate(withDuration: reducedMotion ? 0 : 0.2) {
      self.reactionTray?.alpha = 1
    }
    guard !reducedMotion else { return }
    for (index, button) in reactionButtons.enumerated() {
      UIView.animate(withDuration: 0.2, delay: Double(index) * 0.01,
        options: [.beginFromCurrentState, .curveEaseOut, .allowUserInteraction]) {
        button.alpha = 1
        button.transform = .identity
      }
    }
  }

  override func viewWillTransition(to size: CGSize, with coordinator: UIViewControllerTransitionCoordinator) {
    super.viewWillTransition(to: size, with: coordinator)
    select([:]) // The Flutter anchor is no longer valid after rotation.
  }

  override func accessibilityPerformEscape() -> Bool { select([:]); return true }

  private func select(_ value: [String: String]) {
    guard !closing else { return }
    closing = true
    onSelect?(value)
  }

  func animateOut(completion: @escaping () -> Void) {
    UIView.animate(withDuration: UIAccessibility.isReduceMotionEnabled ? 0 : 0.16,
      delay: 0, options: [.beginFromCurrentState, .curveEaseOut]) {
      self.reactionTray?.alpha = 0
      self.actionMenu?.alpha = 0
      if !UIAccessibility.isReduceMotionEnabled {
        self.previewHost?.transform = self.sourceTransform
        self.actionMenu?.transform = CGAffineTransform(scaleX: 0.95, y: 0.95)
      }
      self.backdrop.effect = nil
    } completion: { _ in completion() }
  }
}

final class NativeMessageSurface: UIVisualEffectView {
  init(radius: CGFloat) {
    let effect: UIVisualEffect
    if #available(iOS 26.0, *) {
      effect = UIGlassEffect(style: .regular)
    } else {
      effect = UIBlurEffect(style: .systemMaterial)
    }
    super.init(effect: effect)
    layer.cornerRadius = radius
    layer.cornerCurve = .continuous
    clipsToBounds = true
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }

  func embed(_ child: UIView) {
    child.translatesAutoresizingMaskIntoConstraints = false
    contentView.addSubview(child)
    NSLayoutConstraint.activate([
      child.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
      child.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
      child.topAnchor.constraint(equalTo: contentView.topAnchor),
      child.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
    ])
  }
}

/// Transparent space around the cards belongs to the dismissal backdrop.
private final class NativeMessageScrollView: UIScrollView {
  weak var passthroughView: UIView?
  override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
    let hit = super.hitTest(point, with: event)
    return hit === self || hit === passthroughView ? nil : hit
  }
}
