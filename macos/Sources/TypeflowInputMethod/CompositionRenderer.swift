import InputMethodKit

private let insertionRange = NSRange(location: NSNotFound, length: 0)

enum TypeflowCompositionRendererKind: Equatable {
    case markedText
    case directCommit
}

protocol TypeflowCompositionRenderer {
    var kind: TypeflowCompositionRendererKind { get }

    func render(text: String, client: IMKTextInput)
    func commit(text: String, client: IMKTextInput)
    func clear(client: IMKTextInput)
}

struct TypeflowMarkedTextRenderer: TypeflowCompositionRenderer {
    let kind = TypeflowCompositionRendererKind.markedText

    func render(text: String, client: IMKTextInput) {
        client.setMarkedText(
            text,
            selectionRange: NSRange(location: text.utf16.count, length: 0),
            replacementRange: insertionRange
        )
    }

    func commit(text: String, client: IMKTextInput) {
        client.insertText(text, replacementRange: insertionRange)
    }

    func clear(client: IMKTextInput) {
        client.setMarkedText(
            "",
            selectionRange: NSRange(location: 0, length: 0),
            replacementRange: insertionRange
        )
    }
}

struct TypeflowDirectCommitRenderer: TypeflowCompositionRenderer {
    let kind = TypeflowCompositionRendererKind.directCommit

    func render(text: String, client: IMKTextInput) {}

    func commit(text: String, client: IMKTextInput) {
        client.insertText(text, replacementRange: insertionRange)
    }

    func clear(client: IMKTextInput) {}
}
