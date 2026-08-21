// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "BuzzNativeDriver",
    platforms: [.macOS(.v13)],
    products: [.executable(name: "buzz-native-driver", targets: ["BuzzNativeDriver"])],
    targets: [
        .target(name: "BuzzNativeDriverSupport"),
        .executableTarget(name: "BuzzNativeDriver", dependencies: ["BuzzNativeDriverSupport"]),
        .testTarget(name: "BuzzNativeDriverSupportTests", dependencies: ["BuzzNativeDriverSupport"]),
    ]
)
