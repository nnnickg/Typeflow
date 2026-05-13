// swift-tools-version: 5.9

import Foundation
import PackageDescription

let environment = ProcessInfo.processInfo.environment
let cargoTargetDirectory = environment["CARGO_TARGET_DIR"] ?? "../target"
let rustProfile = environment["RUST_PROFILE"] == "debug" ? "debug" : "release"
let rustStaticLibrary = "\(cargoTargetDirectory)/\(rustProfile)/libtypeflow_ffi.a"
let rustStaticLinkerFlags = ["-Xlinker", "-force_load", "-Xlinker", rustStaticLibrary]

let package = Package(
    name: "TypeflowMacOS",
    platforms: [
        .macOS(.v13),
    ],
    products: [
        .library(name: "TypeflowKit", targets: ["TypeflowKit"]),
        .executable(name: "typeflow-staticlib-smoke", targets: ["TypeflowSmoke"]),
        .executable(name: "Typeflow", targets: ["TypeflowAgent"]),
    ],
    targets: [
        .systemLibrary(
            name: "TypeflowFFI",
            path: "TypeflowFFI/include"
        ),
        .target(
            name: "TypeflowKit",
            dependencies: ["TypeflowFFI"],
            path: "Sources/TypeflowKit"
        ),
        .executableTarget(
            name: "TypeflowSmoke",
            dependencies: ["TypeflowKit", "TypeflowFFI"],
            path: "Sources/TypeflowSmoke",
            linkerSettings: [
                .unsafeFlags(rustStaticLinkerFlags),
                .linkedFramework("Carbon"),
            ]
        ),
        .executableTarget(
            name: "TypeflowAgent",
            dependencies: ["TypeflowKit", "TypeflowFFI"],
            path: "Sources/TypeflowAgent",
            linkerSettings: [
                .unsafeFlags(rustStaticLinkerFlags),
                .linkedFramework("AppKit"),
                .linkedFramework("ApplicationServices"),
                .linkedFramework("Carbon"),
            ]
        ),
    ]
)
