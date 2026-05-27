import Foundation
import TypeClawFFI

public enum TypeClawHostConfigError: Error, CustomStringConvertible {
    case loadFailed(String)

    public var description: String {
        switch self {
        case let .loadFailed(message):
            return "failed to load TypeClaw host config: \(message)"
        }
    }
}

public struct TypeClawHostSurfaceFacts {
    public var secureInput: Bool
    public var bundleID: String?
    public var applicationName: String?
    public var inputClientClass: String?
    public var focusedElementRole: String?
    public var focusedElementSubrole: String?
    public var focusedElementRoleDescription: String?
    public var focusedElementIdentifier: String?
    public var focusedElementDescription: String?
    public var focusedElementContext: String?
    public var focusedWindowTitle: String?

    public init(
        secureInput: Bool = false,
        bundleID: String? = nil,
        applicationName: String? = nil,
        inputClientClass: String? = nil,
        focusedElementRole: String? = nil,
        focusedElementSubrole: String? = nil,
        focusedElementRoleDescription: String? = nil,
        focusedElementIdentifier: String? = nil,
        focusedElementDescription: String? = nil,
        focusedElementContext: String? = nil,
        focusedWindowTitle: String? = nil
    ) {
        self.secureInput = secureInput
        self.bundleID = bundleID
        self.applicationName = applicationName
        self.inputClientClass = inputClientClass
        self.focusedElementRole = focusedElementRole
        self.focusedElementSubrole = focusedElementSubrole
        self.focusedElementRoleDescription = focusedElementRoleDescription
        self.focusedElementIdentifier = focusedElementIdentifier
        self.focusedElementDescription = focusedElementDescription
        self.focusedElementContext = focusedElementContext
        self.focusedWindowTitle = focusedWindowTitle
    }

