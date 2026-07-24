// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "BuzzPushKit",
    platforms: [.iOS(.v15), .macOS(.v12)],
    products: [
        .library(name: "BuzzPushKit", targets: ["BuzzPushKit"])
    ],
    dependencies: [
        .package(url: "https://github.com/21-DOT-DEV/swift-secp256k1.git", exact: "0.21.1")
    ],
    targets: [
        .target(
            name: "BuzzPushKit",
            dependencies: [.product(name: "P256K", package: "swift-secp256k1")]
        ),
        .testTarget(name: "BuzzPushKitTests", dependencies: ["BuzzPushKit"]),
    ]
)
