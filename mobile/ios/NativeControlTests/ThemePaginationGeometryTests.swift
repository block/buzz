import Foundation

@main
struct ThemePaginationGeometryTests {
  static func check(_ condition: Bool, _ message: String) {
    if !condition {
      print("FAIL: \(message)")
      exit(1)
    }
  }

  static func main() {
    let windows = [(3, 1, 0), (20, 0, 0), (20, 10, 7), (20, 19, 13)]
    var checkedCenters = 0
    for isRTL in [false, true] {
      for (count, selected, firstPage) in windows {
        // Actual dot centers inside the 92-point padded track.
        let centers: [CGFloat] = count == 3 ? [34, 46, 58] : [10, 22, 34, 46, 58, 70, 82]
        let geometry = ThemePaginationGeometry(count: count, selected: selected, width: 92, isRTL: isRTL)
        check(geometry.windowStart == firstPage, "wrong window at \(selected)")
        for (slot, center) in centers.enumerated() {
          let page = firstPage + (isRTL ? centers.count - 1 - slot : slot)
          check(geometry.centerX(for: page) == center, "wrong rendered center for page \(page)")
          for offset: CGFloat in [-2, 0, 2] {
            check(geometry.page(at: center + offset) == page,
              "count=\(count), selected=\(selected), RTL=\(isRTL), tap=\(center + offset), expected=\(page)")
          }
          checkedCenters += 1
        }
        check(geometry.page(at: -100) == firstPage + (isRTL ? centers.count - 1 : 0), "left edge")
        check(geometry.page(at: 200) == firstPage + (isRTL ? 0 : centers.count - 1), "right edge")
      }
    }
    for isRTL in [false, true] {
      for cancel in [false, true] {
        var selected = 10
        func current() -> ThemePaginationGeometry {
          ThemePaginationGeometry(count: 20, selected: selected, width: 92, isRTL: isRTL)
        }
        var scrub = ThemePaginationScrub()
        let target = current().centerX(for: 12)
        scrub.begin(current())
        for _ in 0..<8 {
          selected = scrub.page(at: target, current: current())
          check(selected == 12, "held pan cascaded in RTL=\(isRTL), cancel=\(cancel)")
          check(scrub.geometry?.centerX(for: 12) == target, "rendered window moved")
        }
        // The UIKit ended path consumes its last coordinate before clearing;
        // cancelled/failed paths clear without introducing another selection.
        if !cancel { selected = scrub.page(at: target, current: current()) }
        scrub.end()
        check(scrub.geometry == nil, "end/cancel retained window")
        let nextTarget = current().centerX(for: 14)
        scrub.begin(current())
        selected = scrub.page(at: nextTarget, current: current())
        check(selected == 14, "next gesture reused stale geometry")
        scrub.end()
        check(current().page(at: current().centerX(for: 15)) == 15, "tap after scrub")
      }
    }
    print("PASS: held scrubs, final events, cancellation and fresh gestures in LTR/RTL")
    print("PASS: \(checkedCenters) rendered centers, neighboring taps and clamped edges in LTR/RTL")
  }
}
