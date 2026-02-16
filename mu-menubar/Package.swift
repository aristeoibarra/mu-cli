// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "MuMenuBar",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "MuMenuBar", targets: ["MuMenuBar"])
    ],
    targets: [
        .executableTarget(
            name: "MuMenuBar",
            path: "Sources",
            swiftSettings: [
                .unsafeFlags(["-parse-as-library"])
            ]
        )
    ]
)
