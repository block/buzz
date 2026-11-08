import Foundation

/// Shared geometry for native pagination drawing and pointer selection.
struct ThemePaginationGeometry {
  static let dotSize: CGFloat = 6
  static let pitch: CGFloat = 12
  let count: Int
  let selected: Int
  let width: CGFloat
  let isRTL: Bool

  var visibleCount: Int { min(count, 7) }
  var windowStart: Int { min(max(0, selected - visibleCount / 2), count - visibleCount) }
  var hasEarlierDots: Bool { windowStart > 0 }
  var hasLaterDots: Bool { windowStart + visibleCount < count }
  private var firstCenter: CGFloat {
    let trackWidth = CGFloat(visibleCount - 1) * Self.pitch + Self.dotSize
    return (width - trackWidth) / 2 + Self.dotSize / 2
  }

  func slot(for page: Int) -> Int {
    let logicalSlot = page - windowStart
    return isRTL ? visibleCount - 1 - logicalSlot : logicalSlot
  }

  func centerX(for page: Int) -> CGFloat {
    firstCenter + CGFloat(slot(for: page)) * Self.pitch
  }

  func page(at x: CGFloat) -> Int {
    let slot = Int(((x - firstCenter) / Self.pitch).rounded())
    let boundedSlot = min(max(0, slot), visibleCount - 1)
    return windowStart + (isRTL ? visibleCount - 1 - boundedSlot : boundedSlot)
  }
}

/// Freezes the rendered window for one continuous pan, including its final event.
struct ThemePaginationScrub {
  private(set) var geometry: ThemePaginationGeometry?

  mutating func begin(_ geometry: ThemePaginationGeometry) {
    self.geometry = geometry
  }

  func page(at x: CGFloat, current: ThemePaginationGeometry) -> Int {
    (geometry ?? current).page(at: x)
  }

  mutating func end() {
    geometry = nil
  }
}
