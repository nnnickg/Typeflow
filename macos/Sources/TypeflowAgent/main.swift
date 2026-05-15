import AppKit
import ApplicationServices
import Carbon
import Foundation
import os
import ServiceManagement
#if SWIFT_PACKAGE
import TypeflowKit
#endif
import TypeflowFFI

private let logger = Logger(
    subsystem: "io.github.nnnickg.typeflow.agent",
    category: "Agent"
)
private let performanceLogger = Logger(
    subsystem: "io.github.nnnickg.typeflow.agent",
    category: "Performance"
)
private let performanceLogAll = ProcessInfo.processInfo.environment["TYPEFLOW_PERF_LOG_ALL"] == "1"
private let slowProcessThresholdMs = 2.0
private let slowHostThresholdMs = 0.75
private let slowCallThresholdMs = 0.25
private let accessibilityRefreshDebounceSeconds: TimeInterval = 0.075
private let accessibilitySlowRefreshThresholdMs = 20.0
private let accessibilitySlowRefreshLimit = 3
private let accessibilityBackoffSeconds: TimeInterval = 60.0
private let syntheticEventMarker: Int64 = 0x5459464c4f57

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

private enum TypeflowStartupError: Error, CustomStringConvertible {
    case eventTapCreationFailed
    case eventTapRunLoopSourceFailed

    var description: String {
        switch self {
        case .eventTapCreationFailed:
            return "failed to create CGEvent tap; Accessibility/Input Monitoring permission is required"
        case .eventTapRunLoopSourceFailed:
            return "failed to create event tap run-loop source"
        }
    }
}

private func terminateForStartupFailure(_ error: Error) -> Never {
    let message = "Typeflow startup failed: \(String(describing: error))"
    logger.error("\(message, privacy: .public)")
    FileHandle.standardError.write(Data("\(message)\n".utf8))

    NSApplication.shared.setActivationPolicy(.regular)
    NSApplication.shared.activate(ignoringOtherApps: true)
    let alert = NSAlert()
    alert.alertStyle = .critical
    alert.messageText = "Typeflow Failed to Start"
    alert.informativeText = String(describing: error)
    alert.addButton(withTitle: "Quit")
    alert.runModal()

    exit(1)
}

private let eventTapCallback: CGEventTapCallBack = { _, type, event, userInfo in
    guard event.getIntegerValueField(.eventSourceUserData) != syntheticEventMarker else {
        return Unmanaged.passUnretained(event)
    }
    guard let userInfo else {
        return Unmanaged.passUnretained(event)
    }
    let agent = Unmanaged<TypeflowAgent>.fromOpaque(userInfo).takeUnretainedValue()
    agent.handleEvent(type: type, event: event)
    return Unmanaged.passUnretained(event)
}

private let accessibilityObserverCallback: AXObserverCallback = { _, _, notification, userInfo in
    guard let userInfo else {
        return
    }
    let agent = Unmanaged<TypeflowAgent>.fromOpaque(userInfo).takeUnretainedValue()
    agent.handleAccessibilityNotification(notification as String)
}

private let inputSourceChangedCallback: CFNotificationCallback = { _, observer, _, _, _ in
    guard let observer else {
        return
    }
    let agent = Unmanaged<TypeflowAgent>.fromOpaque(observer).takeUnretainedValue()
    // The distributed notification may arrive on a non-main thread; engine and
    // syncedInputSourceLayout mutation must stay on main.
    DispatchQueue.main.async {
        agent.handleInputSourceChangedNotification()
    }
}

private final class TypeflowAgent: NSObject {
    private let hostConfig: TypeflowHostConfig
    private let engine: TypeflowEngine
    private let sourceSwitcher: InputSourceSwitcher
    private let textReplacer = TextReplacer()
    private let hostRefreshQueue = DispatchQueue(label: "io.github.nnnickg.typeflow.host-refresh", qos: .utility)
    private var eventTap: CFMachPort?
    private var runLoopSource: CFRunLoopSource?
    private var hostPolicyLogKey = ""
    private var cachedHostContextSnapshot: HostContextSnapshot?
    private var currentFrontmostApplication: NSRunningApplication?
    private var accessibilityObserver: AXObserver?
    private var accessibilityObservedApplication: AXUIElement?
    private var accessibilityRefreshWorkItem: DispatchWorkItem?
    private var accessibilityGeneration = 0
    private var accessibilitySlowRefreshCount = 0
    private var accessibilityDegradedPID: pid_t = 0
    private var accessibilityDegradedUntil: TimeInterval = 0
    private var accessibilityTrustLogged = false
    private var pendingOptionManualSwitch = false
    private var manualSwitchCancelled = false
    private var syncedInputSourceLayout: TypeflowLayout?
    private var currentReplacementFocus: ReplacementFocus?
    private var inputSourceStateReady = false
    private var inputSourceObserverRegistered = false
    private var inputSourceSelectionGeneration: UInt64 = 0
    private var pendingInputSourceSelectionWorkItem: DispatchWorkItem?
    private var replacementGeneration: UInt64 = 0
    private var manualReplacementToggle: ManualReplacementToggle?

    init(hostConfig: TypeflowHostConfig, engine: TypeflowEngine) {
        self.hostConfig = hostConfig
        self.engine = engine
        sourceSwitcher = InputSourceSwitcher(config: hostConfig)
        super.init()

        logger.notice(
            "initialized agent source=\(engine.sourceDescription, privacy: .public) config=\(hostConfig.sourcePath == nil ? "defaults" : "loaded", privacy: .public) disabledBundles=\(hostConfig.disabledBundleIDCount, privacy: .public) autoDisabledBundles=\(hostConfig.autoDisabledBundleIDCount, privacy: .public) manualSwitch=option"
        )
    }

