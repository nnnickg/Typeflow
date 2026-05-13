import Foundation
import Carbon
#if SWIFT_PACKAGE
import TypeflowKit
#endif
import TypeflowFFI

enum SmokeError: Error, CustomStringConvertible {
    case wrongOutput(String)
    case wrongLayout(TypeflowLayout)
    case wrongDefaultMaxTokenLen(Int)
    case wrongPackDirectory(String?)
    case wrongDataDirectory(String?)
    case wrongSecondaryLanguage(String)
    case wrongAppPolicy(String)
    case wrongObservation(TypeflowObservationAction)

    var description: String {
        switch self {
        case let .wrongOutput(output):
            return "unexpected output: \(output)"
        case let .wrongLayout(layout):
            return "expected secondary layout, got \(layout)"
        case let .wrongDefaultMaxTokenLen(value):
            return "expected default max_token_len 128, got \(value)"
        case let .wrongPackDirectory(value):
            return "unexpected pack directory: \(value ?? "nil")"
        case let .wrongDataDirectory(value):
            return "unexpected data directory: \(value ?? "nil")"
        case let .wrongSecondaryLanguage(value):
            return "unexpected secondary language: \(value)"
        case let .wrongAppPolicy(value):
            return "unexpected app policy result: \(value)"
        case let .wrongObservation(action):
            return "unexpected observation action: \(action)"
        }
    }
}

func typeToken(_ engine: TypeflowEngine, keyCodes: [Int]) throws -> String {
    var hostText = ""
    for keyCode in keyCodes.map(UInt16.init) {
        guard let physical = TypeflowMacKeyCode.physicalKeyIndex(for: keyCode) else {
            throw SmokeError.wrongOutput("unmapped keycode \(keyCode)")
        }
        _ = try engine.observe(physicalKey: physical)
        hostText.append(try englishCharacter(for: keyCode))
    }
    _ = try engine.endToken()
    return hostText
}

func englishCharacter(for keyCode: UInt16) throws -> Character {
    switch Int(keyCode) {
    case kVK_ANSI_B: return "b"
    case kVK_ANSI_D: return "d"
    case kVK_ANSI_E: return "e"
    case kVK_ANSI_G: return "g"
    case kVK_ANSI_H: return "h"
    case kVK_ANSI_N: return "n"
    case kVK_ANSI_P: return "p"
    case kVK_ANSI_S: return "s"
    case kVK_ANSI_T: return "t"
    case kVK_ANSI_Y: return "y"
    default:
        throw SmokeError.wrongOutput("unmapped smoke keycode \(keyCode)")
    }
}

func fail(_ error: Error) -> Never {
    FileHandle.standardError.write(Data("staticlib smoke failed: \(error)\n".utf8))
    exit(1)
}

