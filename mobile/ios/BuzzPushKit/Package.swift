// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "BuzzPushKit",
    platforms: [.iOS(.v15), .macOS(.v12)],
    products: [
        .library(name: "BuzzPushKit", targets: ["BuzzPushKit"])
    ],
    targets: [
        .target(name: "BuzzPushKit"),
        .testTarget(name: "BuzzPushKitTests", dependencies: ["BuzzPushKit"]),
    ]
)
