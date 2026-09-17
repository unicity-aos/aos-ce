import Foundation

/// Compact layout only for recognized action choices, retaining wire indices.
public struct ApprovalChoices: Equatable, Sendable {
    public let once: Int
    public let session: Int
    public let remembered: Int
    public let deny: Int
    public let restartOnly: Bool

    public init?(_ prompt: RuntimePromptRow) {
        guard let consent = prompt.consent, consent.kind == .actionApproval,
              prompt.options.count == 4, consent.lifetimes.count == 4 else { return nil }
        func unique(_ names: [String]) -> Int? {
            let matches = prompt.options.indices.filter { names.contains(prompt.options[$0]) }
            return matches.count == 1 ? matches[0] : nil
        }
        guard let once = unique(["Allow once", "Approve Once"]),
              let session = unique(["Allow for session", "Approve for Session"]),
              let remembered = unique(["Always allow", "Always Approve", "Until runtime restart"]),
              let deny = unique(["Deny"]),
              Set([once, session, remembered, deny]).count == 4,
              consent.lifetimes[once] == .none, consent.lifetimes[session] == .session,
              consent.lifetimes[deny] == .none,
              [.durable, .untilRuntimeRestart].contains(consent.lifetimes[remembered])
        else { return nil }
        self.once = once; self.session = session; self.remembered = remembered
        self.deny = deny
        self.restartOnly = consent.lifetimes[remembered] == .untilRuntimeRestart
    }
}