func verifyHostConfigPrecedence() throws {
    let root = URL(fileURLWithPath: NSTemporaryDirectory())
        .appendingPathComponent("typeflow-smoke-\(ProcessInfo.processInfo.processIdentifier)-\(UUID().uuidString)")
    let configPath = root
        .appendingPathComponent(".config")
        .appendingPathComponent("typeflow")
        .appendingPathComponent("config.toml")
    try FileManager.default.createDirectory(
        at: configPath.deletingLastPathComponent(),
        withIntermediateDirectories: true
    )
    defer {
        try? FileManager.default.removeItem(at: root)
    }

    try """
    [language]
    secondary = "pl"

    [packs]
    directory = "/config/packs"

    [data]
    directory = "/config/data"

    [apps]
    disable_bundle_ids = [
        "dev.zed.Zed",
        "com.apple.Terminal",
    ]

    disable_auto_bundle_ids = [
        "com.apple.Terminal",
        "com.apple.TextEdit",
    ]

    [macos]
    english_input_source_id = " com.apple.keylayout.ABC "
    secondary_input_source_id = " com.apple.keylayout.Ukrainian "
    """.write(to: configPath, atomically: true, encoding: .utf8)

    let config = try TypeflowHostConfig.load(environment: [
        "HOME": root.path,
        "TYPEFLOW_CONFIG": configPath.path,
        "TYPEFLOW_PACK_DIR": "/env/packs",
        "TYPEFLOW_DATA_DIR": "/env/data",
    ])

    guard config.packDirectory == "/env/packs" else {
        throw SmokeError.wrongPackDirectory(config.packDirectory)
    }
    guard config.dataDirectory == "/env/data" else {
        throw SmokeError.wrongDataDirectory(config.dataDirectory)
    }
    guard config.secondaryLanguage == "pl" else {
        throw SmokeError.wrongSecondaryLanguage(config.secondaryLanguage)
    }
    guard config.macOSEnglishInputSourceID == "com.apple.keylayout.ABC" else {
        throw SmokeError.wrongAppPolicy(
            "englishInputSourceID=\(config.macOSEnglishInputSourceID ?? "nil")"
        )
    }
    guard config.macOSSecondaryInputSourceID == "com.apple.keylayout.Ukrainian" else {
        throw SmokeError.wrongAppPolicy(
            "secondaryInputSourceID=\(config.macOSSecondaryInputSourceID ?? "nil")"
        )
    }
    guard config.disabledBundleIDCount == 2 else {
        throw SmokeError.wrongAppPolicy("disabledCount=\(config.disabledBundleIDCount)")
    }
    guard config.autoDisabledBundleIDCount == 1 else {
        throw SmokeError.wrongAppPolicy("autoDisabledCount=\(config.autoDisabledBundleIDCount)")
    }
    guard config.isBundleDisabled(bundleID: "dev.zed.Zed") else {
        throw SmokeError.wrongAppPolicy("dev.zed.Zed not fully disabled")
    }
    guard config.isAutomaticProcessingDisabled(bundleID: "dev.zed.Zed") else {
        throw SmokeError.wrongAppPolicy("dev.zed.Zed automatic processing not disabled")
    }
    guard config.isBundleDisabled(bundleID: "com.apple.Terminal") else {
        throw SmokeError.wrongAppPolicy("com.apple.Terminal not fully disabled")
    }
    guard config.isAutomaticProcessingDisabled(bundleID: "com.apple.Terminal") else {
        throw SmokeError.wrongAppPolicy("com.apple.Terminal automatic processing not disabled")
    }
    guard !config.isBundleDisabled(bundleID: "com.apple.TextEdit") else {
        throw SmokeError.wrongAppPolicy("com.apple.TextEdit fully disabled")
    }
    guard config.isAutomaticProcessingDisabled(bundleID: "com.apple.TextEdit") else {
        throw SmokeError.wrongAppPolicy("com.apple.TextEdit automatic processing not disabled")
    }
    guard !config.isAutomaticProcessingDisabled(bundleID: "com.apple.Safari") else {
        throw SmokeError.wrongAppPolicy("com.apple.Safari automatic processing disabled")
    }

    let terminalPolicy = config.resolveInputPolicy(
        facts: TypeflowHostSurfaceFacts(bundleID: "com.googlecode.iterm2")
    )
    guard terminalPolicy.automaticProcessingDisabled,
          terminalPolicy.manualSwitchDisabled,
          terminalPolicy.terminalSurface
    else {
        throw SmokeError.wrongAppPolicy("iTerm2 policy=\(terminalPolicy.reasonDescription)")
    }

    let embeddedTerminalPolicy = config.resolveInputPolicy(
        facts: TypeflowHostSurfaceFacts(
            bundleID: "com.apple.Safari",
            focusedElementIdentifier: "workspace-terminal-panel"
        )
    )
    guard embeddedTerminalPolicy.automaticProcessingDisabled,
          embeddedTerminalPolicy.manualSwitchDisabled,
          embeddedTerminalPolicy.terminalSurface
    else {
        throw SmokeError.wrongAppPolicy(
            "embeddedTerminal policy=\(embeddedTerminalPolicy.reasonDescription)"
        )
    }

    let autoDisabledPolicy = config.resolveInputPolicy(
        facts: TypeflowHostSurfaceFacts(bundleID: "com.apple.TextEdit")
    )
    guard autoDisabledPolicy.automaticProcessingDisabled,
          !autoDisabledPolicy.manualSwitchDisabled,
          !autoDisabledPolicy.terminalSurface
    else {
        throw SmokeError.wrongAppPolicy(
            "autoDisabled policy=\(autoDisabledPolicy.reasonDescription)"
        )
    }
}

