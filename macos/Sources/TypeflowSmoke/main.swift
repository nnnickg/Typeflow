import Foundation
import Carbon
import TypeflowFFI

enum SmokeError: Error, CustomStringConvertible {
    case underflow(Int)
    case wrongOutput(String)
    case wrongLayout(TypeflowLayout)
    case wrongDefaultMaxTokenLen(Int)
    case wrongPackDirectory(String?)
    case wrongDataDirectory(String?)
    case wrongSecondaryLanguage(String)
    case wrongAppPolicy(String)
    case wrongAction(TypeflowAction)

    var description: String {
        switch self {
        case let .underflow(count):
            return "replace action tried to remove \(count) characters from a shorter buffer"
        case let .wrongOutput(output):
            return "expected привіт, got \(output)"
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
        case let .wrongAction(action):
            return "unexpected action: \(action)"
        }
    }
}

func apply(_ action: TypeflowAction, to buffer: inout String) throws {
    switch action {
    case .keep, .resetToken:
        break
    case let .commit(character):
        buffer.append(character)
    case let .replaceToken(oldLength, replacement, _):
        guard buffer.count >= oldLength else {
            throw SmokeError.underflow(oldLength)
        }
        for _ in 0..<oldLength {
            buffer.removeLast()
        }
        buffer.append(replacement)
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
          terminalPolicy.manualConversionDisabled,
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
          embeddedTerminalPolicy.manualConversionDisabled,
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
          !autoDisabledPolicy.manualConversionDisabled,
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

func verifyVisibleTailBridge() throws {
    let engine = try TypeflowEngine()
    guard let f = TypeflowMacKeyCode.physicalKeyIndex(for: UInt16(kVK_ANSI_F)) else {
        throw SmokeError.wrongOutput("unmapped keycode \(kVK_ANSI_F)")
    }
    guard let j = TypeflowMacKeyCode.physicalKeyIndex(for: UInt16(kVK_ANSI_J)) else {
        throw SmokeError.wrongOutput("unmapped keycode \(kVK_ANSI_J)")
    }

    let replace = try engine.replaceVisibleTail(
        "hello [fn",
        physicalKey: f,
        modifiers: 0,
        targetLayout: .secondary
    )
    guard replace == .replaceToken(oldLength: 3, replacement: "хата", layout: .secondary) else {
        throw SmokeError.wrongAction(replace)
    }

    let convert = try engine.convertVisibleTail("hello [fnf")
    guard convert == .replaceToken(oldLength: 4, replacement: "хата", layout: .secondary) else {
        throw SmokeError.wrongAction(convert)
    }

    let smartQuoteReplace = try engine.replaceVisibleTail(
        "hello ’dh",
        physicalKey: j,
        modifiers: 0,
        targetLayout: .secondary
    )
    guard smartQuoteReplace == .replaceToken(oldLength: 3, replacement: "євро", layout: .secondary) else {
        throw SmokeError.wrongAction(smartQuoteReplace)
    }

    let smartQuoteConvert = try engine.convertVisibleTail("hello ’dhj")
    guard smartQuoteConvert == .replaceToken(oldLength: 4, replacement: "євро", layout: .secondary) else {
        throw SmokeError.wrongAction(smartQuoteConvert)
    }
}

func verifyAutoDisabledManualLayoutMode() throws {
    let engine = try TypeflowEngine()
    engine.setHostContext(flags: typeflow_ffi_context_automatic_switching_disabled())

    let keyCodes = [kVK_ANSI_G, kVK_ANSI_H, kVK_ANSI_S, kVK_ANSI_D, kVK_ANSI_B, kVK_ANSI_N]
    var committed = ""
    for keyCode in keyCodes.map(UInt16.init) {
        guard let physical = TypeflowMacKeyCode.physicalKeyIndex(for: keyCode) else {
            throw SmokeError.wrongOutput("unmapped keycode \(keyCode)")
        }
        let action = try engine.process(physicalKey: physical)
        try apply(action, to: &committed)
    }
    guard committed == "ghsdbn" else {
        throw SmokeError.wrongOutput(committed)
    }

    engine.resetLayout(.secondary)
    committed = ""
    for keyCode in keyCodes.map(UInt16.init) {
        guard let physical = TypeflowMacKeyCode.physicalKeyIndex(for: keyCode) else {
            throw SmokeError.wrongOutput("unmapped keycode \(keyCode)")
        }
        let action = try engine.process(physicalKey: physical)
        try apply(action, to: &committed)
    }
    guard committed == "привіт" else {
        throw SmokeError.wrongOutput(committed)
    }
}

do {
    let config = TypeflowEngine.defaultConfig()
    guard config.max_token_len == 128 else {
        throw SmokeError.wrongDefaultMaxTokenLen(config.max_token_len)
    }
    try verifyHostConfigPrecedence()
    try verifyMissingDefaultHostConfigUsesDefaults()
    try verifyVisibleTailBridge()
    try verifyAutoDisabledManualLayoutMode()

    let engine = try TypeflowEngine()
    var committed = ""

    let keyCodes = [kVK_ANSI_G, kVK_ANSI_H, kVK_ANSI_S, kVK_ANSI_D, kVK_ANSI_B, kVK_ANSI_N]
    for keyCode in keyCodes.map(UInt16.init) {
        guard let physical = TypeflowMacKeyCode.physicalKeyIndex(for: keyCode) else {
            throw SmokeError.wrongOutput("unmapped keycode \(keyCode)")
        }
        let action = try engine.process(physicalKey: physical)
        try apply(action, to: &committed)
    }

    guard committed == "привіт" else {
        throw SmokeError.wrongOutput(committed)
    }
    guard try engine.currentLayout == .secondary else {
        throw SmokeError.wrongLayout(try engine.currentLayout)
    }

    print("staticlib smoke: ghsdbn -> \(committed)")
} catch {
    fail(error)
}
