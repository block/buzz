import Flutter
import UIKit

@available(iOS 26.0, *)
extension NativeAttachmentPopoverViewController {
  func makeMenuView() -> UIView {
    let container = UIView()
    container.translatesAutoresizingMaskIntoConstraints = false

    let scrollView = UIScrollView()
    scrollView.alwaysBounceVertical = false
    scrollView.translatesAutoresizingMaskIntoConstraints = false
    container.addSubview(scrollView)

    let stack = UIStackView()
    stack.axis = .vertical
    stack.distribution = .fillEqually
    stack.spacing = NativeAttachmentMenuLayout.itemSpacing
    stack.translatesAutoresizingMaskIntoConstraints = false
    scrollView.addSubview(stack)
    let stackHeightConstraint = stack.heightAnchor.constraint(
      equalToConstant: NativeAttachmentMenuLayout.itemsHeight(
        compatibleWith: traitCollection
      )
    )
    menuStackHeightConstraint = stackHeightConstraint
    NSLayoutConstraint.activate([
      scrollView.leadingAnchor.constraint(equalTo: container.leadingAnchor),
      scrollView.trailingAnchor.constraint(equalTo: container.trailingAnchor),
      scrollView.topAnchor.constraint(equalTo: container.topAnchor),
      scrollView.bottomAnchor.constraint(equalTo: container.bottomAnchor),
      stack.leadingAnchor.constraint(
        equalTo: scrollView.frameLayoutGuide.leadingAnchor,
        constant: NativeAttachmentMenuLayout.contentPadding
      ),
      stack.trailingAnchor.constraint(
        equalTo: scrollView.frameLayoutGuide.trailingAnchor,
        constant: -NativeAttachmentMenuLayout.contentPadding
      ),
      stack.topAnchor.constraint(
        equalTo: scrollView.contentLayoutGuide.topAnchor,
        constant: NativeAttachmentMenuLayout.contentPadding
      ),
      stack.bottomAnchor.constraint(
        equalTo: scrollView.contentLayoutGuide.bottomAnchor,
        constant: -NativeAttachmentMenuLayout.contentPadding
      ),
      stackHeightConstraint,
    ])

    stack.addArrangedSubview(
      makeNativeAttachmentMenuButton(
        title: "Camera",
        symbol: "camera",
        action: { [weak self] in self?.showCamera() }
      )
    )
    stack.addArrangedSubview(
      makeNativeAttachmentMenuButton(
        title: "Photos",
        symbol: "photo.on.rectangle.angled",
        action: { [weak self] in self?.showPhotos() }
      )
    )
    stack.addArrangedSubview(
      makeNativeAttachmentMenuButton(
        title: "Video",
        symbol: "video",
        action: { [weak self] in
          self?.finish(method: "pickVideo")
        }
      )
    )
    stack.addArrangedSubview(
      makeNativeAttachmentMenuButton(
        title: "Voice note",
        symbol: "mic",
        action: { [weak self] in
          self?.finish(
            method: "recordVoiceNote",
            notifyBeforeDismissal: true
          )
        }
      )
    )
    stack.addArrangedSubview(
      makeNativeAttachmentMenuButton(
        title: "Files",
        symbol: "doc",
        action: { [weak self] in
          self?.finish(method: "pickFiles")
        }
      )
    )
    return container
  }

  func makeGlassControl(
    title: String?,
    symbol: String?,
    accessibilityLabel: String,
    prominent: Bool = false,
    action: @escaping () -> Void
  ) -> UIButton {
    var configuration =
      prominent
      ? UIButton.Configuration.prominentGlass()
      : UIButton.Configuration.glass()
    configuration.title = title
    if prominent {
      configuration.baseBackgroundColor = .black
    }
    if let symbol {
      configuration.image = UIImage(systemName: symbol)
    }
    configuration.imagePadding = 8
    configuration.baseForegroundColor = .white
    configuration.titleTextAttributesTransformer =
      UIConfigurationTextAttributesTransformer { attributes in
        var interAttributes = attributes
        interAttributes.font = NativeAttachmentMenuTypography.font(
          forTextStyle: .body
        )
        return interAttributes
      }
    configuration.contentInsets = NSDirectionalEdgeInsets(
      top: 11,
      leading: 15,
      bottom: 11,
      trailing: 15
    )
    let button = UIButton(
      configuration: configuration,
      primaryAction: UIAction { _ in
        UISelectionFeedbackGenerator().selectionChanged()
        action()
      }
    )
    button.accessibilityLabel = accessibilityLabel
    return button
  }
}
