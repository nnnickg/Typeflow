import AppKit
import ApplicationServices
import Carbon
import InputMethodKit
import os
#if SWIFT_PACKAGE
import TypeflowKit
#endif
import TypeflowFFI

private let logger = Logger(
    subsystem: "io.github.nnnickg.typeflow.inputmethod.Typeflow",
    category: "InputController"
)
private let performanceLogger = Logger(
    subsystem: "io.github.nnnickg.typeflow.inputmethod.Typeflow",
    category: "Performance"
)
private let performanceLogAll = ProcessInfo.processInfo.environment["TYPEFLOW_PERF_LOG_ALL"] == "1"
private let slowProcessThresholdMs = 2.0
private let slowHostThresholdMs = 0.75
private let slowCallThresholdMs = 0.25
private let hostContextCacheTTLSeconds: TimeInterval = 0.25
private let accessibilityTrustCacheTTLSeconds: TimeInterval = 1.0

@inline(__always)
private func measured<T>(
    _ name: String,
    thresholdMs: Double,
    _ body: () throws -> T
) rethrows -> T {
    let started = ProcessInfo.processInfo.systemUptime
    defer {
        logPerformance(name: name, started: started, thresholdMs: thresholdMs)
    }
    return try body()
}

private func logPerformance(name: String, started: TimeInterval, thresholdMs: Double) {
    let elapsedMs = (ProcessInfo.processInfo.systemUptime - started) * 1000.0
    guard performanceLogAll || elapsedMs >= thresholdMs else {
        return
    }
    performanceLogger.notice(
        "perf name=\(name, privacy: .public) durationMs=\(elapsedMs, privacy: .public) thresholdMs=\(thresholdMs, privacy: .public)"
    )
}

@objc(TypeflowInputController)
final class TypeflowInputController: IMKInputController {
    private static var accessibilityTrustPromptRequested = false

    private let hostConfig: TypeflowHostConfig?
    private let engine: TypeflowEngine?
    private var hostPolicyLogKey = ""
    private var hostPolicyCacheKey = ""
    private var cachedHostPolicy: TypeflowHostInputPolicy?
    private var hostContextCacheClientID: ObjectIdentifier?
    private var hostContextCacheSecureInput = false
    private var hostContextCacheExpiresAt: TimeInterval = 0
    private var cachedHostContextSnapshot: HostContextSnapshot?
    private var accessibilityCacheKey = ""
    private var accessibilityCacheExpiresAt: TimeInterval = 0
    private var accessibilityCache = AccessibilitySnapshot.empty
    private var accessibilityTrustCacheExpiresAt: TimeInterval = 0
    private var accessibilityTrustCache = false
    private var accessibilityTrustLogged = false
    private var pendingOptionManualConvert = false
    private var manualConvertCancelled = false
    private var trackedClientID: ObjectIdentifier?
    private var activeComposition = ""

    override init!(server: IMKServer!, delegate: Any!, client inputClient: Any!) {
        var loadedConfig: TypeflowHostConfig?
        var loadedEngine: TypeflowEngine?
        var engineSource: String?
        var engineError: String?
        do {
            let config = try TypeflowHostConfig.load()
            loadedConfig = config
            let initializedEngine = try TypeflowEngine(hostConfig: config)
            loadedEngine = initializedEngine
            engineSource = initializedEngine.sourceDescription
            engineError = nil
        } catch {
            engineSource = nil
            engineError = String(describing: error)
        }
        hostConfig = loadedConfig
        engine = loadedEngine
        super.init(server: server, delegate: delegate, client: inputClient)
        if let engineError {
            logger.error("Typeflow disabled: \(engineError, privacy: .public)")
        } else if let loadedConfig {
            logger.notice(
                "initialized input controller source=\(engineSource ?? "unknown", privacy: .public) config=\(loadedConfig.sourcePath == nil ? "defaults" : "loaded", privacy: .public) disabledBundles=\(loadedConfig.disabledBundleIDCount, privacy: .public) autoDisabledBundles=\(loadedConfig.autoDisabledBundleIDCount, privacy: .public) manualConvert=option"
            )
        }
    }

