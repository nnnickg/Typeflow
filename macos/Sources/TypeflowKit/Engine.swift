import Foundation
import TypeflowFFI

public enum TypeflowError: Error, CustomStringConvertible {
    case engineCreationFailed
    case engineCreationFailedFromConfig(String)
    case unknownObservationTag(UInt8)
    case unknownLayout(UInt8)

    public var description: String {
        switch self {
        case .engineCreationFailed:
            return "typeflow_engine_new_embedded returned null"
        case let .engineCreationFailedFromConfig(source):
            return "Typeflow engine constructor returned null for \(source)"
        case let .unknownObservationTag(tag):
            return "unknown observation tag: \(tag)"
        case let .unknownLayout(layout):
            return "unknown layout: \(layout)"
        }
    }
}

public enum TypeflowLayout: UInt8, Equatable {
    case english = 0
    case secondary = 1
}

public enum TypeflowObservationAction: Equatable {
    case none
    case switchFutureLayout(TypeflowLayout)
    case resetToken
}

public struct TypeflowReplacement: Equatable {
    public let deleteCount: Int
    public let text: String

    public init(deleteCount: Int, text: String) {
        self.deleteCount = deleteCount
        self.text = text
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

    public func takePendingReplacement() -> TypeflowReplacement? {
        let deleteCount = Int(typeflow_engine_pending_replacement_delete_count(raw))
        let length = Int(typeflow_engine_pending_replacement_utf8_len(raw))
        guard deleteCount > 0, length > 0 else {
            _ = typeflow_engine_take_pending_replacement_utf8(raw, nil, 0)
            return nil
        }

        var buffer = [CChar](repeating: 0, count: length + 1)
        let required = Int(typeflow_engine_take_pending_replacement_utf8(raw, &buffer, buffer.count))
        guard required == length else {
            return nil
        }
        return TypeflowReplacement(deleteCount: deleteCount, text: String(cString: buffer))
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

    public func observe(physicalKey: UInt8, modifiers: UInt8 = 0) throws -> TypeflowObservationAction {
        try observe(event: typeflow_ffi_letter_event(physicalKey, modifiers))
    }

    public func observeLiteral(_ scalar: UnicodeScalar) throws -> TypeflowObservationAction {
        try observe(event: typeflow_ffi_literal_event(scalar.value))
    }

    public func observeBackspace() throws -> TypeflowObservationAction {
        try observe(event: typeflow_ffi_backspace_event())
    }

    public func observeHostBypass(modifiers: UInt8) throws -> TypeflowObservationAction {
        try observe(event: typeflow_ffi_host_bypass_event(modifiers | UInt8(TF_MOD_COMMAND)))
    }

    public func endToken() throws -> TypeflowObservationAction {
        try observe(event: typeflow_ffi_end_token_event())
    }

    public func forceSwitchLayout() throws -> TypeflowObservationAction {
        try withFreshObservation {
            typeflow_engine_force_switch_layout(raw, $0)
        }
    }

    private func observe(event: TfEvent) throws -> TypeflowObservationAction {
        try withFreshObservation {
            typeflow_engine_observe(raw, event, $0)
        }
    }

    private func withFreshObservation(
        _ call: (UnsafeMutablePointer<TfObservation>) -> Void
    ) throws -> TypeflowObservationAction {
        var observation = typeflow_ffi_empty_observation()
        call(&observation)
        return try Self.decode(observation: observation)
    }

    private static func decode(
        observation: TfObservation
    ) throws -> TypeflowObservationAction {
        switch observation.tag {
        case UInt8(TF_OBSERVATION_NONE):
            return .none
        case UInt8(TF_OBSERVATION_SWITCH_FUTURE_LAYOUT):
            return .switchFutureLayout(try decodeLayout(observation.layout))
        case UInt8(TF_OBSERVATION_RESET_TOKEN):
            return .resetToken
        default:
            throw TypeflowError.unknownObservationTag(observation.tag)
        }
    }

    private static func decodeLayout(_ value: UInt8) throws -> TypeflowLayout {
        guard let layout = TypeflowLayout(rawValue: value) else {
            throw TypeflowError.unknownLayout(value)
        }
        return layout
    }

}
