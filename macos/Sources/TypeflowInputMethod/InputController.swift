import AppKit
import Carbon
import InputMethodKit
import TypeflowFFI

@objc(TypeflowInputController)
final class TypeflowInputController: IMKInputController {
    private let engine: TypeflowEngine?

    override init!(server: IMKServer!, delegate: Any!, client inputClient: Any!) {
        engine = try? TypeflowEngine()
        super.init(server: server, delegate: delegate, client: inputClient)
    }

    override func activateServer(_ sender: Any!) {
        engine?.resetToken()
    }

    override func deactivateServer(_ sender: Any!) {
        engine?.resetToken()
    }

    override func commitComposition(_ sender: Any!) {
        engine?.resetToken()
    }

    override func recognizedEvents(_ sender: Any!) -> Int {
        Int(NSEvent.EventTypeMask.keyDown.rawValue)
    }

    override func handle(_ event: NSEvent!, client sender: Any!) -> Bool {
        guard let event, event.type == .keyDown, let engine else {
            return false
        }
        guard let client = sender as? NSTextInputClient else {
            _ = try? engine.processHostBypass(modifiers: ffiModifiers(from: event.modifierFlags))
            return false
        }

        let modifiers = ffiModifiers(from: event.modifierFlags)
        if shouldBypassHost(event.modifierFlags) {
            _ = try? engine.processHostBypass(modifiers: modifiers)
            return false
        }

        switch Int(event.keyCode) {
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
            if let physical = TypeflowMacKeyCode.physicalKeyIndex(for: event.keyCode) {
                let action = try engine.process(physicalKey: physical, modifiers: modifiers)
                return apply(action, client: client)
            }

            guard let scalar = singleScalar(from: event.characters) else {
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

    private func apply(_ action: TypeflowAction, client: NSTextInputClient) -> Bool {
        switch action {
        case .keep:
            return false
        case .resetToken:
            return true
        case let .commit(character):
            client.insertText(String(character), replacementRange: insertionRange)
            return true
        case let .replaceToken(oldLength, replacement, _):
            let selected = client.selectedRange()
            guard selected.location != NSNotFound, selected.length == 0, selected.location >= oldLength else {
                engine?.resetToken()
                return false
            }
            let range = NSRange(location: selected.location - oldLength, length: oldLength)
            client.insertText(replacement, replacementRange: range)
            return true
        }
    }

    private var insertionRange: NSRange {
        NSRange(location: NSNotFound, length: 0)
    }

    private func shouldBypassHost(_ flags: NSEvent.ModifierFlags) -> Bool {
        let bypass: NSEvent.ModifierFlags = [.control, .option, .command, .function]
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
        if flags.contains(.function) {
            modifiers |= UInt8(TF_MOD_FUNCTION)
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