func verifyMissingDefaultHostConfigUsesDefaults() throws {
    let root = URL(fileURLWithPath: NSTemporaryDirectory())
        .appendingPathComponent("typeflow-smoke-\(ProcessInfo.processInfo.processIdentifier)-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer {
        try? FileManager.default.removeItem(at: root)
    }

    let config = try TypeflowHostConfig.load(environment: [
        "HOME": root.path,
    ])

    guard config.sourcePath == nil else {
        throw SmokeError.wrongAppPolicy("sourcePath=\(config.sourcePath ?? "nil")")
    }
    guard config.secondaryLanguage == "uk" else {
        throw SmokeError.wrongSecondaryLanguage(config.secondaryLanguage)
    }
    guard config.engineSourceDescription == "embedded" else {
        throw SmokeError.wrongAppPolicy("engineSource=\(config.engineSourceDescription)")
    }
}

func verifyAutoDisabledManualLayoutMode() throws {
    let engine = try TypeflowEngine()
    engine.setHostContext(flags: typeflow_ffi_context_automatic_switching_disabled())

    let keyCodes = [kVK_ANSI_G, kVK_ANSI_H, kVK_ANSI_S, kVK_ANSI_D, kVK_ANSI_B, kVK_ANSI_N]
    let hostText = try typeToken(engine, keyCodes: keyCodes)
    guard hostText == "ghsdbn" else {
        throw SmokeError.wrongOutput(hostText)
    }
    guard try engine.currentLayout == .english else {
        throw SmokeError.wrongLayout(try engine.currentLayout)
    }

    engine.resetLayout(.secondary)
    _ = try typeToken(engine, keyCodes: keyCodes)
    guard try engine.currentLayout == .secondary else {
        throw SmokeError.wrongLayout(try engine.currentLayout)
    }
}

func verifyManualSwitchChangesFutureLayoutOnly() throws {
    let engine = try TypeflowEngine()
    let keyCodes = [kVK_ANSI_T, kVK_ANSI_Y, kVK_ANSI_P, kVK_ANSI_E]
    for keyCode in keyCodes.map(UInt16.init) {
        guard let physical = TypeflowMacKeyCode.physicalKeyIndex(for: keyCode) else {
            throw SmokeError.wrongOutput("unmapped keycode \(keyCode)")
        }
        _ = try engine.observe(physicalKey: physical)
    }

    let action = try engine.forceSwitchLayout()
    guard action == .switchFutureLayout(.secondary) else {
        throw SmokeError.wrongObservation(action)
    }
    guard let replacement = engine.takePendingReplacement() else {
        throw SmokeError.wrongOutput("missing manual switch replacement")
    }
    guard replacement.deleteCount == keyCodes.count else {
        throw SmokeError.wrongOutput("deleteCount=\(replacement.deleteCount)")
    }
    guard !replacement.text.isEmpty else {
        throw SmokeError.wrongOutput("empty manual switch replacement")
    }
    guard engine.tokenLength == 0 else {
        throw SmokeError.wrongOutput("expected token reset, got \(engine.tokenLength)")
    }
}

do {
    let config = TypeflowEngine.defaultConfig()
    guard config.max_token_len == 128 else {
        throw SmokeError.wrongDefaultMaxTokenLen(config.max_token_len)
    }
    try verifyHostConfigPrecedence()
    try verifyMissingDefaultHostConfigUsesDefaults()
    try verifyAutoDisabledManualLayoutMode()
    try verifyManualSwitchChangesFutureLayoutOnly()

    let engine = try TypeflowEngine()
    let keyCodes = [kVK_ANSI_G, kVK_ANSI_H, kVK_ANSI_S, kVK_ANSI_D, kVK_ANSI_B, kVK_ANSI_N]
    let hostText = try typeToken(engine, keyCodes: keyCodes)

    guard hostText == "ghsdbn" else {
        throw SmokeError.wrongOutput(hostText)
    }
    guard try engine.currentLayout == .secondary else {
        throw SmokeError.wrongLayout(try engine.currentLayout)
    }

    print("staticlib smoke: observed ghsdbn; host text pass-through \(hostText)")
} catch {
    fail(error)
}
