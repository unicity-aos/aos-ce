// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "aos-tray",
    platforms: [
        .macOS(.v13),
    ],
    products: [
        .library(name: "AOSTrayCore", targets: ["AOSTrayCore"]),
        .executable(name: "aos-tray", targets: ["AOSTray"]),
    ],
    targets: [
        .target(
            name: "AOSTrayCore"
        ),
        .executableTarget(
            name: "AOSTray",
            dependencies: ["AOSTrayCore"],
            linkerSettings: [
                .linkedFramework("AppKit"),
                .linkedFramework("ServiceManagement"),
                .linkedFramework("SwiftUI"),
            ]
        ),
        .testTarget(
            name: "AOSTrayCoreTests",
            dependencies: ["AOSTrayCore"]
        ),
    ]
)
