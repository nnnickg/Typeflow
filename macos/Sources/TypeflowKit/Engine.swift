import Foundation
import TypeflowFFI

public enum TypeflowError: Error, CustomStringConvertible {
    case engineCreationFailed
    case engineCreationFailedFromConfig(String)
    case invalidCompositionUTF8
    case unknownCompositionTag(UInt8)
    case unknownLayout(UInt8)

    public var description: String {
        switch self {
        case .engineCreationFailed:
            return "typeflow_engine_new_embedded returned null"
        case let .engineCreationFailedFromConfig(source):
            return "Typeflow engine constructor returned null for \(source)"
        case .invalidCompositionUTF8:
            return "composition payload was not valid UTF-8"
        case let .unknownCompositionTag(tag):
            return "unknown composition tag: \(tag)"
        case let .unknownLayout(layout):
            return "unknown layout: \(layout)"
        }
    }
}

public enum TypeflowLayout: UInt8, Equatable {
    case english = 0
    case secondary = 1
}

public enum TypeflowCompositionAction: Equatable {
    case bypass
    case render(text: String, layout: TypeflowLayout)
    case commit(text: String, consumeEvent: Bool)
    case clear(consumeEvent: Bool)

    public var consumesEvent: Bool {
        switch self {
        case .bypass:
            return false
        case .render:
            return true
        case let .commit(_, consumeEvent), let .clear(consumeEvent):
            return consumeEvent
        }
    }
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
        let sourceDescription = hostConfig.engineSourceDescription
        let engine = typeflow_engine_new_from_host_config(hostConfig.raw)

        guard let engine else {
            let error = TypeflowHostConfig.lastErrorMessage() ?? "unknown error"
            throw TypeflowError.engineCreationFailedFromConfig("\(sourceDescription): \(error)")
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

    public var tokenLength: Int {
        Int(typeflow_engine_token_len(raw))
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

    public func process(physicalKey: UInt8, modifiers: UInt8 = 0) throws -> TypeflowCompositionAction {
        try process(event: typeflow_ffi_letter_event(physicalKey, modifiers))
    }

    public func processLiteral(_ scalar: UnicodeScalar) throws -> TypeflowCompositionAction {
        try process(event: typeflow_ffi_literal_event(scalar.value))
    }

    public func processBackspace() throws -> TypeflowCompositionAction {
        try process(event: typeflow_ffi_backspace_event())
    }

    public func processHostBypass(modifiers: UInt8) throws -> TypeflowCompositionAction {
        try process(event: typeflow_ffi_host_bypass_event(modifiers | UInt8(TF_MOD_COMMAND)))
    }

    public func endToken() throws -> TypeflowCompositionAction {
        try process(event: typeflow_ffi_end_token_event())
    }

    public func forceSwitchToken() throws -> TypeflowCompositionAction {
        try withFreshComposition {
            typeflow_engine_force_switch_token(raw, $0)
        }
    }

    private func process(event: TfEvent) throws -> TypeflowCompositionAction {
        try withFreshComposition {
            typeflow_engine_process(raw, event, $0)
        }
    }

    private func withFreshComposition(
        _ call: (UnsafeMutablePointer<TfComposition>) -> Void
    ) throws -> TypeflowCompositionAction {
        var composition = typeflow_ffi_empty_composition()
        call(&composition)
        return try Self.decode(composition: &composition)
    }

    private static func decode(
        composition: inout TfComposition
    ) throws -> TypeflowCompositionAction {
        switch composition.tag {
        case UInt8(TF_COMPOSITION_BYPASS):
            return .bypass
        case UInt8(TF_COMPOSITION_RENDER):
            let layout = try decodeLayout(composition.layout)
            return .render(
                text: try compositionString(from: &composition),
                layout: layout
            )
        case UInt8(TF_COMPOSITION_COMMIT):
            return .commit(
                text: try compositionString(from: &composition),
                consumeEvent: composition.consume_event != 0
            )
        case UInt8(TF_COMPOSITION_CLEAR):
            return .clear(consumeEvent: composition.consume_event != 0)
        default:
            throw TypeflowError.unknownCompositionTag(composition.tag)
        }
    }

    private static func decodeLayout(_ value: UInt8) throws -> TypeflowLayout {
        guard let layout = TypeflowLayout(rawValue: value) else {
            throw TypeflowError.unknownLayout(value)
        }
        return layout
    }

    private static func compositionString(from composition: inout TfComposition) throws -> String {
        let length = Int(composition.text_len)
        let bytes = try withUnsafeBytes(of: &composition.text) { rawBuffer in
            guard length <= rawBuffer.count else {
                throw TypeflowError.invalidCompositionUTF8
            }
            return Array(rawBuffer.prefix(length))
        }
        guard let string = String(bytes: bytes, encoding: .utf8) else {
            throw TypeflowError.invalidCompositionUTF8
        }
        return string
    }
}
