import Foundation
import TypeClawFFI

public enum TypeClawError: Error, CustomStringConvertible {
    case engineCreationFailed
    case engineCreationFailedFromConfig(String)
    case unknownObservationTag(UInt8)
    case unknownLayout(UInt8)

    public var description: String {
        switch self {
        case .engineCreationFailed:
            return "typeclaw_engine_new_embedded returned null"
        case let .engineCreationFailedFromConfig(source):
            return "TypeClaw engine constructor returned null for \(source)"
        case let .unknownObservationTag(tag):
            return "unknown observation tag: \(tag)"
        case let .unknownLayout(layout):
            return "unknown layout: \(layout)"
        }
    }
}

public enum TypeClawLayout: UInt8, Equatable {
    case english = 0
    case secondary = 1
}

public enum TypeClawObservationAction: Equatable {
    case none
    case switchFutureLayout(TypeClawLayout)
    case resetToken
}

public struct TypeClawReplacement: Equatable {
    public let deleteCount: Int
    public let text: String
    public let inverseText: String?

    public init(deleteCount: Int, text: String, inverseText: String? = nil) {
        self.deleteCount = deleteCount
        self.text = text
        self.inverseText = inverseText
    }
}

public final class TypeClawEngine {
    private let raw: OpaquePointer
    public let sourceDescription: String

    public init() throws {
        guard let engine = typeclaw_engine_new_embedded() else {
            throw TypeClawError.engineCreationFailed
        }
        raw = engine
        sourceDescription = "embedded"
    }

    public init(hostConfig: TypeClawHostConfig) throws {
        let sourceDescription = hostConfig.engineSourceDescription
        let engine = typeclaw_engine_new_from_host_config(hostConfig.raw)

        guard let engine else {
            let error = TypeClawHostConfig.lastErrorMessage() ?? "unknown error"
            throw TypeClawError.engineCreationFailedFromConfig("\(sourceDescription): \(error)")
        }
        raw = engine
        self.sourceDescription = sourceDescription
    }

    deinit {
        typeclaw_engine_free(raw)
    }

    public var currentLayout: TypeClawLayout {
        get throws {
            let layout = typeclaw_engine_current_layout(raw)
            guard let decoded = TypeClawLayout(rawValue: layout) else {
                throw TypeClawError.unknownLayout(layout)
            }
            return decoded
        }
    }

    public var tokenLength: Int {
        Int(typeclaw_engine_token_len(raw))
    }

    public func takePendingReplacement() -> TypeClawReplacement? {
        let deleteCount = Int(typeclaw_engine_pending_replacement_delete_count(raw))
        let length = Int(typeclaw_engine_pending_replacement_utf8_len(raw))
        let inverseText = copyPendingReplacementInverseText()
        guard deleteCount > 0, length > 0 else {
            _ = typeclaw_engine_take_pending_replacement_utf8(raw, nil, 0)
            return nil
        }

        var buffer = [CChar](repeating: 0, count: length + 1)
        let required = Int(typeclaw_engine_take_pending_replacement_utf8(raw, &buffer, buffer.count))
        guard required == length else {
            return nil
        }
        return TypeClawReplacement(
            deleteCount: deleteCount,
            text: String(cString: buffer),
            inverseText: inverseText
        )
    }

    private func copyPendingReplacementInverseText() -> String? {
        let length = Int(typeclaw_engine_pending_replacement_inverse_utf8_len(raw))
        guard length > 0 else {
            return nil
        }

        var buffer = [CChar](repeating: 0, count: length + 1)
        let required = Int(
            typeclaw_engine_copy_pending_replacement_inverse_utf8(raw, &buffer, buffer.count)
        )
        guard required == length else {
            return nil
        }
        return String(cString: buffer)
    }

    public static func defaultConfig() -> TcEngineConfig {
        var config = TcEngineConfig()
        typeclaw_engine_default_config(&config)
        return config
    }

    public func resetToken() {
        typeclaw_engine_reset_token(raw)
    }

    public func resetLayout(_ layout: TypeClawLayout) {
        typeclaw_engine_reset_layout(raw, layout.rawValue)
    }

    public func setHostContext(flags: UInt32) {
        typeclaw_engine_set_host_context(raw, flags)
    }

    public func observe(physicalKey: UInt8, modifiers: UInt8 = 0) throws -> TypeClawObservationAction {
        try observe(event: typeclaw_ffi_letter_event(physicalKey, modifiers))
    }

    public func observeLiteral(_ scalar: UnicodeScalar) throws -> TypeClawObservationAction {
        try observe(event: typeclaw_ffi_literal_event(scalar.value))
    }

    public func observeBackspace() throws -> TypeClawObservationAction {
        try observe(event: typeclaw_ffi_backspace_event())
    }

    public func observeHostBypass(modifiers: UInt8) throws -> TypeClawObservationAction {
        try observe(event: typeclaw_ffi_host_bypass_event(modifiers | UInt8(TC_MOD_COMMAND)))
    }

    public func endToken() throws -> TypeClawObservationAction {
        try observe(event: typeclaw_ffi_end_token_event())
    }

    public func forceSwitchLayout() throws -> TypeClawObservationAction {
        try withFreshObservation {
            typeclaw_engine_force_switch_layout(raw, $0)
        }
    }

    private func observe(event: TcEvent) throws -> TypeClawObservationAction {
        try withFreshObservation {
            typeclaw_engine_observe(raw, event, $0)
        }
    }

    private func withFreshObservation(
        _ call: (UnsafeMutablePointer<TcObservation>) -> Void
    ) throws -> TypeClawObservationAction {
        var observation = typeclaw_ffi_empty_observation()
        call(&observation)
        return try Self.decode(observation: observation)
    }

    private static func decode(
        observation: TcObservation
    ) throws -> TypeClawObservationAction {
        switch observation.tag {
        case UInt8(TC_OBSERVATION_NONE):
            return .none
        case UInt8(TC_OBSERVATION_SWITCH_FUTURE_LAYOUT):
            return .switchFutureLayout(try decodeLayout(observation.layout))
        case UInt8(TC_OBSERVATION_RESET_TOKEN):
            return .resetToken
        default:
            throw TypeClawError.unknownObservationTag(observation.tag)
        }
    }

    private static func decodeLayout(_ value: UInt8) throws -> TypeClawLayout {
        guard let layout = TypeClawLayout(rawValue: value) else {
            throw TypeClawError.unknownLayout(value)
        }
        return layout
    }

}
