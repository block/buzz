import Flutter
import UIKit
import XCTest
@testable import Buzz

class RunnerTests: XCTestCase {

  func testClipboardImageDataPrefersOriginalPngBytes() throws {
    let pasteboard = try XCTUnwrap(
      UIPasteboard(name: UIPasteboard.Name(UUID().uuidString), create: true)
    )
    defer { UIPasteboard.remove(withName: pasteboard.name) }
    let pngData = Data([0x89, 0x50, 0x4E, 0x47])
    let jpegData = Data([0xFF, 0xD8, 0xFF])
    pasteboard.setItems([
      ["public.png": pngData, "public.jpeg": jpegData]
    ])

    XCTAssertEqual(AppDelegate.clipboardImageData(from: pasteboard), pngData)
  }

  func testClipboardImageDataPreservesOriginalWebPBytesForValidation() throws {
    let pasteboard = try XCTUnwrap(
      UIPasteboard(name: UIPasteboard.Name(UUID().uuidString), create: true)
    )
    defer { UIPasteboard.remove(withName: pasteboard.name) }
    let webPData = Data("RIFFxxxxWEBP".utf8)
    pasteboard.setData(webPData, forPasteboardType: "org.webmproject.webp")

    XCTAssertEqual(AppDelegate.clipboardImageData(from: pasteboard), webPData)
  }

  func testClipboardImageDataPreservesOriginalGifBytesForValidation() throws {
    let pasteboard = try XCTUnwrap(
      UIPasteboard(name: UIPasteboard.Name(UUID().uuidString), create: true)
    )
    defer { UIPasteboard.remove(withName: pasteboard.name) }
    let gifData = Data("GIF89a".utf8)
    pasteboard.setData(gifData, forPasteboardType: "com.compuserve.gif")

    XCTAssertEqual(AppDelegate.clipboardImageData(from: pasteboard), gifData)
  }

  func testClipboardImageDataReturnsNilWithoutAnImage() throws {
    let pasteboard = try XCTUnwrap(
      UIPasteboard(name: UIPasteboard.Name(UUID().uuidString), create: true)
    )
    defer { UIPasteboard.remove(withName: pasteboard.name) }
    pasteboard.string = "text only"

    XCTAssertNil(AppDelegate.clipboardImageData(from: pasteboard))
  }

}
