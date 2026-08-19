// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "BuzzNativeDriver",
    platforms: [.macOS(.v13)],
    products: [.executable(name: "buzz-native-driver", targets: ["BuzzNativeDriver"])],
    targets: [.executableTarget(name: "BuzzNativeDriver")]
)
