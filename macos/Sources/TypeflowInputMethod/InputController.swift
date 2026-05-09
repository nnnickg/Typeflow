import AppKit
import Carbon
import InputMethodKit
import os
import TypeflowFFI

private let logger = Logger(
    subsystem: "io.github.nnnickg.typeflow.inputmethod.Typeflow",
    category: "InputController"
)

@objc(TypeflowInputController)
final class TypeflowInputController: IMKInputController {
    private let hostConfig: TypeflowHostConfig
    private let engine: TypeflowEngine?
    private var hostContextFlags: UInt32 = 0
    private var pendingOptionManualConvert = false
    private var manualConvertCancelled = false
    private var trackedClientID: ObjectIdentifier?
    private var expectedSelectedLocation: Int?

    override init!(server: IMKServer!, delegate: Any!, client inputClient: Any!) {
        let loadedConfig = TypeflowHostConfig.load()
        hostConfig = loadedConfig
        let engineSource: String?
        let engineError: String?
        do {
            let initializedEngine = try TypeflowEngine(hostConfig: loadedConfig)
            engine = initializedEngine
            engineSource = initializedEngine.sourceDescription
            engineError = nil
        } catch {
            engine = nil
            engineSource = nil
            engineError = String(describing: error)
        }
        super.init(server: server, delegate: delegate, client: inputClient)
        if let engineError {
            logger.error("failed to initialize Typeflow engine: \(engineError, privacy: .public)")
        } else {
            logger.notice(
                "initialized input controller source=\(engineSource ?? "unknown", privacy: .public) config=\(loadedConfig.sourcePath == nil ? "defaults" : "loaded", privacy: .public) excludedApps=\(loadedConfig.excludedBundleIDs.count, privacy: .public) manualConvert=option"
            )
        }
    }

    override func activateServer(_ sender: Any!) {
        logger.debug("activated input controller")
        _ = updateHostContext(client: sender)
        resetTrackedHostState()
        engine?.resetToken()
    }

    override func deactivateServer(_ sender: Any!) {
        hostContextFlags = 0
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
        if updateHostContext(client: sender) {
            return false
        }
        logger.debug(
            "keyDown origin=\(origin, privacy: .public) keyCode=\(keyCode, privacy: .private) hasText=\(characters?.isEmpty == false, privacy: .private) client=\(String(describing: type(of: sender)), privacy: .private)"
        )
        guard let client = sender as? IMKTextInput else {
            _ = try? engine.processHostBypass(modifiers: ffiModifiers(from: modifierFlags))
            return false
        }

        let modifiers = ffiModifiers(from: modifierFlags)
        if shouldBypassHost(modifierFlags) {
            _ = try? engine.processHostBypass(modifiers: modifiers)
            return false
        }

        switch Int(keyCode) {
        case kVK_Delete:
            _ = try? engine.processBackspace()
            recordExpectedSelectionAfterHostBackspace(client)
            return false
        case kVK_Return, kVK_Tab, kVK_Escape:
            _ = try? engine.endToken()
            expectedSelectedLocation = nil
            return false
        default:
            break
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

        let optionDown = event.modifierFlags.contains(.option)
        logger.debug(
            "manualConvert optionEvent keyCode=\(event.keyCode, privacy: .private) down=\(optionDown, privacy: .private) client=\(String(describing: type(of: sender)), privacy: .private)"
        )

        if updateHostContext(client: sender) {
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
            client.insertText(String(character), replacementRange: insertionRange)
            if selected.location != NSNotFound, selected.length == 0 {
                expectedSelectedLocation = selected.location + 1
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
            expectedSelectedLocation = range.location + replacement.count
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

    private func updateHostContext(client sender: Any!) -> Bool {
        var flags: UInt32 = 0
        let secureInput = IsSecureEventInputEnabled()
        let bundleID = NSWorkspace.shared.frontmostApplication?.bundleIdentifier
        let appExcluded = bundleID.map { hostConfig.excludedBundleIDs.contains($0) } ?? false

        if secureInput {
            flags |= UInt32(TF_CONTEXT_SECURE_INPUT)
        }
        if appExcluded {
            flags |= UInt32(TF_CONTEXT_APP_EXCLUDED)
        }

        engine?.setHostContext(flags: flags)
        if flags != 0 {
            expectedSelectedLocation = nil
        }

        if flags != hostContextFlags {
            logger.debug(
                "hostContext secure=\(secureInput, privacy: .private) appExcluded=\(appExcluded, privacy: .private) bundleID=\(bundleID ?? "unknown", privacy: .private)"
            )
            hostContextFlags = flags
        }

        return flags != 0
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
    }

    private func visibleTail(in client: IMKTextInput, maxLength: Int = 128) -> String? {
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

    private func ffiModifiers(from flags: NSEvent.ModifierFlags) -> UInt8 {
        var modifiers: UInt8 = 0
        if flags.contains(.shift) || flags.contains(.capsLock) {
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
