import AppKit
import ApplicationServices
import Carbon
import InputMethodKit
import os
import TypeflowFFI

private let logger = Logger(
    subsystem: "io.github.nnnickg.typeflow.inputmethod.Typeflow",
    category: "InputController"
)
private let maxVisibleTailUTF16Length = 1024

@objc(TypeflowInputController)
final class TypeflowInputController: IMKInputController {
    private let hostConfig: TypeflowHostConfig?
    private let engine: TypeflowEngine?
    private var hostPolicyLogKey = ""
    private var hostPolicyCacheKey = ""
    private var cachedHostPolicy: TypeflowHostInputPolicy?
    private var accessibilityCacheKey = ""
    private var accessibilityCacheExpiresAt: TimeInterval = 0
    private var accessibilityCache = AccessibilitySnapshot.empty
    private var accessibilityTrustLogged = false
    private var pendingOptionManualConvert = false
    private var manualConvertCancelled = false
    private var trackedClientID: ObjectIdentifier?
    private var expectedSelectedLocation: Int?

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
        resetPendingManualConvert()
        resetTrackedHostState()
        engine?.resetToken()
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
        guard let engine else {
            return false
        }
        cancelPendingManualConvert()
        if updateHostContext(client: sender).keyProcessingDisabled {
            return false
        }
        logger.debug(
            "keyDown origin=\(origin, privacy: .public) keyCode=\(keyCode, privacy: .private) hasText=\(characters?.isEmpty == false, privacy: .private) client=\(String(describing: type(of: sender)), privacy: .private)"
        )
        guard let client = sender as? IMKTextInput else {
            _ = try? engine.processHostBypass(modifiers: ffiModifiers(from: modifierFlags))
            return false
        }

        let modifiers = ffiModifiers(from: modifierFlags, keyCode: keyCode)
        if shouldBypassHost(modifierFlags) {
            _ = try? engine.processHostBypass(modifiers: modifiers)
            return false
        }

        switch Int(keyCode) {
        case kVK_Return, kVK_Tab, kVK_Escape:
            _ = try? engine.endToken()
            expectedSelectedLocation = nil
            return false
        default:
            break
        }

        if Int(keyCode) == kVK_Delete {
            if shouldBypassForHostSelection(client) {
                _ = try? engine.processHostBypass(modifiers: modifiers)
                return false
            }
            _ = try? engine.processBackspace()
            recordExpectedSelectionAfterHostBackspace(client)
            return false
        }

        if shouldBypassNonTextKey(keyCode: keyCode, characters: characters) {
            engine.resetLayout(.english)
            expectedSelectedLocation = nil
            return false
        }
        if shouldBypassForHostSelection(client) {
            _ = try? engine.processHostBypass(modifiers: modifiers)
            return false
        }