    override func activateServer(_ sender: Any!) {
        logger.debug("activated input controller")
        promptForAccessibilityTrustIfNeeded()
        resetTrackedHostState()
        engine?.resetToken()
    }

    override func deactivateServer(_ sender: Any!) {
        hostPolicyLogKey = ""
        resetPendingManualConvert()
        resetTrackedHostState()
        engine?.resetToken()
    }

    override func commitComposition(_ sender: Any!) {
        if let client = sender as? IMKTextInput,
           let action = try? engine?.endToken()
        {
            _ = applyComposition(action, client: client, reason: "commitComposition")
        }
        resetPendingManualConvert()
        resetTrackedHostState()
        engine?.resetToken()
    }

    override func composedString(_ sender: Any!) -> Any! {
        activeComposition.isEmpty ? nil : activeComposition
    }

    override func originalString(_ sender: Any!) -> NSAttributedString! {
        NSAttributedString(string: "")
    }

    override func selectionRange() -> NSRange {
        NSRange(location: activeComposition.utf16.count, length: 0)
    }

    override func replacementRange() -> NSRange {
        NSRange(location: NSNotFound, length: 0)
    }

    override func recognizedEvents(_ sender: Any!) -> Int {
        Int((NSEvent.EventTypeMask.keyDown.rawValue | NSEvent.EventTypeMask.flagsChanged.rawValue))
    }

    override func handle(_ event: NSEvent!, client sender: Any!) -> Bool {
        guard let event else {
            return false
        }
        switch event.type {
        case .keyDown:
            return processKey(
                keyCode: event.keyCode,
                characters: event.characters,
                modifierFlags: event.modifierFlags,
                client: sender,
                origin: "handle"
            )
        case .flagsChanged:
            return processFlagsChanged(event, client: sender)
        default:
            return false
        }
    }

    private func processKey(
        keyCode: UInt16,
        characters: String?,
        modifierFlags: NSEvent.ModifierFlags,
        client sender: Any!,
        origin: String
    ) -> Bool {
        let processStarted = ProcessInfo.processInfo.systemUptime
        defer {
            logPerformance(name: "processKey", started: processStarted, thresholdMs: slowProcessThresholdMs)
        }

        guard let engine else {
            return false
        }
        cancelPendingManualConvert()
        logger.debug(
            "keyDown origin=\(origin, privacy: .public) keyCode=\(keyCode, privacy: .private) hasText=\(characters?.isEmpty == false, privacy: .private) client=\(String(describing: type(of: sender)), privacy: .private)"
        )

        let modifiers = ffiModifiers(from: modifierFlags, keyCode: keyCode)
        guard let client = sender as? IMKTextInput else {
            _ = try? measured("ffi.processHostBypass", thresholdMs: slowCallThresholdMs) {
                try engine.processHostBypass(modifiers: modifiers)
            }
            activeComposition = ""
            return false
        }
        syncClient(client)

        if shouldBypassHost(modifierFlags) {
            let action = (try? measured("ffi.processHostBypass", thresholdMs: slowCallThresholdMs) {
                try engine.processHostBypass(modifiers: modifiers)
            }) ?? .bypass
            return applyComposition(action, client: client, reason: "hostBypass")
        }

        let hostContext = measured("updateHostContext", thresholdMs: slowHostThresholdMs) {
            updateHostContext(client: sender)
        }
        if hostContext.keyProcessingDisabled {
            clearComposition(client: client, reason: "disabled")
            engine.resetToken()
            return false
        }

        do {
            switch Int(keyCode) {
            case kVK_Return, kVK_Tab, kVK_Escape:
                let action = try measured("ffi.endToken", thresholdMs: slowCallThresholdMs) {
                    try engine.endToken()
                }
                return applyComposition(action, client: client, reason: "endToken")
            case kVK_Delete:
                let action = try measured("ffi.processBackspace", thresholdMs: slowCallThresholdMs) {
                    try engine.processBackspace()
                }
                return applyComposition(action, client: client, reason: "backspace")
            default:
                break
            }

            if shouldBypassNonTextKey(keyCode: keyCode, characters: characters) {
                let action = try measured("ffi.processHostBypass", thresholdMs: slowCallThresholdMs) {
                    try engine.processHostBypass(modifiers: modifiers)
                }
                return applyComposition(action, client: client, reason: "nonText")
            }

            if let physical = TypeflowMacKeyCode.physicalKeyIndex(for: keyCode) {
                let action = try measured("ffi.process", thresholdMs: slowCallThresholdMs) {
                    try engine.process(physicalKey: physical, modifiers: modifiers)
                }
                return applyComposition(action, client: client, reason: "key")
            }

            guard let scalar = singleScalar(from: characters) else {
                let action = try measured("ffi.endToken", thresholdMs: slowCallThresholdMs) {
                    try engine.endToken()
                }
                return applyComposition(action, client: client, reason: "literalWithoutScalar")
            }
            let action = try measured("ffi.processLiteral", thresholdMs: slowCallThresholdMs) {
                try engine.processLiteral(scalar)
            }
            return applyComposition(action, client: client, reason: "literal")
        } catch {
            engine.resetToken()
            clearComposition(client: client, reason: "error")
            return false
        }
    }