    func withFFI<T>(_ body: (TcHostSurfaceFacts) -> T) -> T {
        withOptionalCString(bundleID) { bundleIDPointer in
            withOptionalCString(applicationName) { applicationNamePointer in
                withOptionalCString(inputClientClass) { inputClientClassPointer in
                    withOptionalCString(focusedElementRole) { focusedElementRolePointer in
                        withOptionalCString(focusedElementSubrole) { focusedElementSubrolePointer in
                            withOptionalCString(focusedElementRoleDescription) { focusedElementRoleDescriptionPointer in
                                withOptionalCString(focusedElementIdentifier) { focusedElementIdentifierPointer in
                                    withOptionalCString(focusedElementDescription) { focusedElementDescriptionPointer in
                                        withOptionalCString(focusedElementContext) { focusedElementContextPointer in
                                            withOptionalCString(focusedWindowTitle) { focusedWindowTitlePointer in
                                                body(
                                                    TcHostSurfaceFacts(
                                                        secure_input: secureInput ? 1 : 0,
                                                        bundle_id_utf8: bundleIDPointer,
                                                        application_name_utf8: applicationNamePointer,
                                                        input_client_class_utf8: inputClientClassPointer,
                                                        focused_element_role_utf8: focusedElementRolePointer,
                                                        focused_element_subrole_utf8: focusedElementSubrolePointer,
                                                        focused_element_role_description_utf8: focusedElementRoleDescriptionPointer,
                                                        focused_element_identifier_utf8: focusedElementIdentifierPointer,
                                                        focused_element_description_utf8: focusedElementDescriptionPointer,
                                                        focused_element_context_utf8: focusedElementContextPointer,
                                                        focused_window_title_utf8: focusedWindowTitlePointer
                                                    )
                                                )
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    private func withOptionalCString<T>(
        _ value: String?,
        _ body: (UnsafePointer<CChar>?) -> T
    ) -> T {
        guard let value, !value.isEmpty else {
            return body(nil)
        }
        return value.withCString(body)
    }
}

public struct TypeClawHostInputPolicy {
    public let flags: UInt32
    public let reason: UInt8

    public init(flags: UInt32, reason: UInt8) {
        self.flags = flags
        self.reason = reason
    }

    public var secureInput: Bool {
        flags & UInt32(TC_HOST_POLICY_SECURE_INPUT) != 0
    }

    public var automaticProcessingDisabled: Bool {
        flags & UInt32(TC_HOST_POLICY_AUTOMATIC_PROCESSING_DISABLED) != 0
    }

    public var manualSwitchDisabled: Bool {
        flags & UInt32(TC_HOST_POLICY_MANUAL_SWITCH_DISABLED) != 0
    }

    public var terminalSurface: Bool {
        flags & UInt32(TC_HOST_POLICY_TERMINAL_SURFACE) != 0
    }

    public var reasonDescription: String {
        switch reason {
        case UInt8(TC_HOST_POLICY_REASON_NORMAL):
            return "normal"
        case UInt8(TC_HOST_POLICY_REASON_SECURE_INPUT):
            return "secureInput"
        case UInt8(TC_HOST_POLICY_REASON_TERMINAL_BUNDLE):
            return "terminalBundle"
        case UInt8(TC_HOST_POLICY_REASON_TERMINAL_SURFACE):
            return "terminalSurface"
        case UInt8(TC_HOST_POLICY_REASON_DISABLED_BUNDLE):
            return "disabledBundle"
        case UInt8(TC_HOST_POLICY_REASON_AUTOMATIC_PROCESSING_DISABLED_BUNDLE):
            return "automaticProcessingDisabledBundle"
        case UInt8(TC_HOST_POLICY_REASON_UNAVAILABLE_HOST_CONFIG):
            return "unavailableHostConfig"
        default:
            return "unknown(\(reason))"
        }
    }
}

public final class TypeClawHostConfig {
    let raw: OpaquePointer

    public static func load() throws -> TypeClawHostConfig {
        guard let loaded = typeclaw_host_config_load() else {
            throw TypeClawHostConfigError.loadFailed(lastErrorMessage() ?? "unknown error")
        }
        return TypeClawHostConfig(raw: loaded)
    }

    public static func load(environment: [String: String]) throws -> TypeClawHostConfig {
        let configPath = environment["TYPECLAW_CONFIG"].flatMap(nonEmpty)
        let home = environment["HOME"].flatMap(nonEmpty)
        let dataDirectory = environment["TYPECLAW_DATA_DIR"].flatMap(nonEmpty)
        let packDirectory = environment["TYPECLAW_PACK_DIR"].flatMap(nonEmpty)

        let loaded = withOptionalCString(configPath) { configPathPointer in
            withOptionalCString(home) { homePointer in
                withOptionalCString(dataDirectory) { dataDirectoryPointer in
                    withOptionalCString(packDirectory) { packDirectoryPointer in
                        typeclaw_host_config_load_with_environment(
                            configPathPointer,
                            homePointer,
                            dataDirectoryPointer,
                            packDirectoryPointer
                        )
                    }
                }
            }
        }

        guard let loaded else {
            throw TypeClawHostConfigError.loadFailed(lastErrorMessage() ?? "unknown error")
        }
        return TypeClawHostConfig(raw: loaded)
    }

    static func lastErrorMessage() -> String? {
        guard let pointer = typeclaw_last_error_message() else {
            return nil
        }
        return String(cString: pointer)
    }

    private init(raw: OpaquePointer) {
        self.raw = raw
    }

    deinit {
        typeclaw_host_config_free(raw)
    }

    public var engine: TcEngineConfig {
        var config = TcEngineConfig()
        typeclaw_host_config_engine_config(raw, &config)
        return config
    }

    public var secondaryLanguage: String {
        string(from: typeclaw_host_config_secondary_language(raw)) ?? "uk"
    }

    public var packDirectory: String? {
        string(from: typeclaw_host_config_pack_directory(raw))
    }

    public var dataDirectory: String? {
        string(from: typeclaw_host_config_data_directory(raw))
    }

    public var sourcePath: String? {
        string(from: typeclaw_host_config_source_path(raw))
    }

    public var engineSourceDescription: String {
        string(from: typeclaw_host_config_engine_source(raw)) ?? "embedded"
    }

    public var macOSEnglishInputSourceID: String? {
        string(from: typeclaw_host_config_macos_english_input_source_id(raw))
    }

    public var macOSSecondaryInputSourceID: String? {
        string(from: typeclaw_host_config_macos_secondary_input_source_id(raw))
    }

    public var disabledBundleIDCount: Int {
        Int(typeclaw_host_config_disabled_bundle_count(raw))
    }

    public var autoDisabledBundleIDCount: Int {
        Int(typeclaw_host_config_auto_disabled_bundle_count(raw))
    }

    public func isBundleDisabled(bundleID: String) -> Bool {
        bundleID.withCString {
            typeclaw_host_config_is_bundle_disabled(raw, $0) != 0
        }
    }

    public func isAutomaticProcessingDisabled(bundleID: String) -> Bool {
        bundleID.withCString {
            typeclaw_host_config_is_automatic_processing_disabled(raw, $0) != 0
        }
    }

    public func resolveInputPolicy(facts: TypeClawHostSurfaceFacts) -> TypeClawHostInputPolicy {
        var policy = TcHostInputPolicy(
            flags: UInt32(TC_HOST_POLICY_AUTOMATIC_PROCESSING_DISABLED)
                | UInt32(TC_HOST_POLICY_MANUAL_SWITCH_DISABLED),
            reason: UInt8(TC_HOST_POLICY_REASON_UNAVAILABLE_HOST_CONFIG)
        )
        facts.withFFI { ffiFacts in
            typeclaw_host_config_resolve_input_policy(raw, ffiFacts, &policy)
        }
        return TypeClawHostInputPolicy(flags: policy.flags, reason: policy.reason)
    }

    private static func nonEmpty(_ value: String) -> String? {
        value.isEmpty ? nil : value
    }

    private static func withOptionalCString<T>(
        _ value: String?,
        _ body: (UnsafePointer<CChar>?) -> T
    ) -> T {
        guard let value else {
            return body(nil)
        }
        return value.withCString(body)
    }

    private func string(from pointer: UnsafePointer<CChar>?) -> String? {
        guard let pointer else {
            return nil
        }
        return String(cString: pointer)
    }
}
