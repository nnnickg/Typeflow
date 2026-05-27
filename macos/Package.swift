// swift-tools-version: 5.9

import Foundation
import PackageDescription

let environment = ProcessInfo.processInfo.environment
let cargoTargetDirectory = environment["CARGO_TARGET_DIR"] ?? "../target"
let rustProfile = environment["RUST_PROFILE"] == "debug" ? "debug" : "release"
let rustStaticLibrary = "\(cargoTargetDirectory)/\(rustProfile)/libtypeclaw_ffi.a"
let rustStaticLinkerFlags = ["-Xlinker", "-force_load", "-Xlinker", rustStaticLibrary]

let package = Package(
    name: "TypeClawMacOS",
    platforms: [
        .macOS(.v13),
    ],
    products: [
        .library(name: "TypeClawKit", targets: ["TypeClawKit"]),
        .executable(name: "typeclaw-staticlib-smoke", targets: ["TypeClawSmoke"]),
        .executable(name: "TypeClaw", targets: ["TypeClawAgent"]),
    ],
    targets: [
        .systemLibrary(
            name: "TypeClawFFI",
            path: "TypeClawFFI/include"
        ),
        .target(
            name: "TypeClawKit",
            dependencies: ["TypeClawFFI"],
            path: "Sources/TypeClawKit"
        ),
        .executableTarget(
            name: "TypeClawSmoke",
            dependencies: ["TypeClawKit", "TypeClawFFI"],
            path: "Sources/TypeClawSmoke",
            linkerSettings: [
                .unsafeFlags(rustStaticLinkerFlags),
                .linkedFramework("Carbon"),
            ]
        ),
        .executableTarget(
            name: "TypeClawAgent",
            dependencies: ["TypeClawKit", "TypeClawFFI"],
            path: "Sources/TypeClawAgent",
            linkerSettings: [
                .unsafeFlags(rustStaticLinkerFlags),
                .linkedFramework("AppKit"),
                .linkedFramework("ApplicationServices"),
                .linkedFramework("Carbon"),
                .linkedFramework("ServiceManagement"),
                .linkedFramework("IOKit"),
            ]
        ),
    ]
)