    private func processFlagsChanged(_ event: NSEvent, client sender: Any!) -> Bool {
        let processStarted = ProcessInfo.processInfo.systemUptime
        defer {
            logPerformance(
                name: "processFlagsChanged",
                started: processStarted,
                thresholdMs: slowProcessThresholdMs
            )
        }

        let isOptionKey = Int(event.keyCode) == kVK_Option || Int(event.keyCode) == kVK_RightOption
        guard isOptionKey else {
            cancelPendingManualConvert()
            return false
        }
        guard let engine else {
            resetPendingManualConvert()
            return false
        }

        let optionDown = event.modifierFlags.contains(.option)
        logger.debug(
            "manualConvert optionEvent keyCode=\(event.keyCode, privacy: .private) down=\(optionDown, privacy: .private) client=\(String(describing: type(of: sender)), privacy: .private)"
        )

        let hostContext = measured("updateHostContext", thresholdMs: slowHostThresholdMs) {
            updateHostContext(client: sender)
        }
        if hostContext.manualConversionDisabled {
            resetPendingManualConvert()
            return false
        }

        if optionDown {
            pendingOptionManualConvert = true
            manualConvertCancelled = false
            return true
        }

        guard pendingOptionManualConvert else {
            return false
        }

        let shouldConvert = !manualConvertCancelled
        resetPendingManualConvert()
        guard shouldConvert, let client = sender as? IMKTextInput else {
            return false
        }
        syncClient(client)

        do {
            let action = try measured("ffi.forceSwitchToken", thresholdMs: slowCallThresholdMs) {
                try engine.forceSwitchToken()
            }
            let handled = applyComposition(action, client: client, reason: "manual")
            logger.debug("manualConvert action=forceSwitch handled=\(handled, privacy: .private)")
            return handled
        } catch {
            engine.resetToken()
            clearComposition(client: client, reason: "manualError")
            return false
        }
    }

    @discardableResult
    private func applyComposition(
        _ action: TypeflowCompositionAction,
        client: IMKTextInput,
        reason: String
    ) -> Bool {
        switch action {
        case .bypass:
            return false
        case let .render(text, _):
            activeComposition = text
            measured("setMarkedText.\(reason)", thresholdMs: slowHostThresholdMs) {
                client.setMarkedText(
                    text,
                    selectionRange: NSRange(location: text.utf16.count, length: 0),
                    replacementRange: replacementRange()
                )
            }
            return true
        case let .commit(text, consumeEvent):
            activeComposition = ""
            if !text.isEmpty {
                measured("insertText.commit.\(reason)", thresholdMs: slowHostThresholdMs) {
                    client.insertText(text, replacementRange: replacementRange())
                }
            } else {
                clearComposition(client: client, reason: reason)
            }
            return consumeEvent
        case let .clear(consumeEvent):
            clearComposition(client: client, reason: reason)
            return consumeEvent
        }
    }

