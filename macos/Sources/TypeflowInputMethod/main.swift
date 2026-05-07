import AppKit
import Foundation
import InputMethodKit

private let bundle = Bundle.main
private let connectionName = bundle.object(forInfoDictionaryKey: "InputMethodConnectionName") as? String
    ?? "Typeflow_1_Connection"
private let bundleIdentifier = bundle.bundleIdentifier
    ?? "io.github.nnnickg.typeflow.inputmethod.Typeflow"

private var server: IMKServer? = IMKServer(
    name: connectionName,
    bundleIdentifier: bundleIdentifier
)

NSApplication.shared.setActivationPolicy(.accessory)
NSApplication.shared.run()
_ = server
