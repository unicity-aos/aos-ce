import Foundation

/// Display only: choices and their original response indices remain unchanged.
public struct ConsentSummary: Equatable, Sendable {
    public let title: String
    public let message: String
    public let resource: String?
    public let reason: String?
    public let originalMessage: String?

    public init(_ prompt: RuntimePromptRow) {
        guard let consent = prompt.consent,
              consent.isValid(optionCount: prompt.options.count),
              consent.kind == .actionApproval,
              let action = consent.action, let resource = consent.resource else {
            title = "Allow this request?"
            message = prompt.message
            resource = nil
            reason = nil
            originalMessage = nil
            return
        }
        title = "Allow this action?"
        message = action
        self.resource = resource
        reason = consent.reason
        originalMessage = prompt.message
    }
}