    private func clearComposition(client: IMKTextInput, reason: String) {
        guard !activeComposition.isEmpty else {
            return
        }
        activeComposition = ""
        measured("setMarkedText.clear.\(reason)", thresholdMs: slowHostThresholdMs) {
            client.setMarkedText(
                "",
                selectionRange: NSRange(location: 0, length: 0),
                replacementRange: replacementRange()
            )
        }
    }

    private func syncClient(_ client: IMKTextInput) {
        let clientID = ObjectIdentifier(client as AnyObject)
        guard trackedClientID != clientID else {
            return
        }
        trackedClientID = clientID
        activeComposition = ""
        engine?.resetLayout(.english)
    }

    private struct HostContextSnapshot {
        let keyProcessingDisabled: Bool
        let manualConversionDisabled: Bool
    }

    private func updateHostContext(client sender: Any!) -> HostContextSnapshot {
        let now = ProcessInfo.processInfo.systemUptime
        let clientID = sender.map { ObjectIdentifier($0 as AnyObject) }
        let secureInput = measured("secureInput", thresholdMs: slowCallThresholdMs) {
            IsSecureEventInputEnabled()
        }
        if let cachedHostContextSnapshot,
           let clientID,
           clientID == hostContextCacheClientID,
           secureInput == hostContextCacheSecureInput,
           now < hostContextCacheExpiresAt
        {
            if cachedHostContextSnapshot.manualConversionDisabled {
                activeComposition = ""
            }
            return cachedHostContextSnapshot
        }

        let facts = measured("hostSurfaceFacts", thresholdMs: slowHostThresholdMs) {
            hostSurfaceFacts(client: sender, secureInput: secureInput)
        }
        let policyKey = hostPolicyKey(for: facts)
        let policy: TypeflowHostInputPolicy
        if policyKey == hostPolicyCacheKey, let cachedHostPolicy {
            policy = cachedHostPolicy
        } else {
            policy = measured("hostPolicy.resolve", thresholdMs: slowCallThresholdMs) {
                hostConfig?.resolveInputPolicy(facts: facts) ?? TypeflowHostInputPolicy(
                    flags: UInt32(TF_HOST_POLICY_AUTOMATIC_PROCESSING_DISABLED)
                        | UInt32(TF_HOST_POLICY_MANUAL_CONVERSION_DISABLED),
                    reason: UInt8(TF_HOST_POLICY_REASON_UNAVAILABLE_HOST_CONFIG)
                )
            }
            hostPolicyCacheKey = policyKey
            cachedHostPolicy = policy
        }
        var flags: UInt32 = 0

        if policy.secureInput {
            flags |= typeflow_ffi_context_secure_input()
        }
        if policy.manualConversionDisabled {
            flags |= typeflow_ffi_context_automatic_processing_disabled()
        } else if policy.automaticProcessingDisabled {
            flags |= typeflow_ffi_context_automatic_switching_disabled()
        }

        measured("ffi.setHostContext", thresholdMs: slowCallThresholdMs) {
            engine?.setHostContext(flags: flags)
        }
        if policy.manualConversionDisabled {
            activeComposition = ""
        }

        let logKey = [
            String(flags),
            String(policy.flags),
            String(policy.reason),
            facts.bundleID ?? "",
            facts.inputClientClass ?? "",
            facts.focusedElementIdentifier ?? "",
            facts.focusedElementRole ?? "",
        ].joined(separator: "|")

        if logKey != hostPolicyLogKey {
            logger.debug(
                "hostPolicy reason=\(policy.reasonDescription, privacy: .public) secure=\(policy.secureInput, privacy: .private) autoDisabled=\(policy.automaticProcessingDisabled, privacy: .private) manualDisabled=\(policy.manualConversionDisabled, privacy: .private) terminal=\(policy.terminalSurface, privacy: .private) bundleID=\(facts.bundleID ?? "unknown", privacy: .private) client=\(facts.inputClientClass ?? "unknown", privacy: .private) axRole=\(facts.focusedElementRole ?? "unknown", privacy: .private) axSubrole=\(facts.focusedElementSubrole ?? "unknown", privacy: .private) axID=\(facts.focusedElementIdentifier ?? "unknown", privacy: .private)"
            )
            hostPolicyLogKey = logKey
        }

        let snapshot = HostContextSnapshot(
            keyProcessingDisabled: policy.manualConversionDisabled,
            manualConversionDisabled: policy.manualConversionDisabled
        )
        if let clientID {
            hostContextCacheClientID = clientID
            hostContextCacheSecureInput = secureInput
            hostContextCacheExpiresAt = now + hostContextCacheTTLSeconds
            cachedHostContextSnapshot = snapshot
        }
        return snapshot
    }

