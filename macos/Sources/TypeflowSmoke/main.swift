import Foundation
import Carbon

enum SmokeError: Error, CustomStringConvertible {
    case underflow(Int)
    case wrongOutput(String)
    case wrongLayout(TypeflowLayout)
    case wrongDefaultMaxTokenLen(Int)
    case wrongPackDirectory(String?)
    case wrongDataDirectory(String?)
    case wrongSecondaryLanguage(String)
    case wrongExcludedBundleIDs(Set<String>)

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
        case let .wrongExcludedBundleIDs(value):
            return "unexpected excluded bundle IDs: \(value)"
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
    exclude_bundle_ids = [
        "dev.zed.Zed",
        "com.apple.Terminal",
    ]
    """.write(to: configPath, atomically: true, encoding: .utf8)

    let config = TypeflowHostConfig.load(environment: [
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
    guard config.excludedBundleIDs == Set(["dev.zed.Zed", "com.apple.Terminal"]) else {
        throw SmokeError.wrongExcludedBundleIDs(config.excludedBundleIDs)
    }
}

do {
    let config = TypeflowEngine.defaultConfig()
    guard config.max_token_len == 128 else {
        throw SmokeError.wrongDefaultMaxTokenLen(config.max_token_len)
    }
    try verifyHostConfigPrecedence()

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
