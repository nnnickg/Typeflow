import Foundation
import TypeflowFFI

public struct TypeflowHostConfig {
    public var engine: TfEngineConfig
    public var secondaryLanguage: String
    public var packDirectory: String?
    public var dataDirectory: String?
    public var excludedBundleIDs: Set<String>
    public var sourcePath: String?

    public static func load() -> Self {
        load(environment: ProcessInfo.processInfo.environment)
    }

    static func load(environment: [String: String]) -> Self {
        var config = TypeflowHostConfig.defaults(environment: environment)
        guard let path = explicitConfigPath(environment: environment) ?? defaultConfigPath(environment: environment),
              FileManager.default.fileExists(atPath: path.path)
        else {
            config.applyEnvironmentOverrides(environment)
            return config
        }

        config.sourcePath = path.path
        guard let text = try? String(contentsOf: path, encoding: .utf8) else {
            config.applyEnvironmentOverrides(environment)
            return config
        }

        config.applyTomlSubset(text)
        config.applyEnvironmentOverrides(environment)
        return config
    }

    private static func defaults(environment: [String: String]) -> Self {
        TypeflowHostConfig(
            engine: TypeflowEngine.defaultConfig(),
            secondaryLanguage: "uk",
            packDirectory: defaultPackDirectory(environment: environment),
            dataDirectory: nil,
            excludedBundleIDs: [],
            sourcePath: nil
        )
    }

    private static func explicitConfigPath(environment: [String: String]) -> URL? {
        guard let path = environment["TYPEFLOW_CONFIG"], !path.isEmpty else {
            return nil
        }
        return URL(fileURLWithPath: NSString(string: path).expandingTildeInPath)
    }

    private static func defaultConfigPath(environment: [String: String]) -> URL? {
        guard let home = environment["HOME"], !home.isEmpty else {
            return nil
        }
        return URL(fileURLWithPath: home)
            .appendingPathComponent(".config")
            .appendingPathComponent("typeflow")
            .appendingPathComponent("config.toml")
    }

    private static func defaultPackDirectory(environment: [String: String]) -> String? {
        guard let home = environment["HOME"], !home.isEmpty else {
            return nil
        }
        return URL(fileURLWithPath: home)
            .appendingPathComponent("Library")
            .appendingPathComponent("Application Support")
            .appendingPathComponent("Typeflow")
            .appendingPathComponent("packs")
            .path
    }

    private mutating func applyEnvironmentOverrides(_ environment: [String: String]) {
        if let packDirectory = environment["TYPEFLOW_PACK_DIR"], !packDirectory.isEmpty {
            self.packDirectory = packDirectory
        }
        if let dataDirectory = environment["TYPEFLOW_DATA_DIR"], !dataDirectory.isEmpty {
            self.dataDirectory = dataDirectory
        }
    }

    public var engineSourceDescription: String {
        if let dataDirectory, !dataDirectory.isEmpty {
            return "data_dir"
        }
        if normalizedSecondaryLanguage == "uk" {
            return "embedded"
        }
        return "pack:\(normalizedSecondaryLanguage)"
    }

    public var normalizedSecondaryLanguage: String {
        let trimmed = secondaryLanguage.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? "uk" : trimmed
    }

    public var selectedPackPath: String? {
        guard let packDirectory, !packDirectory.isEmpty else {
            return nil
        }
        return URL(fileURLWithPath: NSString(string: packDirectory).expandingTildeInPath)
            .appendingPathComponent(normalizedSecondaryLanguage)
            .path
    }

    private mutating func applyTomlSubset(_ text: String) {
        var section = ""
        let lines = Array(text.components(separatedBy: .newlines))
        var index = 0

        while index < lines.count {
            let rawLine = stripComment(lines[index]).trimmingCharacters(in: .whitespacesAndNewlines)
            index += 1

            if rawLine.isEmpty {
                continue
            }

            if rawLine.hasPrefix("["), rawLine.hasSuffix("]") {
                section = String(rawLine.dropFirst().dropLast()).trimmingCharacters(in: .whitespacesAndNewlines)
                continue
            }

            guard let equals = rawLine.firstIndex(of: "=") else {
                continue
            }

            let key = rawLine[..<equals].trimmingCharacters(in: .whitespacesAndNewlines)
            var value = rawLine[rawLine.index(after: equals)...].trimmingCharacters(in: .whitespacesAndNewlines)
            if value == "[" || value.hasPrefix("[") && !value.contains("]") {
                var arrayLines = [String(value)]
                while index < lines.count {
                    let next = stripComment(lines[index]).trimmingCharacters(in: .whitespacesAndNewlines)
                    index += 1
                    arrayLines.append(next)
                    if next.contains("]") {
                        break
                    }
                }
                value = arrayLines.joined(separator: "\n")
            }

            applyValue(section: section, key: String(key), value: String(value))
        }
    }