    private func hostPolicyKey(for facts: TypeflowHostSurfaceFacts) -> String {
        [
            facts.secureInput ? "1" : "0",
            facts.bundleID ?? "",
            facts.inputClientClass ?? "",
            facts.focusedElementRole ?? "",
            facts.focusedElementSubrole ?? "",
            facts.focusedElementRoleDescription ?? "",
            facts.focusedElementIdentifier ?? "",
            facts.focusedElementDescription ?? "",
        ].joined(separator: "|")
    }

    private func hostSurfaceFacts(client sender: Any!, secureInput: Bool) -> TypeflowHostSurfaceFacts {
        let app = measured("frontmostApplication", thresholdMs: slowCallThresholdMs) {
            NSWorkspace.shared.frontmostApplication
        }
        let clientClass = sender.map { String(describing: type(of: $0)) }
        let ax = measured("accessibilitySnapshot.cached", thresholdMs: slowHostThresholdMs) {
            cachedAccessibilitySnapshot(for: app, clientClass: clientClass)
        }
        return TypeflowHostSurfaceFacts(
            secureInput: secureInput,
            bundleID: app?.bundleIdentifier,
            applicationName: app?.localizedName,
            inputClientClass: clientClass,
            focusedElementRole: ax.focusedElementRole,
            focusedElementSubrole: ax.focusedElementSubrole,
            focusedElementRoleDescription: ax.focusedElementRoleDescription,
            focusedElementIdentifier: ax.focusedElementIdentifier,
            focusedElementDescription: ax.focusedElementDescription,
            focusedWindowTitle: ax.focusedWindowTitle
        )
    }

    private func cachedAccessibilitySnapshot(
        for app: NSRunningApplication?,
        clientClass: String?
    ) -> AccessibilitySnapshot {
        let now = ProcessInfo.processInfo.systemUptime
        let trusted = cachedAccessibilityTrust(now: now)
        guard trusted else {
            if !accessibilityTrustLogged {
                logger.debug("accessibility not trusted; embedded terminal surface detection unavailable")
                accessibilityTrustLogged = true
            }
            return .empty
        }

        let pid = app?.processIdentifier ?? 0
        let key = "\(pid)|\(clientClass ?? "")"
        if key == accessibilityCacheKey, now < accessibilityCacheExpiresAt {
            return accessibilityCache
        }

        let snapshot = measured("accessibilitySnapshot.refresh", thresholdMs: slowHostThresholdMs) {
            accessibilitySnapshot(for: app)
        }
        accessibilityCacheKey = key
        accessibilityCacheExpiresAt = now + 0.10
        accessibilityCache = snapshot
        return snapshot
    }

    private func cachedAccessibilityTrust(now: TimeInterval) -> Bool {
        if now < accessibilityTrustCacheExpiresAt {
            return accessibilityTrustCache
        }

        let trusted = measured("AXIsProcessTrusted", thresholdMs: slowCallThresholdMs) {
            AXIsProcessTrusted()
        }
        accessibilityTrustCache = trusted
        accessibilityTrustCacheExpiresAt = now + accessibilityTrustCacheTTLSeconds
        return trusted
    }