        do {
            if let physical = TypeflowMacKeyCode.physicalKeyIndex(for: keyCode) {
                let action = try engine.process(physicalKey: physical, modifiers: modifiers)
                let resolvedAction = try resolveReplacementFromVisibleTail(
                    action,
                    physicalKey: physical,
                    modifiers: modifiers,
                    client: client
                )
                return apply(resolvedAction, client: client)
            }

            guard let scalar = singleScalar(from: characters) else {
                _ = try? engine.endToken()
                return false
            }
            let action = try engine.processLiteral(scalar)
            return apply(action, client: client)
        } catch {
            engine.resetToken()
            return false
        }
    }

    private func resolveReplacementFromVisibleTail(
        _ action: TypeflowAction,
        physicalKey: UInt8,
        modifiers: UInt8,
        client: IMKTextInput
    ) throws -> TypeflowAction {
        guard case let .replaceToken(_, _, layout) = action,
              let tail = visibleTail(in: client)
        else {
            return action
        }

        let resolved = try engine?.replaceVisibleTail(
            tail,
            physicalKey: physicalKey,
            modifiers: modifiers,
            targetLayout: layout
        )
        return resolved ?? action
    }

    private func processFlagsChanged(_ event: NSEvent, client sender: Any!) -> Bool {
        let isOptionKey = Int(event.keyCode) == kVK_Option || Int(event.keyCode) == kVK_RightOption
        guard isOptionKey else {
            cancelPendingManualConvert()
            return false
        }
        guard engine != nil else {
            resetPendingManualConvert()
            return false
        }

        let optionDown = event.modifierFlags.contains(.option)
        logger.debug(
            "manualConvert optionEvent keyCode=\(event.keyCode, privacy: .private) down=\(optionDown, privacy: .private) client=\(String(describing: type(of: sender)), privacy: .private)"
        )

        let hostContext = updateHostContext(client: sender)
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
        guard shouldConvert, let engine else {
            return false
        }
        guard let client = sender as? IMKTextInput else {
            return false
        }
        if shouldBypassForHostSelection(client) {
            return false
        }

        do {
            let action: TypeflowAction
            if let tail = visibleTail(in: client) {
                action = try engine.convertVisibleTail(tail)
            } else {
                action = try engine.forceSwitchToken()
            }
            _ = apply(action, client: client)
            logger.debug("manualConvert action=forceSwitch")
            return true
        } catch {
            engine.resetToken()
            return false
        }
    }

    private func apply(_ action: TypeflowAction, client: IMKTextInput) -> Bool {
        switch action {
        case .keep:
            logger.debug("action=keep")
            return false
        case .resetToken:
            logger.debug("action=resetToken")
            expectedSelectedLocation = nil
            return false
        case let .commit(character):
            logger.debug("action=commit")
            let selected = client.selectedRange()
            let text = String(character)
            client.insertText(text, replacementRange: insertionRange)
            if selected.location != NSNotFound, selected.length == 0 {
                expectedSelectedLocation = selected.location + text.utf16.count
            } else {
                expectedSelectedLocation = nil
            }
            return true
        case let .replaceToken(oldLength, replacement, _):
            let selected = client.selectedRange()
            logger.debug(
                "action=replaceToken oldLength=\(oldLength, privacy: .private) replacementLength=\(replacement.count, privacy: .private) selected={\(selected.location, privacy: .private),\(selected.length, privacy: .private)}"
            )
            guard selected.location != NSNotFound, selected.length == 0, selected.location >= oldLength else {
                rollbackReplacement(action)
                logger.debug("replaceToken rejected selected range")
                return false
            }
            let range = NSRange(location: selected.location - oldLength, length: oldLength)
            client.insertText(replacement, replacementRange: range)
            expectedSelectedLocation = range.location + replacement.utf16.count
            return true
        }
    }

    private func rollbackReplacement(_ action: TypeflowAction) {
        guard case let .replaceToken(_, _, layout) = action else {
            engine?.resetToken()
            return
        }
        engine?.resetLayout(opposite(layout))
    }

    private func opposite(_ layout: TypeflowLayout) -> TypeflowLayout {
        switch layout {
        case .english:
            return .secondary
        case .secondary:
            return .english
        }
    }

    private struct HostContextSnapshot {
        let keyProcessingDisabled: Bool
        let manualConversionDisabled: Bool
    }

    private func updateHostContext(client sender: Any!) -> HostContextSnapshot {
        let facts = hostSurfaceFacts(client: sender)
        let policyKey = hostPolicyKey(for: facts)
        let policy: TypeflowHostInputPolicy
        if policyKey == hostPolicyCacheKey, let cachedHostPolicy {
            policy = cachedHostPolicy
        } else {
            policy = hostConfig?.resolveInputPolicy(facts: facts) ?? TypeflowHostInputPolicy(
                flags: UInt32(TF_HOST_POLICY_AUTOMATIC_PROCESSING_DISABLED)
                    | UInt32(TF_HOST_POLICY_MANUAL_CONVERSION_DISABLED),
                reason: UInt8(TF_HOST_POLICY_REASON_UNAVAILABLE_HOST_CONFIG)
            )
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

        engine?.setHostContext(flags: flags)
        if policy.manualConversionDisabled {
            expectedSelectedLocation = nil
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

        return HostContextSnapshot(
            keyProcessingDisabled: policy.manualConversionDisabled,
            manualConversionDisabled: policy.manualConversionDisabled
        )
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

    private func hostSurfaceFacts(client sender: Any!) -> TypeflowHostSurfaceFacts {
        let app = NSWorkspace.shared.frontmostApplication
        let clientClass = sender.map { String(describing: type(of: $0)) }
        let ax = cachedAccessibilitySnapshot(for: app, clientClass: clientClass)
        return TypeflowHostSurfaceFacts(
            secureInput: IsSecureEventInputEnabled(),
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
        guard AXIsProcessTrusted() else {
            if !accessibilityTrustLogged {
                logger.debug("accessibility not trusted; embedded terminal surface detection unavailable")
                accessibilityTrustLogged = true
            }
            return .empty
        }

        let pid = app?.processIdentifier ?? 0
        let key = "\(pid)|\(clientClass ?? "")"
        let now = ProcessInfo.processInfo.systemUptime
        if key == accessibilityCacheKey, now < accessibilityCacheExpiresAt {
            return accessibilityCache
        }

        let snapshot = accessibilitySnapshot(for: app)
        accessibilityCacheKey = key
        accessibilityCacheExpiresAt = now + 0.10
        accessibilityCache = snapshot
        return snapshot
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
        let focusedElement = copyAXElement(appElement, attribute: kAXFocusedUIElementAttribute as CFString)
        let focusedWindow = copyAXElement(appElement, attribute: kAXFocusedWindowAttribute as CFString)
        if let focusedElement {
            _ = AXUIElementSetMessagingTimeout(focusedElement, 0.01)
        }
        if let focusedWindow {
            _ = AXUIElementSetMessagingTimeout(focusedWindow, 0.01)
        }

        return AccessibilitySnapshot(
            focusedElementRole: focusedElement.flatMap { copyAXString($0, attribute: kAXRoleAttribute as CFString) },
            focusedElementSubrole: focusedElement.flatMap { copyAXString($0, attribute: kAXSubroleAttribute as CFString) },
            focusedElementRoleDescription: focusedElement.flatMap { copyAXString($0, attribute: kAXRoleDescriptionAttribute as CFString) },
            focusedElementIdentifier: focusedElement.flatMap { copyAXString($0, attribute: kAXIdentifierAttribute as CFString) },
            focusedElementDescription: focusedElement.flatMap { copyAXString($0, attribute: kAXDescriptionAttribute as CFString) },
            focusedWindowTitle: focusedWindow.flatMap { copyAXString($0, attribute: kAXTitleAttribute as CFString) }
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
        // Swift requires a forced downcast for CoreFoundation AX types; the
        // CFTypeID guard above is the runtime type check.
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

    private func shouldBypassForHostSelection(_ client: IMKTextInput) -> Bool {
        let clientID = ObjectIdentifier(client as AnyObject)
        let selected = client.selectedRange()

        if trackedClientID != clientID {
            trackedClientID = clientID
            expectedSelectedLocation = selected.location != NSNotFound && selected.length == 0
                ? selected.location
                : nil
            engine?.resetLayout(.english)
        }

        guard selected.location != NSNotFound, selected.length == 0 else {
            engine?.resetLayout(.english)
            expectedSelectedLocation = nil
            logger.debug("host selection is not a collapsed caret; bypassing")
            return true
        }

        if let expectedSelectedLocation, selected.location != expectedSelectedLocation {
            engine?.resetLayout(.english)
            logger.debug(
                "host caret moved expected=\(expectedSelectedLocation, privacy: .private) actual=\(selected.location, privacy: .private); token reset"
            )
        }
        self.expectedSelectedLocation = selected.location
        return false
    }

    private func recordExpectedSelectionAfterHostBackspace(_ client: IMKTextInput) {
        let selected = client.selectedRange()
        guard selected.location != NSNotFound, selected.length == 0 else {
            expectedSelectedLocation = nil
            return
        }
        expectedSelectedLocation = max(0, selected.location - 1)
    }

    private func resetTrackedHostState() {
        trackedClientID = nil
        expectedSelectedLocation = nil
        hostPolicyCacheKey = ""
        cachedHostPolicy = nil
        accessibilityCacheKey = ""
        accessibilityCacheExpiresAt = 0
        accessibilityCache = .empty
    }

    private func visibleTail(
        in client: IMKTextInput,
        maxLength: Int = maxVisibleTailUTF16Length
    ) -> String? {
        let selected = client.selectedRange()
        guard selected.location != NSNotFound, selected.length == 0, selected.location > 0 else {
            return nil
        }

        let length = min(selected.location, maxLength)
        guard let attributed = client.attributedSubstring(
            from: NSRange(location: selected.location - length, length: length)
        ) else {
            return nil
        }

        return attributed.string.isEmpty ? nil : attributed.string
    }

    private var insertionRange: NSRange {
        NSRange(location: NSNotFound, length: NSNotFound)
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
