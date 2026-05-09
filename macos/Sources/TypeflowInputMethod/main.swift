import AppKit
import Foundation
import InputMethodKit
import os

private let logger = Logger(
    subsystem: "io.github.nnnickg.typeflow.inputmethod.Typeflow",
    category: "Main"
)

private let bundle = Bundle.main
private let connectionName = bundle.object(forInfoDictionaryKey: "InputMethodConnectionName") as? String
    ?? "Typeflow_1_Connection"
private let bundleIdentifier = bundle.bundleIdentifier
    ?? "io.github.nnnickg.typeflow.inputmethod.Typeflow"

NSApplication.shared.setActivationPolicy(.accessory)

guard let server = IMKServer(
    name: connectionName,
    bundleIdentifier: bundleIdentifier
) else {
    logger.error("failed to create IMKServer")
    exit(1)
}

logger.notice("started IMKServer")
withExtendedLifetime(server) {
    NSApplication.shared.run()
}
