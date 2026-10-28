// swift-tools-version: 6.2

import PackageDescription

let package = Package(
    name: "SbobinoDiarizer",
    platforms: [
        .macOS(.v15),
    ],
    products: [
        .executable(name: "sbobino-diarizer", targets: ["SbobinoDiarizer"]),
    ],
    dependencies: [
        .package(
            url: "https://github.com/FluidInference/FluidAudio.git",
            revision: "793a2c5a0e99705a55fefbad00e65ca684329349",
            traits: []
        ),
    ],
    targets: [
        .executableTarget(
            name: "SbobinoDiarizer",
            dependencies: [
                .product(name: "FluidAudio", package: "FluidAudio"),
            ]
        ),
        .testTarget(
            name: "SbobinoDiarizerTests",
            dependencies: ["SbobinoDiarizer"]
        ),
    ]
)
