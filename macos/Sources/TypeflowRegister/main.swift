import Carbon
import Foundation

let typeflowInputMethodID = "io.github.nnnickg.typeflow.inputmethod.Typeflow"
let typeflowModeID = "io.github.nnnickg.typeflow.inputmethod.Typeflow.Ukrainian"
let hitoolboxDomain = "com.apple.HIToolbox" as CFString
let enabledInputSourcesKey = "AppleEnabledInputSources" as CFString

func stringProperty(_ source: TISInputSource, _ key: CFString) -> String? {
    TISGetInputSourceProperty(source, key).map {
        Unmanaged<CFString>.fromOpaque($0).takeUnretainedValue() as String
    }
}

func findInputSource(id targetID: String) -> TISInputSource? {
    guard let sources = TISCreateInputSourceList(nil, true)?.takeRetainedValue() as? [TISInputSource] else {
        return nil
    }

    return sources.first { source in
        stringProperty(source, kTISPropertyInputSourceID) == targetID
    }
}

func inputSourceEntryExists(
    _ entries: [[String: Any]],
    kind: String,
    bundleID: String,
    inputMode: String? = nil
) -> Bool {
    entries.contains { entry in
        guard entry["InputSourceKind"] as? String == kind,
              entry["Bundle ID"] as? String == bundleID
        else {
            return false
        }

        return entry["Input Mode"] as? String == inputMode
    }
}

func ensureHIToolboxEnabledRecords() {
    var entries = CFPreferencesCopyAppValue(
        enabledInputSourcesKey,
        hitoolboxDomain
    ) as? [[String: Any]] ?? []

    if !inputSourceEntryExists(
        entries,
        kind: "Input Mode",
        bundleID: typeflowInputMethodID,
        inputMode: typeflowModeID
    ) {
        entries.append([
            "Bundle ID": typeflowInputMethodID,
            "Input Mode": typeflowModeID,
            "InputSourceKind": "Input Mode",
        ])
    }

    if !inputSourceEntryExists(
        entries,
        kind: "Keyboard Input Method",
        bundleID: typeflowInputMethodID
    ) {
        entries.append([
            "Bundle ID": typeflowInputMethodID,
            "InputSourceKind": "Keyboard Input Method",
        ])
    }

    CFPreferencesSetAppValue(
        enabledInputSourcesKey,
        entries as CFArray,
        hitoolboxDomain
    )

    guard CFPreferencesAppSynchronize(hitoolboxDomain) else {
        FileHandle.standardError.write(Data("failed to synchronize com.apple.HIToolbox preferences\n".utf8))
        exit(1)
    }
}

guard CommandLine.arguments.count == 2 else {
    FileHandle.standardError.write(Data("usage: typeflow-register-input-source <Typeflow.app>\n".utf8))
    exit(64)
}

let appURL = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true) as CFURL
let status = TISRegisterInputSource(appURL)
guard status == noErr else {
    FileHandle.standardError.write(Data("TISRegisterInputSource failed: \(status)\n".utf8))
    exit(1)
}

print("registered input source: \(CommandLine.arguments[1])")

guard let inputMethod = findInputSource(id: typeflowInputMethodID) else {
    FileHandle.standardError.write(Data("registered app, but input method was not visible to TIS: \(typeflowInputMethodID)\n".utf8))
    exit(1)
}

let inputMethodEnableStatus = TISEnableInputSource(inputMethod)
guard inputMethodEnableStatus == noErr else {
    FileHandle.standardError.write(Data("TISEnableInputSource failed for \(typeflowInputMethodID): \(inputMethodEnableStatus)\n".utf8))
    exit(1)
}

guard let mode = findInputSource(id: typeflowModeID) else {
    FileHandle.standardError.write(Data("registered app, but input mode was not visible to TIS: \(typeflowModeID)\n".utf8))
    exit(1)
}

let enableStatus = TISEnableInputSource(mode)
guard enableStatus == noErr else {
    FileHandle.standardError.write(Data("TISEnableInputSource failed for \(typeflowModeID): \(enableStatus)\n".utf8))
    exit(1)
}

print("enabled input method: \(typeflowInputMethodID)")
print("enabled input source: \(typeflowModeID)")

ensureHIToolboxEnabledRecords()
print("updated HIToolbox enabled input sources")
