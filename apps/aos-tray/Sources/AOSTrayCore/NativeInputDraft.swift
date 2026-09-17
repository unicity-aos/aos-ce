import Foundation

/// Transient editor state for `NativeInputDialog`. Not persisted, not Codable,
/// and never initialized from a secret default.
public struct NativeInputDraft: Equatable, CustomStringConvertible, CustomDebugStringConvertible {
    public var description: String { "NativeInputDraft(<redacted>)" }
    public var debugDescription: String { description }
    public struct Row: Identifiable, Equatable {
        public let id: UUID
        public var value: String
        public init(id: UUID = UUID(), value: String = "") {
            self.id = id
            self.value = value
        }
    }

    public var text: String
    public var secret: String
    public var selection: String
    public var rows: [Row]

    public init(request: NativeInputRequest) {
        switch request.kind {
        case .text:
            text = request.defaultValue ?? ""
            secret = ""
            selection = ""
            rows = []
        case .secret:
            text = ""
            secret = ""
            selection = ""
            rows = []
        case .select:
            text = ""
            secret = ""
            selection = request.defaultValue ?? request.options?.first ?? ""
            rows = []
        case .array:
            text = ""
            secret = ""
            selection = ""
            rows = [Row()]
        }
    }

    public mutating func clearSecrets() {
        secret = ""
    }

    public mutating func addRow() {
        guard rows.count < NativeInputRequest.maxItems else { return }
        rows.append(Row())
    }

    public mutating func removeRow(id: UUID) {
        rows.removeAll { $0.id == id }
    }

    public func answer(for request: NativeInputRequest) -> NativeInputAnswer {
        switch request.kind {
        case .text:
            return .value(text)
        case .secret:
            return .value(secret)
        case .select:
            return .value(selection)
        case .array:
            return .values(rows.map(\.value))
        }
    }

    public func canSubmit(_ request: NativeInputRequest) -> Bool {
        (try? request.validateAnswer(answer(for: request))) != nil
    }
}
