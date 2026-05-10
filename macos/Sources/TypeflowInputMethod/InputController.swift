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
private let selectionPollAfterIdleSeconds: TimeInterval = 0.20
private let selectionPollIntervalSeconds: TimeInterval = 1.0
private let accessibilityTrustCacheTTLSeconds: TimeInterval = 1.0
private let maxDeferredReplacementRetryCount = 2

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
    private var expectedSelectedLocation: Int?
    private var deferredToken: DeferredTokenSession?
    private var lastKeyDownAt: TimeInterval = 0
    private var lastSelectionCheckAt: TimeInterval = 0

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
        if let client = sender as? IMKTextInput {
            _ = flushDeferredReplacementWithAuthoritativeSelection(client: client, reason: "commitComposition")
        }
        resetPendingManualConvert()
        resetTrackedHostState()
        engine?.resetToken()
    }

    override func composedString(_ sender: Any!) -> Any! {
        nil
    }

    override func originalString(_ sender: Any!) -> NSAttributedString! {
        NSAttributedString(string: "")
    }

    override func selectionRange() -> NSRange {
        NSRange(location: 0, length: 0)
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
        let previousKeyDownAt = lastKeyDownAt
        lastKeyDownAt = processStarted
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
        if shouldBypassHost(modifierFlags) {
            if let client = sender as? IMKTextInput {
                _ = flushDeferredReplacementWithAuthoritativeSelection(client: client, reason: "hostBypass")
            }
            _ = try? measured("ffi.processHostBypass", thresholdMs: slowCallThresholdMs) {
                try engine.processHostBypass(modifiers: modifiers)
            }
            resetDeferredSession(reason: "hostBypass")
            return false
        }

        switch Int(keyCode) {
        case kVK_Return, kVK_Tab, kVK_Escape:
            if let client = sender as? IMKTextInput {
                _ = flushDeferredReplacementWithAuthoritativeSelection(client: client, reason: "endToken")
            }
            _ = try? measured("ffi.endToken", thresholdMs: slowCallThresholdMs) {
                try engine.endToken()
            }
            expectedSelectedLocation = nil
            resetDeferredSession(reason: "endToken")
            return false
        default:
            break
        }

        if shouldBypassNonTextKey(keyCode: keyCode, characters: characters) {
            if let client = sender as? IMKTextInput {
                _ = flushDeferredReplacementWithAuthoritativeSelection(client: client, reason: "nonText")
            }
            engine.resetLayout(.english)
            expectedSelectedLocation = nil
            resetDeferredSession(reason: "nonText")
            return false
        }

        guard let client = sender as? IMKTextInput else {
            _ = try? measured("ffi.processHostBypass", thresholdMs: slowCallThresholdMs) {
                try engine.processHostBypass(modifiers: modifiers)
            }
            resetDeferredSession(reason: "missingClient")
            return false
        }
        let hostContext = measured("updateHostContext", thresholdMs: slowHostThresholdMs) {
            updateHostContext(client: sender)
        }
        if hostContext.keyProcessingDisabled {
            resetDeferredSession(reason: "disabled")
            return false
        }

        do {
            let isBackspace = Int(keyCode) == kVK_Delete
            let selection = hostSelectionSnapshot(
                client,
                allowPrediction: !isBackspace,
                previousEventAt: previousKeyDownAt
            )
            if isBackspace {
                return handleDeferredBackspace(
                    engine: engine,
                    client: client,
                    selection: selection,
                    modifiers: modifiers
                )
            }
            if selection.bypassesTextMutation {
                _ = try? measured("ffi.processHostBypass", thresholdMs: slowCallThresholdMs) {
                    try engine.processHostBypass(modifiers: modifiers)
                }
                resetDeferredSession(reason: "selectionBypass")
                return false
            }

            if let physical = TypeflowMacKeyCode.physicalKeyIndex(for: keyCode) {
                let action = try measured("ffi.process", thresholdMs: slowCallThresholdMs) {
                    try engine.process(physicalKey: physical, modifiers: modifiers)
                }
                if engine.tokenLength == 0 {
                    let boundarySelection = selection.authoritative
                        ? selection
                        : hostSelectionSnapshot(
                            client,
                            allowPrediction: false,
                            previousEventAt: previousKeyDownAt
                        )
                    _ = flushDeferredReplacement(
                        client: client,
                        selection: boundarySelection,
                        reason: "boundary"
                    )
                    resetDeferredSession(reason: "boundary")
                    recordExpectedSelectionAfterNativeInsertion(
                        selected: boundarySelection.selected,
                        characters: characters
                    )
                    return false
                }

                return beginOrAdvanceDeferredSession(
                    action: action,
                    engine: engine,
                    client: client,
                    selected: selection.selected,
                    characters: characters
                )
            }

            let literalSelection = selection.authoritative
                ? selection
                : hostSelectionSnapshot(
                    client,
                    allowPrediction: false,
                    previousEventAt: previousKeyDownAt
                )
            _ = flushDeferredReplacement(
                client: client,
                selection: literalSelection,
                reason: "literal"
            )
            guard let scalar = singleScalar(from: characters) else {
                _ = try? engine.endToken()
                resetDeferredSession(reason: "literalWithoutScalar")
                return false
            }
            _ = try measured("ffi.processLiteral", thresholdMs: slowCallThresholdMs) {
                try engine.processLiteral(scalar)
            }
            resetDeferredSession(reason: "literal")
            recordExpectedSelectionAfterNativeInsertion(selected: literalSelection.selected, characters: characters)
            return false
        } catch {
            engine.resetToken()
            resetDeferredSession(reason: "error")
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
        return handleDeferredManualConvert(engine: engine, client: client)
    }

    private func deferredTokenBelongs(to client: IMKTextInput) -> Bool {
        deferredToken?.clientID == ObjectIdentifier(client as AnyObject)
    }

    private func beginOrAdvanceDeferredSession(
        action: TypeflowAction,
        engine: TypeflowEngine,
        client: IMKTextInput,
        selected: NSRange,
        characters: String?
    ) -> Bool {
        guard let characters,
              !characters.isEmpty,
              selected.location != NSNotFound,
              selected.length == 0
        else {
            engine.resetToken()
            expectedSelectedLocation = nil
            resetDeferredSession(reason: "untrackableKey")
            return false
        }

        let clientID = ObjectIdentifier(client as AnyObject)
        let existingSession = deferredToken?.clientID == clientID ? deferredToken : nil
        guard let desired = desiredTokenText(
            engine: engine,
            fallback: action,
            existingSession: existingSession
        ) else {
            engine.resetToken()
            expectedSelectedLocation = nil
            resetDeferredSession(reason: "missingDesiredToken")
            return false
        }

        var session = existingSession
            ?? DeferredTokenSession(
                clientID: clientID,
                baseLocation: selected.location,
                nativeText: "",
                visibleText: "",
                desiredText: "",
                layout: desired.layout,
                lastCaretLocation: selected.location,
                lastCaretAuthoritative: true,
                pendingHostMutationStartCaret: nil,
                pendingHostMutationReadyCaret: nil,
                applyGeneration: 0,
                retryCount: 0
            )

        let mutationStartCaret = selected.location
        session.nativeText.append(characters)
        session.visibleText.append(characters)
        session.desiredText = desired.text
        session.layout = desired.layout
        session.lastCaretLocation = session.visibleEndLocation
        session.lastCaretAuthoritative = false
        session.pendingHostMutationStartCaret = mutationStartCaret
        session.pendingHostMutationReadyCaret = session.visibleEndLocation
        expectedSelectedLocation = session.visibleEndLocation
        deferredToken = session

        logger.debug(
            "action=deferredAdvance visibleLength=\(session.visibleText.utf16.count, privacy: .private) desiredLength=\(session.desiredText.utf16.count, privacy: .private) pending=\(session.needsReplacement, privacy: .private)"
        )
        if session.needsReplacement {
            scheduleDeferredReplacement(client: client, reason: "key")
        }
        return false
    }

    private func desiredTokenText(
        engine: TypeflowEngine,
        fallback action: TypeflowAction,
        existingSession: DeferredTokenSession?
    ) -> DeferredDesiredToken? {
        switch action {
        case let .replaceToken(_, replacement, layout):
            return DeferredDesiredToken(text: replacement, layout: layout)
        case let .commit(character):
            let layout = existingSession?.layout ?? (try? engine.currentLayout) ?? .english
            return DeferredDesiredToken(
                text: (existingSession?.desiredText ?? "") + String(character),
                layout: layout
            )
        case .keep, .resetToken:
            break
        }

        if case let .some(.replaceToken(_, replacement, layout)) = try? engine.currentToken() {
            return DeferredDesiredToken(text: replacement, layout: layout)
        }
        return nil
    }

    @discardableResult
    private func flushDeferredReplacementWithAuthoritativeSelection(
        client: IMKTextInput,
        reason: String
    ) -> Bool {
        let selection = hostSelectionSnapshot(
            client,
            allowPrediction: false,
            previousEventAt: lastKeyDownAt
        )
        guard !selection.bypassesTextMutation else {
            return false
        }
        return flushDeferredReplacement(client: client, selection: selection, reason: reason)
    }

    @discardableResult
    private func flushDeferredReplacement(
        client: IMKTextInput,
        selection: HostSelectionSnapshot,
        reason: String
    ) -> Bool {
        let attempt = applyDeferredReplacementNow(client: client, selection: selection, reason: reason)
        guard let session = deferredToken,
              session.clientID == ObjectIdentifier(client as AnyObject)
        else {
            return attempt.appliedHostMutation
        }

        if session.needsReplacement {
            logger.debug("deferredFlush left pending reason=\(reason, privacy: .public)")
        } else {
            resetDeferredSession(reason: reason)
        }
        return attempt.appliedHostMutation
    }

    private func applyDeferredReplacementNow(
        client: IMKTextInput,
        selection: HostSelectionSnapshot,
        reason: String
    ) -> DeferredReplacementAttempt {
        let clientID = ObjectIdentifier(client as AnyObject)
        guard var session = deferredToken,
              session.clientID == clientID,
              session.needsReplacement
        else {
            return .notNeeded
        }
        guard selection.selected.location != NSNotFound,
              selection.selected.length == 0
        else {
            return .abandoned
        }

        let caret = selection.selected.location
        guard caret == session.visibleEndLocation else {
            if caret == session.desiredEndLocation {
                session.visibleText = session.desiredText
                session.lastCaretLocation = caret
                session.lastCaretAuthoritative = selection.authoritative
                session.pendingHostMutationStartCaret = nil
                session.pendingHostMutationReadyCaret = nil
                expectedSelectedLocation = caret
                deferredToken = session
                return .alreadySatisfied
            } else if session.isWaitingForHostMutation(caret: caret) {
                session.lastCaretLocation = caret
                session.lastCaretAuthoritative = selection.authoritative
                deferredToken = session
                logger.debug(
                    "deferredReplace wait reason=\(reason, privacy: .public) caret=\(caret, privacy: .private) expected=\(session.visibleEndLocation, privacy: .private)"
                )
                return .waitingForCaret
            } else {
                session.lastCaretLocation = caret
                session.lastCaretAuthoritative = selection.authoritative
                deferredToken = session
                logger.debug(
                    "deferredReplace abandon reason=\(reason, privacy: .public) caret=\(caret, privacy: .private) expected=\(session.visibleEndLocation, privacy: .private)"
                )
                return .abandoned
            }
        }

        let range = NSRange(location: session.baseLocation, length: session.visibleText.utf16.count)
        measured("insertText.deferredReplace.\(reason)", thresholdMs: slowHostThresholdMs) {
            client.insertText(session.desiredText, replacementRange: range)
        }
        session.visibleText = session.desiredText
        session.lastCaretLocation = session.desiredEndLocation
        session.lastCaretAuthoritative = false
        session.pendingHostMutationStartCaret = nil
        session.pendingHostMutationReadyCaret = nil
        session.retryCount = 0
        expectedSelectedLocation = session.desiredEndLocation
        deferredToken = session
        logger.debug(
            "action=deferredReplace reason=\(reason, privacy: .public) visibleLength=\(session.visibleText.utf16.count, privacy: .private)"
        )
        return .applied
    }

    private func scheduleDeferredReplacement(client: IMKTextInput, reason: String) {
        let clientID = ObjectIdentifier(client as AnyObject)
        guard var session = deferredToken,
              session.clientID == clientID,
              session.needsReplacement
        else {
            return
        }

        session.applyGeneration &+= 1
        session.retryCount = 0
        let generation = session.applyGeneration
        deferredToken = session

        DispatchQueue.main.async { [weak self, weak clientObject = client as AnyObject] in
            guard let self,
                  let client = clientObject as? IMKTextInput
            else {
                return
            }
            self.applyScheduledDeferredReplacement(
                client: client,
                generation: generation,
                reason: reason
            )
        }
    }

    private func applyScheduledDeferredReplacement(
        client: IMKTextInput,
        generation: UInt64,
        reason: String
    ) {
        let clientID = ObjectIdentifier(client as AnyObject)
        guard let session = deferredToken,
              session.clientID == clientID,
              session.applyGeneration == generation,
              session.needsReplacement
        else {
            return
        }

        let selected = measured("selectedRange.deferred", thresholdMs: slowCallThresholdMs) {
            client.selectedRange()
        }
        let selection = HostSelectionSnapshot(
            selected: selected,
            bypassesTextMutation: selected.location == NSNotFound || selected.length != 0,
            authoritative: true
        )
        let attempt = applyDeferredReplacementNow(
            client: client,
            selection: selection,
            reason: reason
        )
        switch attempt {
        case .applied, .alreadySatisfied, .notNeeded:
            return
        case .abandoned:
            engine?.resetToken()
            resetDeferredSession(reason: "deferredCaretMismatch")
            return
        case .waitingForCaret:
            break
        }

        guard var current = deferredToken,
              current.clientID == clientID,
              current.applyGeneration == generation,
              current.needsReplacement,
              current.retryCount < maxDeferredReplacementRetryCount
        else {
            return
        }

        current.retryCount += 1
        deferredToken = current
        DispatchQueue.main.async { [weak self, weak clientObject = client as AnyObject] in
            guard let self,
                  let client = clientObject as? IMKTextInput
            else {
                return
            }
            self.applyScheduledDeferredReplacement(
                client: client,
                generation: generation,
                reason: reason
            )
        }
    }

    private func handleDeferredBackspace(
        engine: TypeflowEngine,
        client: IMKTextInput,
        selection: HostSelectionSnapshot,
        modifiers: UInt8
    ) -> Bool {
        if selection.bypassesTextMutation {
            _ = try? measured("ffi.processHostBypass", thresholdMs: slowCallThresholdMs) {
                try engine.processHostBypass(modifiers: modifiers)
            }
            resetDeferredSession(reason: "backspaceBypass")
            return false
        }

        let clientID = ObjectIdentifier(client as AnyObject)
        guard var session = deferredToken,
              session.clientID == clientID
        else {
            _ = try? measured("ffi.processBackspace", thresholdMs: slowCallThresholdMs) {
                try engine.processBackspace()
            }
            recordExpectedSelectionAfterHostBackspace(selection.selected)
            return false
        }

        guard selection.selected.location == session.visibleEndLocation,
              selection.selected.length == 0
        else {
            engine.resetToken()
            resetDeferredSession(reason: "backspaceCaretMoved")
            expectedSelectedLocation = nil
            return false
        }

        _ = try? measured("ffi.processBackspace", thresholdMs: slowCallThresholdMs) {
            try engine.processBackspace()
        }
        guard let deleted = session.visibleText.last else {
            resetDeferredSession(reason: "backspaceEmpty")
            recordExpectedSelectionAfterHostBackspace(selection.selected)
            return false
        }

        session.visibleText.removeLast()
        if !session.nativeText.isEmpty {
            session.nativeText.removeLast()
        }

        let caretAfterBackspace = max(
            session.baseLocation,
            selection.selected.location - String(deleted).utf16.count
        )
        expectedSelectedLocation = caretAfterBackspace
        if engine.tokenLength == 0 || session.visibleText.isEmpty {
            resetDeferredSession(reason: "backspaceCleared")
            logger.debug("action=deferredBackspaceClear")
            return false
        }

        if let desired = desiredTokenText(engine: engine, fallback: .keep, existingSession: session) {
            session.desiredText = desired.text
            session.layout = desired.layout
        } else {
            engine.resetToken()
            resetDeferredSession(reason: "backspaceLostToken")
            return false
        }

        session.lastCaretLocation = caretAfterBackspace
        session.lastCaretAuthoritative = false
        session.pendingHostMutationStartCaret = selection.selected.location
        session.pendingHostMutationReadyCaret = caretAfterBackspace
        deferredToken = session
        logger.debug(
            "action=deferredBackspace visibleLength=\(session.visibleText.utf16.count, privacy: .private) pending=\(session.needsReplacement, privacy: .private)"
        )
        if session.needsReplacement {
            scheduleDeferredReplacement(client: client, reason: "backspace")
        }
        return false
    }

    private func handleDeferredManualConvert(engine: TypeflowEngine, client: IMKTextInput) -> Bool {
        let selection = hostSelectionSnapshot(
            client,
            allowPrediction: false,
            previousEventAt: lastKeyDownAt
        )
        guard !selection.bypassesTextMutation,
              var session = deferredToken,
              session.clientID == ObjectIdentifier(client as AnyObject)
        else {
            logger.debug("manualConvert action=noDeferredSession")
            return false
        }

        do {
            let action = try measured("ffi.forceSwitchToken", thresholdMs: slowCallThresholdMs) {
                try engine.forceSwitchToken()
            }
            guard let desired = desiredTokenText(
                engine: engine,
                fallback: action,
                existingSession: session
            ) else {
                logger.debug("manualConvert action=noToken")
                return true
            }

            session.desiredText = desired.text
            session.layout = desired.layout
            session.lastCaretLocation = selection.selected.location
            session.lastCaretAuthoritative = true
            deferredToken = session
            _ = applyDeferredReplacementNow(client: client, selection: selection, reason: "manual")
            logger.debug("manualConvert action=deferredForceSwitch")
            return true
        } catch {
            engine.resetToken()
            resetDeferredSession(reason: "manualError")
            return false
        }
    }

    private func recordExpectedSelectionAfterNativeInsertion(selected: NSRange, characters: String?) {
        guard let characters,
              selected.location != NSNotFound,
              selected.length == 0
        else {
            expectedSelectedLocation = nil
            return
        }

        let insertionLocation = expectedSelectedLocation ?? selected.location
        expectedSelectedLocation = insertionLocation + characters.utf16.count
    }

    private func recordExpectedSelectionAfterHostBackspace(_ selected: NSRange) {
        guard selected.location != NSNotFound, selected.length == 0 else {
            expectedSelectedLocation = nil
            return
        }
        expectedSelectedLocation = max(0, selected.location - 1)
    }

    private func resetDeferredSession(reason: String) {
        if deferredToken != nil {
            logger.debug("deferredReset reason=\(reason, privacy: .public)")
        }
        deferredToken = nil
    }

    private struct HostContextSnapshot {
        let keyProcessingDisabled: Bool
        let manualConversionDisabled: Bool
    }

    private struct HostSelectionSnapshot {
        let selected: NSRange
        let bypassesTextMutation: Bool
        let authoritative: Bool
    }

    private struct DeferredDesiredToken {
        let text: String
        let layout: TypeflowLayout
    }

    private enum DeferredReplacementAttempt {
        case applied
        case alreadySatisfied
        case waitingForCaret
        case abandoned
        case notNeeded

        var appliedHostMutation: Bool {
            self == .applied
        }
    }

    private struct DeferredTokenSession {
        let clientID: ObjectIdentifier
        let baseLocation: Int
        var nativeText: String
        var visibleText: String
        var desiredText: String
        var layout: TypeflowLayout
        var lastCaretLocation: Int
        var lastCaretAuthoritative: Bool
        var pendingHostMutationStartCaret: Int?
        var pendingHostMutationReadyCaret: Int?
        var applyGeneration: UInt64
        var retryCount: Int

        var visibleEndLocation: Int {
            baseLocation + visibleText.utf16.count
        }

        var desiredEndLocation: Int {
            baseLocation + desiredText.utf16.count
        }

        var needsReplacement: Bool {
            !visibleText.isEmpty && visibleText != desiredText
        }

        func isWaitingForHostMutation(caret: Int) -> Bool {
            guard let start = pendingHostMutationStartCaret,
                  let ready = pendingHostMutationReadyCaret,
                  caret != ready
            else {
                return false
            }

            let lowerBound = min(start, ready)
            let upperBound = max(start, ready)
            return (lowerBound...upperBound).contains(caret)
        }
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
                expectedSelectedLocation = nil
                resetDeferredSession(reason: "cachedDisabled")
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
            expectedSelectedLocation = nil
            resetDeferredSession(reason: "disabled")
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

    private func hostSelectionSnapshot(
        _ client: IMKTextInput,
        allowPrediction: Bool = false,
        previousEventAt: TimeInterval = 0
    ) -> HostSelectionSnapshot {
        let clientID = ObjectIdentifier(client as AnyObject)
        let now = ProcessInfo.processInfo.systemUptime
        if allowPrediction,
           trackedClientID == clientID,
           let expectedSelectedLocation,
           shouldUsePredictedSelection(now: now, previousEventAt: previousEventAt)
        {
            return HostSelectionSnapshot(
                selected: NSRange(location: expectedSelectedLocation, length: 0),
                bypassesTextMutation: false,
                authoritative: false
            )
        }

        let selected = measured("selectedRange", thresholdMs: slowCallThresholdMs) {
            client.selectedRange()
        }
        lastSelectionCheckAt = now

        if trackedClientID != clientID {
            trackedClientID = clientID
            expectedSelectedLocation = selected.location != NSNotFound && selected.length == 0
                ? selected.location
                : nil
            engine?.resetLayout(.english)
            resetDeferredSession(reason: "clientChanged")
        }

        guard selected.location != NSNotFound, selected.length == 0 else {
            engine?.resetLayout(.english)
            expectedSelectedLocation = nil
            resetDeferredSession(reason: "nonCollapsedSelection")
            logger.debug("host selection is not a collapsed caret; bypassing")
            return HostSelectionSnapshot(
                selected: selected,
                bypassesTextMutation: true,
                authoritative: true
            )
        }

        if let expectedSelectedLocation, selected.location != expectedSelectedLocation {
            engine?.resetLayout(.english)
            resetDeferredSession(reason: "caretMoved")
            logger.debug(
                "host caret moved expected=\(expectedSelectedLocation, privacy: .private) actual=\(selected.location, privacy: .private); token reset"
            )
        }
        self.expectedSelectedLocation = selected.location
        return HostSelectionSnapshot(
            selected: selected,
            bypassesTextMutation: false,
            authoritative: true
        )
    }

    private func shouldUsePredictedSelection(now: TimeInterval, previousEventAt: TimeInterval) -> Bool {
        guard lastSelectionCheckAt > 0,
              now - lastSelectionCheckAt <= selectionPollIntervalSeconds,
              previousEventAt > 0,
              now - previousEventAt <= selectionPollAfterIdleSeconds
        else {
            return false
        }
        return true
    }

    private func resetTrackedHostState() {
        trackedClientID = nil
        expectedSelectedLocation = nil
        resetDeferredSession(reason: "trackedHostReset")
        lastKeyDownAt = 0
        lastSelectionCheckAt = 0
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
