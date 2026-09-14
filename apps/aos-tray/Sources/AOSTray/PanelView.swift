import SwiftUI
import AOSTrayCore

struct PanelView: View {
    @ObservedObject var session: TraySession

    var body: some View {
        let presentation = session.presentation
        VStack(alignment: .leading, spacing: 10) {
            if presentation.showsDemoBanner {
                Text(presentation.demoBannerText)
                    .font(.system(size: 13, weight: .bold))
                    .foregroundStyle(.black)
                    .padding(8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.yellow)
                    .accessibilityIdentifier("demo-banner")
                    .accessibilityLabel(presentation.demoBannerText)
            }

            Text(presentation.connectionLabel)
                .font(.system(size: 18, weight: .semibold, design: .monospaced))
                .accessibilityIdentifier("connection-label")
                .accessibilityLabel(presentation.connectionLabel)

            Text(presentation.explanation)
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            Picker("Section", selection: $session.section) {
                Text("Requests").tag(PanelSection.requests)
                Text("Capsules").tag(PanelSection.capsules)
            }
            .pickerStyle(.segmented)
            .accessibilityLabel("Panel section")

            if let error = session.lastError {
                Text(error)
                    .font(.system(size: 12))
                    .foregroundStyle(.red)
                    .accessibilityIdentifier("decision-error")
            }

            Group {
                if session.section == .requests {
                    requestList(presentation)
                } else {
                    capsuleList(presentation)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
        .padding(12)
        .frame(width: 440, height: 520, alignment: .topLeading)
    }

    @ViewBuilder
    private func requestList(_ presentation: TrayPresentation) -> some View {
        if presentation.requests.isEmpty {
            Text(presentation.emptyRequestsText)
                .font(.system(size: 13))
                .foregroundStyle(.secondary)
                .accessibilityIdentifier("empty-requests")
        } else {
            List(presentation.requests) { request in
                VStack(alignment: .leading, spacing: 6) {
                    Text(request.stateLabel)
                        .font(.system(size: 11, weight: .semibold))
                    Text("principal \(request.principal)")
                    Text("capsule \(request.capsule)")
                    Text("scope \(request.scope)")
                    Text(request.reason)
                        .foregroundStyle(.secondary)
                    if let applied = request.appliedDecision {
                        Text("decision \(applied)")
                            .foregroundStyle(.secondary)
                    }
                    if request.prompt {
                        HStack {
                            ForEach(request.decisions) { button in
                                Button(button.label) {
                                    session.apply(requestID: request.id, decision: button.verb)
                                }
                                .accessibilityLabel(button.label)
                            }
                        }
                    }
                }
                .textSelection(.enabled)
                .padding(.vertical, 4)
                .accessibilityElement(children: .combine)
                .accessibilityLabel(
                    "\(request.stateLabel), principal \(request.principal), capsule \(request.capsule), scope \(request.scope)"
                )
            }
            .listStyle(.plain)
        }
    }

    @ViewBuilder
    private func capsuleList(_ presentation: TrayPresentation) -> some View {
        if presentation.capsules.isEmpty {
            Text(presentation.emptyCapsulesText)
                .font(.system(size: 13))
                .foregroundStyle(.secondary)
                .accessibilityIdentifier("empty-capsules")
        } else {
            List(presentation.capsules) { capsule in
                VStack(alignment: .leading, spacing: 6) {
                    Text(capsule.stateLabel)
                        .font(.system(size: 11, weight: .semibold))
                    Text("principal \(capsule.principal)")
                    Text("capsule \(capsule.capsule)")
                    Text("scope \(capsule.scope)")
                    if capsule.pregranted {
                        Text("Pregranted. No prompt.")
                            .foregroundStyle(.secondary)
                    }
                }
                .textSelection(.enabled)
                .padding(.vertical, 4)
                .accessibilityElement(children: .combine)
                .accessibilityLabel(
                    "\(capsule.stateLabel), principal \(capsule.principal), capsule \(capsule.capsule), scope \(capsule.scope)"
                )
            }
            .listStyle(.plain)
        }
    }
}
