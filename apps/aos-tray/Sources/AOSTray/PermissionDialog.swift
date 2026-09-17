import SwiftUI
import AOSTrayCore

/// AOS-owned presentation, not an operating-system authorization prompt.
struct PermissionDialog: View {
    @ObservedObject var session: TraySession
    var reviewLater: () -> Void

    var body: some View {
        if let prompt = session.runtimePrompts.first {
            PermissionContent(prompt: prompt, moreCount: session.runtimePrompts.count - 1,
                select: { session.selectRuntimePrompt(promptID: prompt.id, index: $0) },
                reviewLater: reviewLater).id(prompt.id)
        }
    }
}

private struct PermissionContent: View {
    let prompt: RuntimePromptRow
    let moreCount: Int
    let select: (Int) -> Void
    let reviewLater: () -> Void
    @State private var selected: Int? = nil
    @State private var details = false
    private var summary: ConsentSummary { ConsentSummary(prompt) }

    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: "hand.raised.fill")
                .font(.system(size: 26, weight: .medium))
                .foregroundStyle(.tint)
                .frame(width: 52, height: 52)
                .background(Color.accentColor.opacity(0.1), in: RoundedRectangle(cornerRadius: 12))
                .accessibilityHidden(true)
            Text(summary.title)
                .font(.headline)
            Group {
                ScrollView {
                VStack(spacing: 6) {
                    Text(summary.message)
                        .font(.subheadline)
                        .multilineTextAlignment(.center)
                        .frame(maxWidth: .infinity)
                        .textSelection(.enabled)
                    if let resource = summary.resource {
                        Text("Resource: \(resource)").font(.subheadline)
                            .textSelection(.enabled)
                    }
                    if let reason = summary.reason {
                        Text(reason).font(.caption).foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    }
                }
                .fixedSize(horizontal: false, vertical: true)
                }.frame(maxHeight: 160)
                    .fixedSize(horizontal: false, vertical: true)
                if let consent = prompt.consent {
                    DisclosureGroup("Details", isExpanded: $details) {
                        ConsentDetails(consent: consent, originalMessage: summary.originalMessage)
                            .padding(.top, 6)
                    }.font(.caption)
                }
                if let choices = ApprovalChoices(prompt) {
                    VStack(alignment: .leading, spacing: 6) {
                        Picker("Remember", selection: Binding(
                            get: { selected ?? choices.once }, set: { selected = $0 })) {
                            Text("Just this time").tag(choices.once)
                            Text("This session").tag(choices.session)
                            Text(choices.restartOnly ? "Until runtime restart" : "Always").tag(choices.remembered)
                        }.accessibilityIdentifier("approval-duration")
                        Text(explanation(choices)).font(.caption).foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    HStack(spacing: 10) {
                        Button("Deny") { select(choices.deny) }.frame(maxWidth: .infinity)
                        Button("Allow") { select(selected ?? choices.once) }
                            .buttonStyle(.borderedProminent).frame(maxWidth: .infinity)
                    }.controlSize(.large)
                } else {
                VStack(spacing: 8) {
                    ForEach(Array(prompt.options.enumerated()), id: \.offset) { index, label in
                        Button {
                            select(index)
                        } label: {
                            ConsentChoiceLabel(label: label, index: index, consent: prompt.consent)
                        }
                        .buttonStyle(.bordered)
                        .controlSize(.large)
                        .accessibilityIdentifier("dialog-option-\(index)")
                    }
                }
                }
            }
            HStack {
                if moreCount > 0 {
                    Text("\(moreCount) more waiting")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Button("Review later", action: reviewLater)
                    .buttonStyle(.link).font(.caption)
                    .keyboardShortcut(.cancelAction)
            }
        }
        .padding(24)
        .frame(width: 340)
        .fixedSize(horizontal: false, vertical: true)
    }
    private func explanation(_ choices: ApprovalChoices) -> String {
        switch selected ?? choices.once {
        case choices.session: "Remembered for this session."
        case choices.remembered:
            choices.restartOnly ? "Forgotten when the runtime restarts." : "Saved across runtime restarts for this approval scope."
        default: "Applies only to this request."
        }
    }
}