    private func promptForAccessibilityTrustIfNeeded() {
        guard !Self.accessibilityTrustPromptRequested else {
            return
        }
        Self.accessibilityTrustPromptRequested = true

        guard !AXIsProcessTrusted() else {
            return
        }

        let options = [
            kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true
        ] as NSDictionary
        let trusted = AXIsProcessTrustedWithOptions(options)
        logger.notice(
            "accessibility trust prompt requested trusted=\(trusted, privacy: .public)"
        )
    }

    private struct AccessibilitySnapshot {
        let focusedElementRole: String?
        let focusedElementSubrole: String?
        let focusedElementRoleDescription: String?
        let focusedElementIdentifier: String?
        let focusedElementDescription: String?
        let focusedWindowTitle: String?

        static let empty = AccessibilitySnapshot(
            focusedElementRole: nil,
            focusedElementSubrole: nil,
            focusedElementRoleDescription: nil,
            focusedElementIdentifier: nil,
            focusedElementDescription: nil,
            focusedWindowTitle: nil
        )
    }

    private func accessibilitySnapshot(for app: NSRunningApplication?) -> AccessibilitySnapshot {
        guard let app else {
            return .empty
        }

        let appElement = AXUIElementCreateApplication(app.processIdentifier)
        _ = AXUIElementSetMessagingTimeout(appElement, 0.01)
        let focusedElement = measured("AX.focusedElement", thresholdMs: slowCallThresholdMs) {
            copyAXElement(appElement, attribute: kAXFocusedUIElementAttribute as CFString)
        }
        let focusedWindow = measured("AX.focusedWindow", thresholdMs: slowCallThresholdMs) {
            copyAXElement(appElement, attribute: kAXFocusedWindowAttribute as CFString)
        }
        if let focusedElement {
            _ = AXUIElementSetMessagingTimeout(focusedElement, 0.01)
        }
        if let focusedWindow {
            _ = AXUIElementSetMessagingTimeout(focusedWindow, 0.01)
        }

        return AccessibilitySnapshot(
            focusedElementRole: focusedElement.flatMap { element in
                measured("AX.elementRole", thresholdMs: slowCallThresholdMs) {
                    copyAXString(element, attribute: kAXRoleAttribute as CFString)
                }
            },
            focusedElementSubrole: focusedElement.flatMap { element in
                measured("AX.elementSubrole", thresholdMs: slowCallThresholdMs) {
                    copyAXString(element, attribute: kAXSubroleAttribute as CFString)
                }
            },
            focusedElementRoleDescription: focusedElement.flatMap { element in
                measured("AX.elementRoleDescription", thresholdMs: slowCallThresholdMs) {
                    copyAXString(element, attribute: kAXRoleDescriptionAttribute as CFString)
                }
            },
            focusedElementIdentifier: focusedElement.flatMap { element in
                measured("AX.elementIdentifier", thresholdMs: slowCallThresholdMs) {
                    copyAXString(element, attribute: kAXIdentifierAttribute as CFString)
                }
            },
            focusedElementDescription: focusedElement.flatMap { element in
                measured("AX.elementDescription", thresholdMs: slowCallThresholdMs) {
                    copyAXString(element, attribute: kAXDescriptionAttribute as CFString)
                }
            },
            focusedWindowTitle: focusedWindow.flatMap { window in
                measured("AX.windowTitle", thresholdMs: slowCallThresholdMs) {
                    copyAXString(window, attribute: kAXTitleAttribute as CFString)
                }
            }
        )
    }