    func start() throws {
        configureLaunchAtLogin()
        promptForAccessibilityTrustIfNeeded()
        currentFrontmostApplication = NSWorkspace.shared.frontmostApplication
        syncLayoutWithCurrentInputSource()
        registerInputSourceChangedObserver()
        let baseContext = installBaseHostContext(reason: "start")
        configureAccessibilityObserver(
            for: currentFrontmostApplication,
            basePolicy: baseContext.policy,
            reason: "start"
        )
        requestAccessibilityRefresh(reason: "start", force: true)

        NSWorkspace.shared.notificationCenter.addObserver(
            self,
            selector: #selector(frontmostApplicationChanged(_:)),
            name: NSWorkspace.didActivateApplicationNotification,
            object: nil
        )

        let eventMask = (1 << CGEventType.keyDown.rawValue)
            | (1 << CGEventType.flagsChanged.rawValue)
            | (1 << CGEventType.leftMouseDown.rawValue)
            | (1 << CGEventType.rightMouseDown.rawValue)
            | (1 << CGEventType.otherMouseDown.rawValue)
        guard let tap = CGEvent.tapCreate(
            tap: .cgSessionEventTap,
            place: .tailAppendEventTap,
            options: .listenOnly,
            eventsOfInterest: CGEventMask(eventMask),
            callback: eventTapCallback,
            userInfo: Unmanaged.passUnretained(self).toOpaque()
        ) else {
            throw TypeflowStartupError.eventTapCreationFailed
        }

        eventTap = tap
        guard let source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0) else {
            throw TypeflowStartupError.eventTapRunLoopSourceFailed
        }
        runLoopSource = source
        CFRunLoopAddSource(CFRunLoopGetMain(), source, .commonModes)
        CGEvent.tapEnable(tap: tap, enable: true)
        logger.notice("started event-tap observer")
    }

    private func configureLaunchAtLogin() {
        let bundleURL = Bundle.main.bundleURL.standardizedFileURL
        guard bundleURL.pathExtension == "app",
              isInstalledApplicationBundle(bundleURL)
        else {
            logger.debug("login item registration skipped bundle=\(bundleURL.path, privacy: .public)")
            return
        }

        let service = SMAppService.mainApp
        switch service.status {
        case .enabled:
            logger.debug("login item already enabled")
        case .requiresApproval:
            logger.notice("login item requires user approval in System Settings")
        case .notRegistered, .notFound:
            do {
                try service.register()
                logger.notice("login item registered bundle=\(bundleURL.path, privacy: .public)")
            } catch {
                logger.error("login item registration failed: \(String(describing: error), privacy: .public)")
            }
        @unknown default:
            logger.error("login item status unknown")
        }
    }

    private func isInstalledApplicationBundle(_ bundleURL: URL) -> Bool {
        let path = bundleURL.path
        return path.hasPrefix("/Applications/")
            || path.hasPrefix("\(NSHomeDirectory())/Applications/")
    }

    func handleEvent(type: CGEventType, event: CGEvent) {
        switch type {
        case .tapDisabledByTimeout, .tapDisabledByUserInput:
            if let eventTap {
                CGEvent.tapEnable(tap: eventTap, enable: true)
                logger.notice("re-enabled event tap")
            }
        case .keyDown:
            processKey(event)
        case .flagsChanged:
            processFlagsChanged(event)
        case .leftMouseDown, .rightMouseDown, .otherMouseDown:
            handlePotentialFocusChange(reason: "mouseDown")
        default:
            break
        }
    }

    @objc private func frontmostApplicationChanged(_ notification: Notification) {
        resetPendingManualSwitch()
        resetCachedHostState()
        engine.resetToken()
        currentFrontmostApplication = notification.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication
            ?? NSWorkspace.shared.frontmostApplication
        let baseContext = installBaseHostContext(reason: "appChanged")
        configureAccessibilityObserver(
            for: currentFrontmostApplication,
            basePolicy: baseContext.policy,
            reason: "appChanged"
        )
        requestAccessibilityRefresh(reason: "appChanged", force: true)
        logger.debug("frontmost application changed")
    }

    fileprivate func handleAccessibilityNotification(_ notificationName: String) {
        invalidateHostContext(reason: notificationName)
        requestAccessibilityRefresh(reason: notificationName, force: false)
    }

    private func processKey(_ event: CGEvent) {
        let processStarted = ProcessInfo.processInfo.systemUptime
        defer {
            logPerformance(name: "processKey", started: processStarted, thresholdMs: slowProcessThresholdMs)
        }

        cancelPendingReplacement(reason: "keyDown")
        cancelPendingManualSwitch()

        let keyCode = UInt16(event.getIntegerValueField(.keyboardEventKeycode))
        let flags = event.flags
        let modifiers = ffiModifiers(from: flags, keyCode: keyCode)

        let hostContext = currentHostContext(engine: engine, reason: "keyDown")
        if hostContext.automaticProcessingDisabled {
            engine.resetToken()
            return
        }
        guard inputSourceStateReady else {
            engine.resetToken()
            return
        }

        if shouldBypassHost(flags) {
            observeHostBypass(engine: engine, modifiers: modifiers)
            return
        }

        do {
            switch Int(keyCode) {
            case kVK_Return, kVK_Tab, kVK_Escape, kVK_Space:
                let action = try measured("ffi.endToken", thresholdMs: slowCallThresholdMs) {
                    try engine.endToken()
                }
                applyObservation(action, engine: engine, source: "auto")
                if Int(keyCode) == kVK_Tab || Int(keyCode) == kVK_Escape {
                    handlePotentialFocusChange(reason: "focusKey")
                }
                return
            case kVK_Delete:
                let action = try measured("ffi.observeBackspace", thresholdMs: slowCallThresholdMs) {
                    try engine.observeBackspace()
                }
                applyObservation(action, engine: engine, source: "auto")
                return
            default:
                break
            }

            if shouldBypassNonTextKey(keyCode: keyCode) {
                observeHostBypass(engine: engine, modifiers: modifiers)
                handlePotentialFocusChange(reason: "navigationKey")
                return
            }

            if let physical = TypeflowMacKeyCode.physicalKeyIndex(for: keyCode) {
                let action = try measured("ffi.observe", thresholdMs: slowCallThresholdMs) {
                    try engine.observe(physicalKey: physical, modifiers: modifiers)
                }
                applyObservation(action, engine: engine, source: "auto")
                return
            }

            let action = try measured("ffi.endToken", thresholdMs: slowCallThresholdMs) {
                try engine.endToken()
            }
            applyObservation(action, engine: engine, source: "auto")
        } catch {
            engine.resetToken()
            logger.error("engine observe failed: \(String(describing: error), privacy: .public)")
        }
    }

    private func processFlagsChanged(_ event: CGEvent) {
        let processStarted = ProcessInfo.processInfo.systemUptime
        defer {
            logPerformance(
                name: "processFlagsChanged",
                started: processStarted,
                thresholdMs: slowProcessThresholdMs
            )
        }

        cancelPendingReplacement(reason: "flagsChanged", clearsManualToggle: false)

        let keyCode = Int(event.getIntegerValueField(.keyboardEventKeycode))
        let isOptionKey = keyCode == kVK_Option || keyCode == kVK_RightOption
        guard isOptionKey else {
            cancelPendingManualSwitch()
            return
        }

        let optionDown = event.flags.contains(.maskAlternate)
        let nonOptionModifiers: CGEventFlags = event.flags
            .intersection([.maskShift, .maskControl, .maskCommand])

        if optionDown {
            // Option pressed while another modifier is held (e.g. Shift+Option
            // for special-character entry): treat as a chord, never as a
            // manual switch arm. Without this, a Shift-down followed by an
            // Option-tap would commit a layout flip on Option release.
            if !nonOptionModifiers.isEmpty {
                resetPendingManualSwitch()
                return
            }
            pendingOptionManualSwitch = true
            manualSwitchCancelled = false
            return
        }

        guard pendingOptionManualSwitch else {
            return
        }

        // Belt-and-suspenders: if another modifier is still held at Option
        // release, the user is still inside a chord — don't commit.
        let shouldSwitch = !manualSwitchCancelled && nonOptionModifiers.isEmpty
        resetPendingManualSwitch()
        guard shouldSwitch else {
            return
        }

        let hostContext = currentHostContext(engine: engine, reason: "flagsChanged")
        guard !hostContext.manualSwitchDisabled else {
            return
        }
        guard inputSourceStateReady else {
            engine.resetToken()
            return
        }

        do {
            let action = try measured("ffi.forceSwitchLayout", thresholdMs: slowCallThresholdMs) {
                try engine.forceSwitchLayout()
            }
            applyObservation(
                action,
                engine: engine,
                source: "manual",
                replacementDelaySeconds: 0
            )
        } catch {
            engine.resetToken()
            logger.error("manual layout switch failed: \(String(describing: error), privacy: .public)")
        }
    }

    private struct ReplacementPlan {
        let deleteCount: Int
        let text: String
        let inverseText: String?
        let delaySeconds: TimeInterval
        let focus: ReplacementFocus
    }

    private struct ManualReplacementToggle {
        let focus: ReplacementFocus
        let englishText: String
        let secondaryText: String
        let currentLayout: TypeflowLayout

        func replacementPlan(
            to targetLayout: TypeflowLayout,
            delaySeconds: TimeInterval
        ) -> ReplacementPlan? {
            let currentText = text(for: currentLayout)
            let targetText = text(for: targetLayout)
            guard !currentText.isEmpty, !targetText.isEmpty else {
                return nil
            }
            return ReplacementPlan(
                deleteCount: currentText.count,
                text: targetText,
                inverseText: currentText,
                delaySeconds: delaySeconds,
                focus: focus
            )
        }

        private func text(for layout: TypeflowLayout) -> String {
            switch layout {
            case .english:
                return englishText
            case .secondary:
                return secondaryText
            }
        }
    }

    private func applyObservation(
        _ action: TypeflowObservationAction,
        engine: TypeflowEngine,
        source: String,
        replacementDelaySeconds: TimeInterval = 0.012
    ) {
        let started = ProcessInfo.processInfo.systemUptime
        defer {
            logPerformance(
                name: "observation.apply.\(source)",
                started: started,
                thresholdMs: slowCallThresholdMs
            )
        }

        switch action {
        case .none, .resetToken:
            return
        case let .switchFutureLayout(layout):
            var replacementPlan = measured(
                "\(source).replacementPlan",
                thresholdMs: slowCallThresholdMs
            ) {
                self.replacementPlan(
                    from: engine.takePendingReplacement(),
                    delaySeconds: replacementDelaySeconds
                )
            }
            if source == "manual", replacementPlan == nil {
                replacementPlan = manualToggleReplacementPlan(
                    to: layout,
                    delaySeconds: replacementDelaySeconds
                )
            }
            measured("engine.resetToken.\(source)", thresholdMs: slowCallThresholdMs) {
                engine.resetToken()
            }
            scheduleInputSourceSelection(
                layout,
                source: source,
                replacementPlan: replacementPlan,
                replacementGeneration: replacementGeneration
            )
        }
    }

    private func scheduleInputSourceSelection(
        _ layout: TypeflowLayout,
        source: String,
        replacementPlan: ReplacementPlan?,
        replacementGeneration capturedReplacementGeneration: UInt64
    ) {
        pendingInputSourceSelectionWorkItem?.cancel()
        inputSourceSelectionGeneration &+= 1
        let selectionGeneration = inputSourceSelectionGeneration
        let workItem = DispatchWorkItem { [weak self] in
            guard let self,
                  selectionGeneration == self.inputSourceSelectionGeneration
            else {
                return
            }

            self.pendingInputSourceSelectionWorkItem = nil
            self.completeInputSourceSelection(
                layout,
                source: source,
                replacementPlan: replacementPlan,
                replacementGeneration: capturedReplacementGeneration
            )
        }
        pendingInputSourceSelectionWorkItem = workItem
        DispatchQueue.main.async(execute: workItem)
    }

    private func completeInputSourceSelection(
        _ layout: TypeflowLayout,
        source: String,
        replacementPlan: ReplacementPlan?,
        replacementGeneration capturedReplacementGeneration: UInt64
    ) {
        let selected = measured("inputSource.selectForFuture.\(source)", thresholdMs: slowCallThresholdMs) {
            sourceSwitcher.selectForFuture(layout, reason: source)
        }
        guard selected else {
            syncedInputSourceLayout = nil
            _ = syncLayoutWithCurrentInputSource()
            cancelPendingReplacement(reason: "inputSourceSelectionFailed")
            return
        }
        syncedInputSourceLayout = layout
        inputSourceStateReady = true
        guard capturedReplacementGeneration == replacementGeneration else {
            return
        }
        if let replacementPlan {
            if source == "manual" {
                updateManualReplacementToggle(
                    from: replacementPlan,
                    targetLayout: layout
                )
            } else {
                manualReplacementToggle = nil
            }
            textReplacer.replaceLastToken(
                reason: source,
                deleteCount: replacementPlan.deleteCount,
                with: replacementPlan.text,
                delaySeconds: replacementPlan.delaySeconds,
                isStillValid: { [weak self] in
                    self?.replacementFocusIsStillValid(replacementPlan.focus) ?? false
                }
            )
        }
    }

    private func replacementPlan(
        from replacement: TypeflowReplacement?,
        delaySeconds: TimeInterval
    ) -> ReplacementPlan? {
        guard let focus = currentReplacementFocus else {
            return nil
        }
        guard let replacement else {
            return nil
        }
        return ReplacementPlan(
            deleteCount: replacement.deleteCount,
            text: replacement.text,
            inverseText: replacement.inverseText,
            delaySeconds: delaySeconds,
            focus: focus
        )
    }

    private func manualToggleReplacementPlan(
        to targetLayout: TypeflowLayout,
        delaySeconds: TimeInterval
    ) -> ReplacementPlan? {
        guard let manualReplacementToggle else {
            return nil
        }
        guard replacementFocusIsStillValid(manualReplacementToggle.focus) else {
            self.manualReplacementToggle = nil
            return nil
        }
        return manualReplacementToggle.replacementPlan(
            to: targetLayout,
            delaySeconds: delaySeconds
        )
    }

    private func updateManualReplacementToggle(
        from replacementPlan: ReplacementPlan,
        targetLayout: TypeflowLayout
    ) {
        guard let inverseText = replacementPlan.inverseText,
              !inverseText.isEmpty
        else {
            manualReplacementToggle = nil
            return
        }

        let englishText: String
        let secondaryText: String
        switch targetLayout {
        case .english:
            englishText = replacementPlan.text
            secondaryText = inverseText
        case .secondary:
            englishText = inverseText
            secondaryText = replacementPlan.text
        }
        manualReplacementToggle = ManualReplacementToggle(
            focus: replacementPlan.focus,
            englishText: englishText,
            secondaryText: secondaryText,
            currentLayout: targetLayout
        )
    }

    private func replacementFocusIsStillValid(_ focus: ReplacementFocus) -> Bool {
        guard currentReplacementFocus == focus else {
            return false
        }
        return !cachedHostContext().automaticProcessingDisabled
    }

    private func observeHostBypass(engine: TypeflowEngine, modifiers: UInt8) {
        _ = try? measured("ffi.observeHostBypass", thresholdMs: slowCallThresholdMs) {
            try engine.observeHostBypass(modifiers: modifiers)
        }
    }

    @discardableResult
    private func syncLayoutWithCurrentInputSource() -> Bool {
        guard let layout = sourceSwitcher.currentLayout() else {
            inputSourceStateReady = false
            syncedInputSourceLayout = nil
            engine.resetToken()
            logger.debug("current input source is unresolved; key processing is bypassed")
            return false
        }
        inputSourceStateReady = true
        guard layout != syncedInputSourceLayout else {
            return true
        }
        syncedInputSourceLayout = layout
        engine.resetLayout(layout)
        return true
    }

    fileprivate func handleInputSourceChangedNotification() {
        syncLayoutWithCurrentInputSource()
    }

    private func handlePotentialFocusChange(reason: String) {
        resetPendingManualSwitch()
        guard accessibilityObserver != nil else {
            return
        }
        invalidateHostContext(reason: reason)
        requestAccessibilityRefresh(reason: reason, force: false)
    }

    private func registerInputSourceChangedObserver() {
        guard !inputSourceObserverRegistered else {
            return
        }
        let center = CFNotificationCenterGetDistributedCenter()
        let observer = Unmanaged.passUnretained(self).toOpaque()
        CFNotificationCenterAddObserver(
            center,
            observer,
            inputSourceChangedCallback,
            kTISNotifySelectedKeyboardInputSourceChanged,
            nil,
            .deliverImmediately
        )
        inputSourceObserverRegistered = true
        logger.debug("subscribed to TIS selected keyboard input source notification")
    }

    private func unregisterInputSourceChangedObserver() {
        guard inputSourceObserverRegistered else {
            return
        }
        let center = CFNotificationCenterGetDistributedCenter()
        let observer = Unmanaged.passUnretained(self).toOpaque()
        CFNotificationCenterRemoveEveryObserver(center, observer)
        inputSourceObserverRegistered = false
    }

    deinit {
        unregisterInputSourceChangedObserver()
    }

    private struct HostContextSnapshot {
        let secureInput: Bool
        let automaticProcessingDisabled: Bool
        let manualSwitchDisabled: Bool

        static let unknown = HostContextSnapshot(
            secureInput: false,
            automaticProcessingDisabled: true,
            manualSwitchDisabled: true
        )
    }

    private struct ResolvedHostContext {
        let snapshot: HostContextSnapshot
        let engineFlags: UInt32
        let policy: TypeflowHostInputPolicy
        let facts: TypeflowHostSurfaceFacts
        let pid: pid_t
        let secureInput: Bool
    }

    private struct ReplacementFocus: Equatable {
        let pid: pid_t
        let bundleID: String?
        let focusedElementRole: String?
        let focusedElementSubrole: String?
        let focusedElementIdentifier: String?
        let focusedWindowTitle: String?

        init(_ resolved: ResolvedHostContext) {
            pid = resolved.pid
            bundleID = resolved.facts.bundleID
            focusedElementRole = resolved.facts.focusedElementRole
            focusedElementSubrole = resolved.facts.focusedElementSubrole
            focusedElementIdentifier = resolved.facts.focusedElementIdentifier
            focusedWindowTitle = resolved.facts.focusedWindowTitle
        }
    }

    private func cachedHostContext() -> HostContextSnapshot {
        if let cachedHostContextSnapshot {
            return cachedHostContextSnapshot
        }

        return .unknown
    }

    private func currentHostContext(engine: TypeflowEngine, reason: String) -> HostContextSnapshot {
        let secureInput = measured("secureInput.keyPath", thresholdMs: slowCallThresholdMs) {
            IsSecureEventInputEnabled()
        }
        if secureInput {
            engine.resetToken()
            return .unknown
        }

        if let cachedHostContextSnapshot,
           cachedHostContextSnapshot.secureInput
        {
            if accessibilityObserver != nil {
                invalidateHostContext(reason: "\(reason).secureInputCleared")
                requestAccessibilityRefresh(reason: "\(reason).secureInputCleared", force: true)
                return .unknown
            }
            return installBaseHostContext(reason: "\(reason).secureInputCleared").snapshot
        }

        return cachedHostContext()
    }

    @discardableResult
    private func installBaseHostContext(reason: String) -> ResolvedHostContext {
        let resolved = resolveHostContext(
            app: currentFrontmostApplication,
            hostConfig: hostConfig,
            accessibility: .empty,
            metricPrefix: "hostBaseFacts"
        )
        installHostContext(resolved, reason: reason)
        return resolved
    }

    private func requestAccessibilityRefresh(reason: String, force: Bool) {
        guard let app = currentFrontmostApplication,
              accessibilityObserver != nil,
              shouldUseAccessibility(for: app)
        else {
            return
        }

        accessibilityRefreshWorkItem?.cancel()
        let generation = accessibilityGeneration
        let hostConfig = hostConfig
        let delay = force ? 0 : accessibilityRefreshDebounceSeconds
        let workItem = DispatchWorkItem { [weak self] in
            guard let self else {
                return
            }

            self.hostRefreshQueue.async { [weak self] in
                guard let self else {
                    return
                }

                let started = ProcessInfo.processInfo.systemUptime
                let secureInput = measured("secureInput", thresholdMs: slowCallThresholdMs) {
                    IsSecureEventInputEnabled()
                }
                let accessibility = measured(
                    "accessibilitySnapshot.refresh",
                    thresholdMs: slowHostThresholdMs
                ) {
                    self.accessibilitySnapshot(for: app)
                }
                let snapshotMs = (ProcessInfo.processInfo.systemUptime - started) * 1000.0
                let resolved = self.resolveHostContext(
                    app: app,
                    hostConfig: hostConfig,
                    secureInput: secureInput,
                    accessibility: accessibility,
                    metricPrefix: "hostSurfaceFacts"
                )

                DispatchQueue.main.async { [weak self] in
                    guard let self,
                          generation == self.accessibilityGeneration,
                          app.processIdentifier == self.currentFrontmostApplication?.processIdentifier
                    else {
                        return
                    }

                    let degraded = self.recordAccessibilityRefresh(
                        durationMs: snapshotMs,
                        resolved: resolved,
                        app: app
                    )
                    guard !degraded else {
                        return
                    }
                    self.installHostContext(resolved, reason: reason)
                }
            }
        }

        accessibilityRefreshWorkItem = workItem
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: workItem)
    }

    private func resolveHostContext(
        app: NSRunningApplication?,
        hostConfig: TypeflowHostConfig,
        accessibility: AccessibilitySnapshot,
        metricPrefix: String
    ) -> ResolvedHostContext {
        let secureInput = measured("secureInput", thresholdMs: slowCallThresholdMs) {
            IsSecureEventInputEnabled()
        }
        return resolveHostContext(
            app: app,
            hostConfig: hostConfig,
            secureInput: secureInput,
            accessibility: accessibility,
            metricPrefix: metricPrefix
        )
    }

    private func resolveHostContext(
        app: NSRunningApplication?,
        hostConfig: TypeflowHostConfig,
        secureInput: Bool,
        accessibility: AccessibilitySnapshot,
        metricPrefix: String
    ) -> ResolvedHostContext {
        let facts = measured("\(metricPrefix).refresh", thresholdMs: slowHostThresholdMs) {
            TypeflowHostSurfaceFacts(
                secureInput: secureInput,
                bundleID: app?.bundleIdentifier,
                applicationName: app?.localizedName,
                inputClientClass: nil,
                focusedElementRole: accessibility.focusedElementRole,
                focusedElementSubrole: accessibility.focusedElementSubrole,
                focusedElementRoleDescription: accessibility.focusedElementRoleDescription,
                focusedElementIdentifier: accessibility.focusedElementIdentifier,
                focusedElementDescription: accessibility.focusedElementDescription,
                focusedWindowTitle: accessibility.focusedWindowTitle
            )
        }
        let policy = measured("hostPolicy.resolve", thresholdMs: slowCallThresholdMs) {
            hostConfig.resolveInputPolicy(facts: facts)
        }

        var flags: UInt32 = 0
        if policy.secureInput {
            flags |= typeflow_ffi_context_secure_input()
        }
        if policy.automaticProcessingDisabled && policy.manualSwitchDisabled {
            flags |= typeflow_ffi_context_automatic_processing_disabled()
        } else if policy.automaticProcessingDisabled {
            flags |= typeflow_ffi_context_automatic_switching_disabled()
        }

        return ResolvedHostContext(
            snapshot: HostContextSnapshot(
                secureInput: policy.secureInput,
                automaticProcessingDisabled: policy.automaticProcessingDisabled && policy.manualSwitchDisabled,
                manualSwitchDisabled: policy.manualSwitchDisabled
            ),
            engineFlags: flags,
            policy: policy,
            facts: facts,
            pid: app?.processIdentifier ?? 0,
            secureInput: secureInput
        )
    }

    private func configureAccessibilityObserver(
        for app: NSRunningApplication?,
        basePolicy: TypeflowHostInputPolicy,
        reason: String
    ) {
        teardownAccessibilityObserver()

        guard let app,
              shouldRefineWithAccessibility(basePolicy),
              shouldUseAccessibility(for: app)
        else {
            return
        }
        guard AXIsProcessTrusted() else {
            if !accessibilityTrustLogged {
                logger.debug("accessibility not trusted; embedded terminal detection unavailable")
                accessibilityTrustLogged = true
            }
            return
        }

        var observer: AXObserver?
        let observerStatus = AXObserverCreate(
            app.processIdentifier,
            accessibilityObserverCallback,
            &observer
        )
        guard observerStatus == .success,
              let observer
        else {
            logger.debug(
                "AXObserverCreate failed status=\(observerStatus.rawValue, privacy: .public) reason=\(reason, privacy: .public)"
            )
            return
        }

        let appElement = AXUIElementCreateApplication(app.processIdentifier)
        _ = AXUIElementSetMessagingTimeout(appElement, 0.01)
        let refcon = Unmanaged.passUnretained(self).toOpaque()
        var registeredNotification = false
        for notification in [
            kAXFocusedUIElementChangedNotification,
            kAXFocusedWindowChangedNotification,
        ] {
            let status = AXObserverAddNotification(observer, appElement, notification as CFString, refcon)
            if status == .success {
                registeredNotification = true
            } else {
                logger.debug(
                    "AXObserverAddNotification failed status=\(status.rawValue, privacy: .public) notification=\(notification as String, privacy: .public)"
                )
            }
        }

        guard registeredNotification else {
            return
        }

        CFRunLoopAddSource(CFRunLoopGetMain(), AXObserverGetRunLoopSource(observer), .commonModes)
        accessibilityObserver = observer
        accessibilityObservedApplication = appElement
        logger.debug(
            "AX observer attached reason=\(reason, privacy: .public) pid=\(app.processIdentifier, privacy: .public)"
        )
    }

    private func teardownAccessibilityObserver() {
        accessibilityRefreshWorkItem?.cancel()
        accessibilityRefreshWorkItem = nil
        accessibilityGeneration += 1
        if let observer = accessibilityObserver {
            CFRunLoopRemoveSource(CFRunLoopGetMain(), AXObserverGetRunLoopSource(observer), .commonModes)
        }
        accessibilityObserver = nil
        accessibilityObservedApplication = nil
    }

    private func shouldRefineWithAccessibility(_ policy: TypeflowHostInputPolicy) -> Bool {
        !(policy.automaticProcessingDisabled && policy.manualSwitchDisabled)
    }

    private func shouldUseAccessibility(for app: NSRunningApplication) -> Bool {
        let now = ProcessInfo.processInfo.systemUptime
        return accessibilityDegradedPID != app.processIdentifier || now >= accessibilityDegradedUntil
    }

    private func recordAccessibilityRefresh(
        durationMs: Double,
        resolved: ResolvedHostContext,
        app: NSRunningApplication
    ) -> Bool {
        if durationMs >= accessibilitySlowRefreshThresholdMs && !resolved.policy.terminalSurface {
            accessibilitySlowRefreshCount += 1
        } else {
            accessibilitySlowRefreshCount = 0
        }

        guard accessibilitySlowRefreshCount >= accessibilitySlowRefreshLimit else {
            return false
        }

        accessibilitySlowRefreshCount = 0
        accessibilityDegradedPID = app.processIdentifier
        accessibilityDegradedUntil = ProcessInfo.processInfo.systemUptime + accessibilityBackoffSeconds
        logger.notice(
            "AX degraded pid=\(app.processIdentifier, privacy: .public) backoffSeconds=\(accessibilityBackoffSeconds, privacy: .public)"
        )

        teardownAccessibilityObserver()
        let recoveryGeneration = accessibilityGeneration
        let baseContext = installBaseHostContext(reason: "axDegraded")
        guard shouldRefineWithAccessibility(baseContext.policy) else {
            return true
        }

        DispatchQueue.main.asyncAfter(deadline: .now() + accessibilityBackoffSeconds) { [weak self, weak app] in
            guard let self,
                  let app,
                  recoveryGeneration == self.accessibilityGeneration,
                  app.processIdentifier == self.currentFrontmostApplication?.processIdentifier,
                  self.shouldUseAccessibility(for: app)
            else {
                return
            }

            let baseContext = self.installBaseHostContext(reason: "axBackoffExpired")
            self.configureAccessibilityObserver(
                for: app,
                basePolicy: baseContext.policy,
                reason: "axBackoffExpired"
            )
            self.requestAccessibilityRefresh(reason: "axBackoffExpired", force: true)
        }
        return true
    }

    private func installHostContext(_ resolved: ResolvedHostContext, reason: String) {
        measured("ffi.setHostContext", thresholdMs: slowCallThresholdMs) {
            engine.setHostContext(flags: resolved.engineFlags)
        }

        let logKey = [
            String(resolved.engineFlags),
            String(resolved.policy.flags),
            String(resolved.policy.reason),
            resolved.facts.bundleID ?? "",
            resolved.facts.focusedElementIdentifier ?? "",
            resolved.facts.focusedElementRole ?? "",
        ].joined(separator: "|")

        if logKey != hostPolicyLogKey {
            logger.debug(
                "hostPolicy reason=\(resolved.policy.reasonDescription, privacy: .public) refresh=\(reason, privacy: .public) secure=\(resolved.policy.secureInput, privacy: .private) autoDisabled=\(resolved.policy.automaticProcessingDisabled, privacy: .private) manualDisabled=\(resolved.policy.manualSwitchDisabled, privacy: .private) terminal=\(resolved.policy.terminalSurface, privacy: .private) bundleID=\(resolved.facts.bundleID ?? "unknown", privacy: .private) axRole=\(resolved.facts.focusedElementRole ?? "unknown", privacy: .private) axSubrole=\(resolved.facts.focusedElementSubrole ?? "unknown", privacy: .private) axID=\(resolved.facts.focusedElementIdentifier ?? "unknown", privacy: .private)"
            )
            hostPolicyLogKey = logKey
        }

        cachedHostContextSnapshot = resolved.snapshot
        currentReplacementFocus = ReplacementFocus(resolved)
    }

    private func promptForAccessibilityTrustIfNeeded() {
        guard !AXIsProcessTrusted() else {
            return
        }

        let options = [
            kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true
        ] as NSDictionary
        let trusted = AXIsProcessTrustedWithOptions(options)
        logger.notice("accessibility trust prompt requested trusted=\(trusted, privacy: .public)")
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

    private func cancelPendingManualSwitch() {
        if pendingOptionManualSwitch {
            manualSwitchCancelled = true
        }
    }

    private func resetPendingManualSwitch() {
        pendingOptionManualSwitch = false
        manualSwitchCancelled = false
    }

    private func resetCachedHostState() {
        hostPolicyLogKey = ""
        cachedHostContextSnapshot = .unknown
        currentReplacementFocus = nil
        cancelPendingReplacement(reason: "hostStateReset")
        cancelPendingInputSourceSelection(reason: "hostStateReset")
        accessibilitySlowRefreshCount = 0
        teardownAccessibilityObserver()
    }

    private func invalidateHostContext(reason: String) {
        cachedHostContextSnapshot = .unknown
        currentReplacementFocus = nil
        cancelPendingReplacement(reason: reason)
        cancelPendingInputSourceSelection(reason: reason)
        measured("ffi.setHostContext.invalidate", thresholdMs: slowCallThresholdMs) {
            engine.setHostContext(flags: typeflow_ffi_context_automatic_processing_disabled())
        }
        engine.resetToken()
        logger.debug("hostPolicy invalidated reason=\(reason, privacy: .public)")
    }

    private func cancelPendingReplacement(reason: String, clearsManualToggle: Bool = true) {
        replacementGeneration &+= 1
        if clearsManualToggle {
            manualReplacementToggle = nil
        }
        textReplacer.cancelPending(reason: reason)
    }

    private func cancelPendingInputSourceSelection(reason: String) {
        guard pendingInputSourceSelectionWorkItem != nil else {
            return
        }
        inputSourceSelectionGeneration &+= 1
        pendingInputSourceSelectionWorkItem?.cancel()
        pendingInputSourceSelectionWorkItem = nil
        logger.debug("pending input source selection cancelled reason=\(reason, privacy: .public)")
    }

    private func shouldBypassHost(_ flags: CGEventFlags) -> Bool {
        let bypass: CGEventFlags = [.maskControl, .maskAlternate, .maskCommand]
        return !flags.intersection(bypass).isEmpty
    }

    private func shouldBypassNonTextKey(keyCode: UInt16) -> Bool {
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
             kVK_Help,
             kVK_F1,
             kVK_F2,
             kVK_F3,
             kVK_F4,
             kVK_F5,
             kVK_F6,
             kVK_F7,
             kVK_F8,
             kVK_F9,
             kVK_F10,
             kVK_F11,
             kVK_F12,
             kVK_F13,
             kVK_F14,
             kVK_F15,
             kVK_F16,
             kVK_F17,
             kVK_F18,
             kVK_F19,
             kVK_F20:
            return true
        default:
            return false
        }
    }

    private func ffiModifiers(from flags: CGEventFlags, keyCode: UInt16? = nil) -> UInt8 {
        var modifiers: UInt8 = 0
        let shiftDown = flags.contains(.maskShift)
        let capsLockAffectsKey = keyCode.map { flags.contains(.maskAlphaShift) && isAnsiLetterKey($0) } ?? false
        if shiftDown != capsLockAffectsKey {
            modifiers |= UInt8(TF_MOD_SHIFT)
        }
        if flags.contains(.maskControl) {
            modifiers |= UInt8(TF_MOD_CONTROL)
        }
        if flags.contains(.maskAlternate) {
            modifiers |= UInt8(TF_MOD_OPTION)
        }
        if flags.contains(.maskCommand) {
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
}

private final class InputSourceSwitcher {
    private let logger = Logger(
        subsystem: "io.github.nnnickg.typeflow.agent",
        category: "InputSource"
    )
    private let englishInputSourceID: String?
    private let secondaryInputSourceID: String?

    init(config: TypeflowHostConfig) {
        let currentID = Self.currentInputSourceID()
        let englishID = config.macOSEnglishInputSourceID
            ?? Self.currentASCIICapableInputSourceID()
            ?? currentID
        let secondaryID = config.macOSSecondaryInputSourceID
            ?? Self.findInputSourceID(
                language: config.secondaryLanguage,
                excluding: englishID
            )

        englishInputSourceID = englishID
        secondaryInputSourceID = secondaryID

        logger.notice(
            "inputSources english=\(englishID ?? "unresolved", privacy: .public) secondary=\(secondaryID ?? "unresolved", privacy: .public)"
        )
    }

    func currentLayout() -> TypeflowLayout? {
        guard let currentID = Self.currentInputSourceID() else {
            return nil
        }
        if currentID == secondaryInputSourceID {
            return .secondary
        }
        if currentID == englishInputSourceID {
            return .english
        }
        return nil
    }

    func selectForFuture(_ layout: TypeflowLayout, reason: String) -> Bool {
        guard let targetID = inputSourceID(for: layout) else {
            logger.error("cannot switch to \(String(describing: layout), privacy: .public): input source unresolved")
            return false
        }

        let scheduledAt = ProcessInfo.processInfo.systemUptime
        defer {
            logPerformance(
                name: "inputSource.selectFuture.\(reason)",
                started: scheduledAt,
                thresholdMs: slowCallThresholdMs
            )
        }

        guard Self.currentInputSourceID() != targetID else {
            logPerformance(
                name: "inputSource.alreadySelected.\(reason)",
                started: scheduledAt,
                thresholdMs: slowCallThresholdMs
            )
            return true
        }
        let source = measured(
            "inputSource.find.\(reason)",
            thresholdMs: slowCallThresholdMs
        ) {
            Self.findInputSource(id: targetID)
        }
        guard let source else {
            logger.error("input source not found: \(targetID, privacy: .public)")
            return false
        }
        let enableStatus = measured(
            "inputSource.enable.\(reason)",
            thresholdMs: slowCallThresholdMs
        ) {
            TISEnableInputSource(source)
        }
        if enableStatus != noErr {
            logger.debug("TISEnableInputSource status=\(enableStatus, privacy: .public) id=\(targetID, privacy: .public)")
        }
        let selectStatus = measured(
            "inputSource.select.\(reason)",
            thresholdMs: slowCallThresholdMs
        ) {
            TISSelectInputSource(source)
        }
        if selectStatus == noErr {
            logger.notice("selected input source reason=\(reason, privacy: .public) id=\(targetID, privacy: .public)")
            return true
        } else {
            logger.error("TISSelectInputSource failed status=\(selectStatus, privacy: .public) id=\(targetID, privacy: .public)")
            return false
        }
    }

    private func inputSourceID(for layout: TypeflowLayout) -> String? {
        switch layout {
        case .english:
            return englishInputSourceID
        case .secondary:
            return secondaryInputSourceID
        }
    }

    private static func currentInputSourceID() -> String? {
        guard let source = TISCopyCurrentKeyboardInputSource()?.takeRetainedValue() else {
            return nil
        }
        return stringProperty(source, kTISPropertyInputSourceID)
    }

    private static func currentASCIICapableInputSourceID() -> String? {
        guard let source = TISCopyCurrentASCIICapableKeyboardLayoutInputSource()?.takeRetainedValue() else {
            return nil
        }
        return stringProperty(source, kTISPropertyInputSourceID)
    }

    private static func findInputSource(id targetID: String) -> TISInputSource? {
        inputSources(includeAllInstalled: true).first { source in
            stringProperty(source, kTISPropertyInputSourceID) == targetID
        }
    }

    private static func findInputSourceID(language: String, excluding excludedID: String?) -> String? {
        let normalizedLanguage = language.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !normalizedLanguage.isEmpty else {
            return nil
        }

        return inputSources(includeAllInstalled: true).first { source in
            guard isSelectableKeyboardSource(source),
                  let sourceID = stringProperty(source, kTISPropertyInputSourceID),
                  sourceID != excludedID,
                  !sourceID.localizedCaseInsensitiveContains("typeflow")
            else {
                return false
            }
            let languages = stringArrayProperty(source, kTISPropertyInputSourceLanguages)
            return languages.contains { language in
                let value = language.lowercased()
                return value == normalizedLanguage || value.hasPrefix("\(normalizedLanguage)-")
            }
        }.flatMap {
            stringProperty($0, kTISPropertyInputSourceID)
        }
    }

    private static func inputSources(includeAllInstalled: Bool) -> [TISInputSource] {
        TISCreateInputSourceList(nil, includeAllInstalled)?.takeRetainedValue() as? [TISInputSource] ?? []
    }

    private static func isSelectableKeyboardSource(_ source: TISInputSource) -> Bool {
        let category = stringProperty(source, kTISPropertyInputSourceCategory)
        guard category == (kTISCategoryKeyboardInputSource as String) else {
            return false
        }
        return boolProperty(source, kTISPropertyInputSourceIsSelectCapable) ?? false
    }

    private static func stringProperty(_ source: TISInputSource, _ key: CFString) -> String? {
        guard let value = TISGetInputSourceProperty(source, key) else {
            return nil
        }
        return Unmanaged<CFString>.fromOpaque(value).takeUnretainedValue() as String
    }

    private static func stringArrayProperty(_ source: TISInputSource, _ key: CFString) -> [String] {
        guard let value = TISGetInputSourceProperty(source, key) else {
            return []
        }
        return Unmanaged<CFArray>.fromOpaque(value).takeUnretainedValue() as? [String] ?? []
    }

    private static func boolProperty(_ source: TISInputSource, _ key: CFString) -> Bool? {
        guard let value = TISGetInputSourceProperty(source, key) else {
            return nil
        }
        return CFBooleanGetValue(Unmanaged<CFBoolean>.fromOpaque(value).takeUnretainedValue())
    }
}

private final class TextReplacer {
    private let logger = Logger(
        subsystem: "io.github.nnnickg.typeflow.agent",
        category: "Replacement"
    )
    private let source = CGEventSource(stateID: .hidSystemState)
    private var pendingWorkItem: DispatchWorkItem?
    private var generation: UInt64 = 0

    func cancelPending(reason: String) {
        guard pendingWorkItem != nil else {
            return
        }
        generation &+= 1
        pendingWorkItem?.cancel()
        pendingWorkItem = nil
        logger.debug("cancelled pending replacement reason=\(reason, privacy: .public)")
    }

    func replaceLastToken(
        reason: String,
        deleteCount: Int,
        with text: String,
        delaySeconds: TimeInterval,
        isStillValid: @escaping () -> Bool
    ) {
        guard deleteCount > 0, !text.isEmpty else {
            return
        }

        cancelPending(reason: "superseded")
        generation &+= 1
        let scheduledGeneration = generation
        let requestedAt = ProcessInfo.processInfo.systemUptime
        let workItem = DispatchWorkItem { [weak self, source, logger] in
            guard let self,
                  self.generation == scheduledGeneration
            else {
                return
            }
            guard isStillValid() else {
                self.cancelPending(reason: "validationFailed")
                return
            }

            let workStarted = ProcessInfo.processInfo.systemUptime
            measured("replacement.deleteLoop.\(reason)", thresholdMs: slowCallThresholdMs) {
                for _ in 0..<deleteCount {
                    Self.postKey(virtualKey: CGKeyCode(kVK_Delete), keyDown: true, source: source)
                    Self.postKey(virtualKey: CGKeyCode(kVK_Delete), keyDown: false, source: source)
                }
            }
            measured("replacement.postUnicode.\(reason)", thresholdMs: slowCallThresholdMs) {
                Self.postUnicode(text, source: source)
            }
            logPerformance(
                name: "replacement.work.\(reason)",
                started: workStarted,
                thresholdMs: slowCallThresholdMs
            )
            logPerformance(
                name: "replacement.decisionToPost.\(reason)",
                started: requestedAt,
                thresholdMs: slowCallThresholdMs
            )
            logger.notice(
                "replaced token reason=\(reason, privacy: .public) deleteCount=\(deleteCount, privacy: .public) insertedUtf16=\(text.utf16.count, privacy: .public)"
            )
            if self.generation == scheduledGeneration {
                self.pendingWorkItem = nil
            }
        }
        pendingWorkItem = workItem
        DispatchQueue.main.asyncAfter(deadline: .now() + delaySeconds, execute: workItem)
    }

    private static func postKey(
        virtualKey: CGKeyCode,
        keyDown: Bool,
        source: CGEventSource?
    ) {
        guard let event = CGEvent(
            keyboardEventSource: source,
            virtualKey: virtualKey,
            keyDown: keyDown
        ) else {
            return
        }
        event.setIntegerValueField(.eventSourceUserData, value: syntheticEventMarker)
        event.post(tap: .cghidEventTap)
    }

    private static func postUnicode(_ text: String, source: CGEventSource?) {
        let units = Array(text.utf16)
        guard !units.isEmpty,
              let keyDown = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: true),
              let keyUp = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: false)
        else {
            return
        }

        keyDown.setIntegerValueField(.eventSourceUserData, value: syntheticEventMarker)
        keyUp.setIntegerValueField(.eventSourceUserData, value: syntheticEventMarker)
        units.withUnsafeBufferPointer { buffer in
            guard let baseAddress = buffer.baseAddress else {
                return
            }
            keyDown.keyboardSetUnicodeString(
                stringLength: units.count,
                unicodeString: baseAddress
            )
            keyUp.keyboardSetUnicodeString(
                stringLength: units.count,
                unicodeString: baseAddress
            )
        }
        keyDown.post(tap: .cghidEventTap)
        keyUp.post(tap: .cghidEventTap)
    }
}

NSApplication.shared.setActivationPolicy(.accessory)

do {
    let hostConfig = try TypeflowHostConfig.load()
    let engine = try TypeflowEngine(hostConfig: hostConfig)
    let agent = TypeflowAgent(hostConfig: hostConfig, engine: engine)
    try agent.start()
    NSApplication.shared.run()
    _ = agent
} catch {
    terminateForStartupFailure(error)
}
