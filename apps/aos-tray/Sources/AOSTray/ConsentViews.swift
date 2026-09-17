import SwiftUI
import AOSTrayCore

/// Shared by the popup and deferred-review panel so scope cannot disappear.
struct ConsentDetails: View {
    let consent: ConsentPresentation
    var originalMessage: String? = nil

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 6) {
                Text(consent.kind.title).font(.subheadline.weight(.semibold))
                detail("Principal", consent.principal)
                detail("Capsule", consent.capsule)
                detail("Action", consent.action)
                detail("Resource", consent.resource)
                detail("Tool", consent.tool)
                detail("Runtime reason", consent.reason)
                detail("Original request", originalMessage)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(maxHeight: 140)
        .padding(12)
        .background(.quaternary, in: RoundedRectangle(cornerRadius: 10))
    }

    @ViewBuilder
    private func detail(_ label: String, _ value: String?) -> some View {
        if let value {
            Text("\(label): \(value)")
                .font(.caption)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

struct ConsentChoiceLabel: View {
    let label: String
    let index: Int
    let consent: ConsentPresentation?

    var body: some View {
        VStack(spacing: 3) {
            Text(label)
            if let consent,
               consent.lifetimes.indices.contains(index),
               let text = consent.lifetimes[index].explanation {
                Text(text).font(.caption).foregroundStyle(.secondary)
            }
        }.frame(maxWidth: .infinity).padding(.vertical, 3)
    }
}
