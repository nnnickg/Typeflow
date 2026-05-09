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
    private var loggedFirstKeyDown = false
    private var hostContextFlags: UInt32 = 0
    private var pendingOptionManualConvert = false
    private var manualConvertCancelled = false

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
        logger.notice("activated input controller")
        _ = updateHostContext(client: sender)
        engine?.resetToken()
    }

    override func deactivateServer(_ sender: Any!) {
        hostContextFlags = 0
        resetPendingManualConvert()
        engine?.resetToken()
    }

    override func commitComposition(_ sender: Any!) {
        resetPendingManualConvert()
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
        if !loggedFirstKeyDown {
            loggedFirstKeyDown = true
            logger.notice("received first keyDown event")
        }
        logger.notice(
            "keyDown origin=\(origin, privacy: .public) keyCode=\(keyCode, privacy: .public) hasText=\(characters?.isEmpty == false, privacy: .public) client=\(String(describing: type(of: sender)), privacy: .public)"
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
            return false
        case kVK_Return, kVK_Tab, kVK_Escape:
            _ = try? engine.endToken()
            return false
        default:
            break
        }

        do {
            if let physical = TypeflowMacKeyCode.physicalKeyIndex(for: keyCode) {
                let action = try engine.process(physicalKey: physical, modifiers: modifiers)
                return apply(action, client: client)
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

    private func processFlagsChanged(_ event: NSEvent, client sender: Any!) -> Bool {
        let isOptionKey = Int(event.keyCode) == kVK_Option || Int(event.keyCode) == kVK_RightOption
        guard isOptionKey else {
            cancelPendingManualConvert()
            return false
        }

        let optionDown = event.modifierFlags.contains(.option)
        logger.notice(
            "manualConvert optionEvent keyCode=\(event.keyCode, privacy: .public) down=\(optionDown, privacy: .public) client=\(String(describing: type(of: sender)), privacy: .public)"
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

        do {
            _ = apply(try engine.forceSwitchToken(), client: client)
            logger.notice("manualConvert action=forceSwitch")
            return true
        } catch {
            engine.resetToken()
            return false
        }
    }

    private func apply(_ action: TypeflowAction, client: IMKTextInput) -> Bool {
        switch action {
        case .keep:
            logger.notice("action=keep")
            return false
        case .resetToken:
            logger.notice("action=resetToken")
            return false
        case let .commit(character):
            logger.notice("action=commit")
            client.insertText(String(character), replacementRange: insertionRange)
            return true
        case let .replaceToken(oldLength, replacement, _):
            let selected = client.selectedRange()
            logger.notice(
                "action=replaceToken oldLength=\(oldLength, privacy: .public) replacementLength=\(replacement.count, privacy: .public) selected={\(selected.location, privacy: .public),\(selected.length, privacy: .public)}"
            )
            guard selected.location != NSNotFound, selected.length == 0, selected.location >= oldLength else {
                engine?.resetToken()
                logger.notice("replaceToken rejected selected range")
                return false
            }
            if deleteBackward(oldLength, client: client) {
                logger.notice("replaceToken used doCommandBySelector delete path")
                client.insertText(replacement, replacementRange: insertionRange)
            } else {
                logger.notice("replaceToken used absolute replacement range path")
                let range = NSRange(location: selected.location - oldLength, length: oldLength)
                client.insertText(replacement, replacementRange: range)
            }
            return true
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

        if flags != hostContextFlags {
            logger.notice(
                "hostContext secure=\(secureInput, privacy: .public) appExcluded=\(appExcluded, privacy: .public) bundleID=\(bundleID ?? "unknown", privacy: .public)"
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

    private func deleteBackward(_ count: Int, client: IMKTextInput) -> Bool {
        let object = client as AnyObject
        let commandSelector = NSSelectorFromString("doCommandBySelector:")
        guard object.responds(to: commandSelector), let method = object.method(for: commandSelector) else {
            return false
        }

        typealias CommandSender = @convention(c) (AnyObject, Selector, Selector) -> Void
        let send = unsafeBitCast(method, to: CommandSender.self)
        for _ in 0..<count {
            send(object, commandSelector, #selector(NSResponder.deleteBackward(_:)))
        }
        return true
    }

    private var insertionRange: NSRange {
        NSRange(location: NSNotFound, length: NSNotFound)
    }

    private func shouldBypassHost(_ flags: NSEvent.ModifierFlags) -> Bool {
        let bypass: NSEvent.ModifierFlags = [.control, .option, .command]
        return !flags.intersection(bypass).isEmpty
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