    private mutating func applyValue(section: String, key: String, value: String) {
        switch (section, key) {
        case ("engine", "min_token_len"):
            if let parsed = parseUInt(value) {
                engine.min_token_len = parsed
            }
        case ("engine", "max_token_len"):
            if let parsed = parseUInt(value) {
                engine.max_token_len = parsed
            }
        case ("engine", "confidence_margin"):
            if let parsed = parseFloat(value) {
                engine.confidence_margin = parsed
            }
        case ("engine", "dict_exact_weight"):
            if let parsed = parseFloat(value) {
                engine.dict_exact_weight = parsed
            }
        case ("engine", "dict_prefix_weight"):
            if let parsed = parseFloat(value) {
                engine.dict_prefix_weight = parsed
            }
        case ("engine", "ngram_only_confidence_margin"):
            if let parsed = parseFloat(value) {
                engine.ngram_only_confidence_margin = parsed
            }
        case ("engine", "bigram_weight"):
            if let parsed = parseFloat(value) {
                engine.bigram_weight = parsed
            }
        case ("engine", "trigram_weight"):
            if let parsed = parseFloat(value) {
                engine.trigram_weight = parsed
            }
        case ("engine", "length_normalize"):
            if let parsed = parseBool(value) {
                engine.length_normalize = parsed ? 1 : 0
            }
        case ("engine", "disable_on_internal_caps"):
            if let parsed = parseBool(value) {
                engine.disable_on_internal_caps = parsed ? 1 : 0
            }
        case ("language", "secondary"):
            if let parsed = parseString(value) {
                secondaryLanguage = parsed
            }
        case ("packs", "directory"):
            if let parsed = parseString(value) {
                packDirectory = parsed
            }
        case ("data", "directory"):
            if let parsed = parseString(value) {
                dataDirectory = parsed
            }
        case ("apps", "exclude_bundle_ids"):
            excludedBundleIDs = Set(parseStringArray(value))
        default:
            break
        }
    }

    private func stripComment(_ line: String) -> String {
        var inString = false
        var escaped = false
        var output = ""

        for character in line {
            if escaped {
                output.append(character)
                escaped = false
                continue
            }
            if character == "\\" {
                output.append(character)
                escaped = true
                continue
            }
            if character == "\"" {
                inString.toggle()
                output.append(character)
                continue
            }
            if character == "#", !inString {
                break
            }
            output.append(character)
        }

        return output
    }

    private func parseString(_ value: String) -> String? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.hasPrefix("\""), trimmed.hasSuffix("\""), trimmed.count >= 2 else {
            return nil
        }
        let inner = trimmed.dropFirst().dropLast()
        return String(inner)
    }

    private func parseStringArray(_ value: String) -> [String] {
        var result: [String] = []
        var current = ""
        var inString = false
        var escaped = false

        for character in value {
            if escaped {
                current.append(character)
                escaped = false
                continue
            }
            if character == "\\" {
                if inString {
                    escaped = true
                }
                continue
            }
            if character == "\"" {
                if inString {
                    result.append(current)
                    current = ""
                }
                inString.toggle()
                continue
            }
            if inString {
                current.append(character)
            }
        }

        return result
    }

    private func parseUInt(_ value: String) -> Int? {
        Int(value.trimmingCharacters(in: .whitespacesAndNewlines))
    }

    private func parseFloat(_ value: String) -> Float? {
        Float(value.trimmingCharacters(in: .whitespacesAndNewlines))
    }

    private func parseBool(_ value: String) -> Bool? {
        switch value.trimmingCharacters(in: .whitespacesAndNewlines) {
        case "true":
            return true
        case "false":
            return false
        default:
            return nil
        }
    }
}
