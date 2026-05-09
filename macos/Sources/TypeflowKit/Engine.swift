import Foundation
import TypeflowFFI

public enum TypeflowError: Error, CustomStringConvertible {
    case engineCreationFailed
    case engineCreationFailedFromConfig(String)
    case invalidCommitCodepoint(UInt32)
    case invalidReplacementUTF8
    case unknownActionTag(UInt8)
    case unknownLayout(UInt8)

    public var description: String {
        switch self {
        case .engineCreationFailed:
            return "typeflow_engine_new_embedded returned null"
        case let .engineCreationFailedFromConfig(source):
            return "Typeflow engine constructor returned null for \(source)"
        case let .invalidCommitCodepoint(value):
            return "invalid commit codepoint: \(value)"
        case .invalidReplacementUTF8:
            return "replacement payload was not valid UTF-8"
        case let .unknownActionTag(tag):
            return "unknown action tag: \(tag)"
        case let .unknownLayout(layout):
            return "unknown layout: \(layout)"
        }
    }
}

public enum TypeflowLayout: UInt8, Equatable {
    case english = 0
    case secondary = 1
}

public enum TypeflowAction: Equatable {
    case keep
    case commit(Character)
    case replaceToken(oldLength: Int, replacement: String, layout: TypeflowLayout)
    case resetToken
}

public final class TypeflowEngine {
    private let raw: OpaquePointer
    public let sourceDescription: String

    public init() throws {
        guard let engine = typeflow_engine_new_embedded() else {
            throw TypeflowError.engineCreationFailed
        }
        raw = engine
        sourceDescription = "embedded"
    }

    public init(hostConfig: TypeflowHostConfig) throws {
        let config = hostConfig.engine
        let sourceDescription = hostConfig.engineSourceDescription
        let engine: OpaquePointer?

        if let dataDirectory = hostConfig.dataDirectory, !dataDirectory.isEmpty {
            engine = dataDirectory.withCString {
                typeflow_engine_new_from_data_dir_with_config($0, config)
            }
        } else if hostConfig.normalizedSecondaryLanguage == "uk" {
            engine = typeflow_engine_new_embedded_with_config(config)
        } else if let packPath = hostConfig.selectedPackPath {
            engine = packPath.withCString {
                typeflow_engine_new_from_pack_dir_with_config($0, config)
            }
        } else {
            engine = nil
        }

        guard let engine else {
            throw TypeflowError.engineCreationFailedFromConfig(sourceDescription)
        }
        raw = engine
        self.sourceDescription = sourceDescription
    }

    deinit {
        typeflow_engine_free(raw)
    }

    public var currentLayout: TypeflowLayout {
        get throws {
            let layout = typeflow_engine_current_layout(raw)
            guard let decoded = TypeflowLayout(rawValue: layout) else {
                throw TypeflowError.unknownLayout(layout)
            }
            return decoded
        }
    }

    public static func defaultConfig() -> TfEngineConfig {
        var config = TfEngineConfig()
        typeflow_engine_default_config(&config)
        return config
    }

    public func resetToken() {
        typeflow_engine_reset_token(raw)
    }

    public func resetLayout(_ layout: TypeflowLayout) {
        typeflow_engine_reset_layout(raw, layout.rawValue)
    }

    public func setHostContext(flags: UInt32) {
        typeflow_engine_set_host_context(raw, flags)
    }

    public func process(physicalKey: UInt8, modifiers: UInt8 = 0) throws -> TypeflowAction {
        try process(event: typeflow_ffi_letter_event(physicalKey, modifiers))
    }

    public func processLiteral(_ scalar: UnicodeScalar) throws -> TypeflowAction {
        try process(event: typeflow_ffi_literal_event(scalar.value))
    }

    public func processBackspace() throws -> TypeflowAction {
        try process(event: typeflow_ffi_backspace_event())
    }

    public func processHostBypass(modifiers: UInt8) throws -> TypeflowAction {
        try process(event: typeflow_ffi_host_bypass_event(modifiers | UInt8(TF_MOD_COMMAND)))
    }

    public func endToken() throws -> TypeflowAction {
        try process(event: typeflow_ffi_end_token_event())
    }

    public func forceSwitchToken() throws -> TypeflowAction {
        var action = typeflow_ffi_empty_action()
        typeflow_engine_force_switch_token(raw, &action)
        return try Self.decode(action: &action)
    }

    private func process(event: TfEvent) throws -> TypeflowAction {
        var action = typeflow_ffi_empty_action()
        typeflow_engine_process(raw, event, &action)
        return try Self.decode(action: &action)
    }

    private static func decode(action: inout TfAction) throws -> TypeflowAction {
        switch action.tag {
        case UInt8(TF_ACTION_KEEP):
            return .keep
        case UInt8(TF_ACTION_COMMIT):
            guard let scalar = UnicodeScalar(action.commit_codepoint) else {
                throw TypeflowError.invalidCommitCodepoint(action.commit_codepoint)
            }
            return .commit(Character(scalar))
        case UInt8(TF_ACTION_REPLACE):
            let layout = try decodeLayout(action.replace_layout)
            let replacement = try replacementString(from: &action)
            return .replaceToken(
                oldLength: Int(action.replace_old_len),
                replacement: replacement,
                layout: layout
            )
        case UInt8(TF_ACTION_RESET):
            return .resetToken
        default:
            throw TypeflowError.unknownActionTag(action.tag)
        }
    }

    private static func decodeLayout(_ value: UInt8) throws -> TypeflowLayout {
        guard let layout = TypeflowLayout(rawValue: value) else {
            throw TypeflowError.unknownLayout(value)
        }
        return layout
    }

    private static func replacementString(from action: inout TfAction) throws -> String {
        let length = Int(action.replace_text_len)
        let bytes = withUnsafeBytes(of: &action.replace_text) { rawBuffer in
            Array(rawBuffer.prefix(length))
        }
        guard let string = String(bytes: bytes, encoding: .utf8) else {
            throw TypeflowError.invalidReplacementUTF8
        }
        return string
    }
}
