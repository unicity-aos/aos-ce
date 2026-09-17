import Foundation

/// A runtime input request, distinct from an MCP approval-choice presentation.
/// Secret answers must go back over the local runtime connection, never MCP.
public struct NativeInputRequest: Decodable, Equatable, Sendable {
    public enum Kind: String, Decodable, Sendable { case text, secret, select, array }
    public let id: UUID
    public let principal: String
    public let capsule: String
    public let key: String
    public let prompt: String
    public let kind: Kind
    public let options: [String]?
    public let defaultValue: String?

    public static let maxValueBytes = 4_096
    public static let maxItems = 64

    public static func decode(_ bytes: Data) throws -> Self {
        guard bytes.count <= PresentationLimits.maxFrameBytes else { throw NativeInputError.invalidRequest }
        let request = try JSONDecoder().decode(Self.self, from: bytes)
        try request.validate()
        return request
    }

    public func validate() throws {
        for identity in [principal, capsule, key] {
            guard !identity.isEmpty, identity.utf8.count <= PresentationLimits.maxIDBytes,
                  !identity.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) })
            else { throw NativeInputError.invalidRequest }
        }
        guard !prompt.isEmpty, prompt.utf8.count <= PresentationLimits.maxMessageBytes else {
            throw NativeInputError.invalidRequest
        }
        if kind == .select {
            guard let options, !options.isEmpty, options.count <= Self.maxItems,
                  Set(options).count == options.count,
                  options.allSatisfy({ !$0.isEmpty && $0.utf8.count <= PresentationLimits.maxLabelBytes })
            else { throw NativeInputError.invalidRequest }
            if let defaultValue, !options.contains(defaultValue) { throw NativeInputError.invalidRequest }
        } else if options != nil { throw NativeInputError.invalidRequest }
        // Do not transmit a secret default to the GUI or put one in a preview.
        if kind == .secret || kind == .array {
            guard defaultValue == nil else { throw NativeInputError.invalidRequest }
        } else if let defaultValue, defaultValue.utf8.count > Self.maxValueBytes {
            throw NativeInputError.invalidRequest
        }
    }

    public func validateAnswer(_ answer: NativeInputAnswer) throws {
        try validate()
        switch (kind, answer) {
        case (_, .cancelled): return
        case (.text, .value(let text)), (.secret, .value(let text)), (.select, .value(let text)):
            guard text.utf8.count <= Self.maxValueBytes else { throw NativeInputError.invalidAnswer }
            if kind == .secret && text.isEmpty { throw NativeInputError.invalidAnswer }
            if kind == .select && options?.contains(text) != true { throw NativeInputError.invalidAnswer }
        case (.array, .values(let values)):
            guard values.count <= Self.maxItems,
                  values.reduce(0, { $0 + $1.utf8.count }) <= Self.maxValueBytes
            else { throw NativeInputError.invalidAnswer }
        default: throw NativeInputError.invalidAnswer
        }
    }
}

/// Transient human input. Deliberately not Codable: transport adapters must
/// explicitly route it, and must not attach it to snapshots or MCP messages.
public enum NativeInputAnswer: Equatable, Sendable, CustomStringConvertible, CustomDebugStringConvertible {
    case cancelled
    case value(String)
    case values([String])

    public var description: String {
        switch self {
        case .cancelled: "NativeInputAnswer.cancelled"
        case .value: "NativeInputAnswer.value(<redacted>)"
        case .values: "NativeInputAnswer.values(<redacted>)"
        }
    }
    public var debugDescription: String { description }
}

public enum NativeInputError: Error { case invalidRequest, invalidAnswer }