    private func copyAXElement(_ element: AXUIElement, attribute: CFString) -> AXUIElement? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, attribute, &value) == .success,
              let value,
              CFGetTypeID(value) == AXUIElementGetTypeID()
        else {
            return nil
        }
        return (value as! AXUIElement)
    }

    private func copyAXString(_ element: AXUIElement, attribute: CFString) -> String? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, attribute, &value) == .success,
              let value
        else {
            return nil
        }
        return value as? String
    }

    private func cancelPendingManualConvert() {
        if pendingOptionManualConvert {
            manualConvertCancelled = true
        }
    }

    private func resetPendingManualConvert() {
        pendingOptionManualConvert = false
        manualConvertCancelled = false
    }

    private func resetTrackedHostState() {
        trackedClientID = nil
        activeComposition = ""
        hostContextCacheClientID = nil
        hostContextCacheSecureInput = false
        hostContextCacheExpiresAt = 0
        cachedHostContextSnapshot = nil
        hostPolicyCacheKey = ""
        cachedHostPolicy = nil
        accessibilityCacheKey = ""
        accessibilityCacheExpiresAt = 0
        accessibilityCache = .empty
        accessibilityTrustCacheExpiresAt = 0
        accessibilityTrustCache = false
    }

    private func shouldBypassHost(_ flags: NSEvent.ModifierFlags) -> Bool {
        let bypass: NSEvent.ModifierFlags = [.control, .option, .command]
        return !flags.intersection(bypass).isEmpty
    }

    private func shouldBypassNonTextKey(keyCode: UInt16, characters: String?) -> Bool {
        switch Int(keyCode) {
        case kVK_UpArrow,
             kVK_DownArrow,
             kVK_LeftArrow,
             kVK_RightArrow,
             kVK_Home,
             kVK_End,
             kVK_PageUp,
             kVK_PageDown,
             kVK_ForwardDelete,
             kVK_Help:
            return true
        default:
            break
        }

        guard let scalar = singleScalar(from: characters) else {
            return false
        }
        return isFunctionKeyScalar(scalar) || isControlScalar(scalar)
    }

    private func isFunctionKeyScalar(_ scalar: UnicodeScalar) -> Bool {
        (0xF700...0xF8FF).contains(Int(scalar.value))
    }

    private func isControlScalar(_ scalar: UnicodeScalar) -> Bool {
        scalar.value < 0x20 || scalar.value == 0x7F
    }

    private func ffiModifiers(from flags: NSEvent.ModifierFlags, keyCode: UInt16? = nil) -> UInt8 {
        var modifiers: UInt8 = 0
        let shiftDown = flags.contains(.shift)
        let capsLockAffectsKey = keyCode.map { flags.contains(.capsLock) && isAnsiLetterKey($0) } ?? false
        if shiftDown != capsLockAffectsKey {
            modifiers |= UInt8(TF_MOD_SHIFT)
        }
        if flags.contains(.control) {
            modifiers |= UInt8(TF_MOD_CONTROL)
        }
        if flags.contains(.option) {
            modifiers |= UInt8(TF_MOD_OPTION)
        }
        if flags.contains(.command) {
            modifiers |= UInt8(TF_MOD_COMMAND)
        }
        return modifiers
    }

    private func isAnsiLetterKey(_ keyCode: UInt16) -> Bool {
        switch Int(keyCode) {
        case kVK_ANSI_A,
             kVK_ANSI_B,
             kVK_ANSI_C,
             kVK_ANSI_D,
             kVK_ANSI_E,
             kVK_ANSI_F,
             kVK_ANSI_G,
             kVK_ANSI_H,
             kVK_ANSI_I,
             kVK_ANSI_J,
             kVK_ANSI_K,
             kVK_ANSI_L,
             kVK_ANSI_M,
             kVK_ANSI_N,
             kVK_ANSI_O,
             kVK_ANSI_P,
             kVK_ANSI_Q,
             kVK_ANSI_R,
             kVK_ANSI_S,
             kVK_ANSI_T,
             kVK_ANSI_U,
             kVK_ANSI_V,
             kVK_ANSI_W,
             kVK_ANSI_X,
             kVK_ANSI_Y,
             kVK_ANSI_Z:
            return true
        default:
            return false
        }
    }

    private func singleScalar(from text: String?) -> UnicodeScalar? {
        guard let text else {
            return nil
        }
        var iterator = text.unicodeScalars.makeIterator()
        guard let scalar = iterator.next(), iterator.next() == nil else {
            return nil
        }
        return scalar
    }
}
