# UIKit media fixtures

These 2 x 2 fixtures were produced on an iOS simulator with UIKit, not by a generic image encoder.

## Regeneration

1. Create a small source image and run this program against the simulator SDK:

   ```swift
   import Foundation
   import UIKit

   let arguments = CommandLine.arguments
   let source = try Data(contentsOf: URL(fileURLWithPath: arguments[1]))
   guard
     let image = UIImage(data: source),
     let png = image.pngData(),
     let jpeg = image.jpegData(compressionQuality: 1.0)
   else {
     fatalError("UIKit could not encode the source image")
   }
   try png.write(to: URL(fileURLWithPath: arguments[2]))
   try jpeg.write(to: URL(fileURLWithPath: arguments[3]))
   ```

   Compile and run it with the active Xcode toolchain:

   ```sh
   SDK_PATH="$(xcrun --sdk iphonesimulator --show-sdk-path)"
   xcrun --sdk iphonesimulator swiftc \
     -sdk "$SDK_PATH" \
     -target arm64-apple-ios16.0-simulator \
     reencode.swift -o reencode
   xcrun simctl spawn booted ./reencode \
     source.png uikit-encoded.png uikit-encoded.jpg
   ```

2. Copy the encoded files into both fixture directories:

   ```sh
   cp uikit-encoded.png mobile/ios/RunnerTests/Fixtures/UIKitEncoded.png
   cp uikit-encoded.jpg mobile/ios/RunnerTests/Fixtures/UIKitEncoded.jpg
   cp uikit-encoded.png crates/buzz-media/tests/fixtures/ios/
   cp uikit-encoded.jpg crates/buzz-media/tests/fixtures/ios/
   ```

3. Add a temporary Runner test that loads `UIKitEncoded.png` and `UIKitEncoded.jpg`, calls `MediaSanitizer.scrubPng` and `MediaSanitizer.scrubJpeg`, and writes those outputs to `uikit-sanitized.png` and `uikit-sanitized.jpg`. Run it once, copy the files here, then remove the temporary test.
4. Run `cmp` on each encoded copy to confirm that the Runner and Rust fixtures are byte-identical.
5. Run `cargo test -p buzz-media test_ios_uikit` to verify that UIKit's encoded output is rejected and the matching sanitizer output is accepted by the relay contract.

Regenerate both encoded and sanitized pairs whenever UIKit encoding or `MediaSanitizer` changes. Do not update only the sanitized files, because the test is intended to cover the exact encoder-to-sanitizer boundary.
