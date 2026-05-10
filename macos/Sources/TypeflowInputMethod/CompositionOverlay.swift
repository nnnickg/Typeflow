import AppKit
import InputMethodKit

struct TypeflowOverlayAnchor {
    let lineRect: NSRect
    let windowLevel: Int
    let style: TypeflowOverlayStyle
}

struct TypeflowOverlayStyle {
    let font: NSFont
    let textColor: NSColor

    static func from(attributes: [AnyHashable: Any]?) -> TypeflowOverlayStyle {
        let font = attribute(NSAttributedString.Key.font, from: attributes) as? NSFont
            ?? NSFont.systemFont(ofSize: NSFont.systemFontSize)
        let textColor = attribute(NSAttributedString.Key.foregroundColor, from: attributes) as? NSColor
            ?? NSColor.labelColor
        return TypeflowOverlayStyle(font: font, textColor: textColor)
    }

    func textAttributes() -> [NSAttributedString.Key: Any] {
        [
            .font: font,
            .foregroundColor: textColor,
            .ligature: 0,
        ]
    }
}

private func attribute(
    _ key: NSAttributedString.Key,
    from attributes: [AnyHashable: Any]?
) -> Any? {
    guard let attributes else {
        return nil
    }
    return attributes[key] ?? attributes[key.rawValue]
}

final class TypeflowCompositionOverlay {
    private let panel: TypeflowOverlayPanel
    private let content: TypeflowOverlayView

    init() {
        content = TypeflowOverlayView(frame: .zero)
        panel = TypeflowOverlayPanel(
            contentRect: .zero,
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: true
        )
        panel.contentView = content
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = false
        panel.ignoresMouseEvents = true
        panel.hidesOnDeactivate = false
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        panel.animationBehavior = .none
        panel.isReleasedWhenClosed = false
    }

    func render(text: String, anchor: TypeflowOverlayAnchor) {
        guard !text.isEmpty else {
            clear()
            return
        }

        let frame = windowFrame(for: text, anchor: anchor)
        content.update(text: text, style: anchor.style, height: frame.height)
        panel.level = NSWindow.Level(rawValue: anchor.windowLevel + 1)
        panel.setFrame(frame, display: true)
        panel.orderFrontRegardless()
    }

    func clear() {
        panel.orderOut(nil)
        content.clear()
    }

    private func windowFrame(for text: String, anchor: TypeflowOverlayAnchor) -> NSRect {
        let lineRect = normalized(anchor.lineRect)
        let attributes = anchor.style.textAttributes()
        let textSize = (text as NSString).size(withAttributes: attributes)
        let fontHeight = ceil(anchor.style.font.ascender - anchor.style.font.descender + anchor.style.font.leading)
        let height = max(ceil(lineRect.height), fontHeight, 14)
        let width = max(ceil(textSize.width) + TypeflowOverlayView.caretWidth + 2, 2)
        return NSRect(x: lineRect.minX, y: lineRect.minY, width: width, height: height)
    }

    private func normalized(_ rect: NSRect) -> NSRect {
        let x = min(rect.minX, rect.maxX)
        let y = min(rect.minY, rect.maxY)
        let width = abs(rect.width)
        let height = abs(rect.height)
        return NSRect(x: x, y: y, width: width, height: height)
    }
}

private final class TypeflowOverlayPanel: NSPanel {
    override var canBecomeKey: Bool {
        false
    }

    override var canBecomeMain: Bool {
        false
    }
}

private final class TypeflowOverlayView: NSView {
    static let caretWidth: CGFloat = 1

    private var text = ""
    private var style = TypeflowOverlayStyle(
        font: NSFont.systemFont(ofSize: NSFont.systemFontSize),
        textColor: .labelColor
    )
    private var cachedTextSize = NSSize.zero

    override var isFlipped: Bool {
        false
    }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        layer?.backgroundColor = NSColor.clear.cgColor
    }

    required init?(coder: NSCoder) {
        nil
    }

    func update(text: String, style: TypeflowOverlayStyle, height: CGFloat) {
        self.text = text
        self.style = style
        cachedTextSize = (text as NSString).size(withAttributes: style.textAttributes())
        frame = NSRect(x: 0, y: 0, width: ceil(cachedTextSize.width) + Self.caretWidth + 2, height: height)
        needsDisplay = true
    }

    func clear() {
        text = ""
        cachedTextSize = .zero
        needsDisplay = true
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard !text.isEmpty else {
            return
        }

        let attributes = style.textAttributes()
        let y = floor((bounds.height - cachedTextSize.height) / 2)
        (text as NSString).draw(at: NSPoint(x: 0, y: y), withAttributes: attributes)

        let caretX = ceil(cachedTextSize.width) + 1
        let caretHeight = max(bounds.height - 2, 1)
        let caretRect = NSRect(x: caretX, y: 1, width: Self.caretWidth, height: caretHeight)
        style.textColor.setFill()
        caretRect.fill()
    }
}
