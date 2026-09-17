import SwiftUI
import AOSTrayCore

struct LibraryPrincipalForm: View {
    let discovery: PrincipalDiscovery?
    let selected: String
    let cancel: () -> Void
    let select: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Choose a principal").font(.headline)
            Text(PrincipalDiscoveryCopy.localOperatorExplanation)
                .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            content
            HStack {
                Button("Cancel", action: cancel)
                Spacer()
            }
        }.padding(16).frame(width: 320)
    }

    @ViewBuilder
    private var content: some View {
        switch discovery {
        case .owned(let principals) where principals.isEmpty:
            Text(PrincipalDiscoveryCopy.emptyDirectory).font(.callout)
        case .owned(let principals):
            ScrollView {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(principals) { principal in
                        Button {
                            if principal.enabled { select(principal.id) }
                        } label: {
                            HStack {
                                Text(principal.id)
                                Spacer()
                                if !principal.enabled {
                                    Text("Unavailable").font(.caption).foregroundStyle(.secondary)
                                } else if principal.id == selected {
                                    Image(systemName: "checkmark")
                                }
                            }
                        }
                        .buttonStyle(.plain)
                        .disabled(!principal.enabled)
                        .accessibilityLabel(principal.enabled ? principal.id : "\(principal.id), unavailable")
                    }
                }
            }.frame(maxHeight: 180)
        case .unsupported:
            Text(PrincipalDiscoveryCopy.unsupported).font(.callout)
        case .failed:
            Text(PrincipalDiscoveryCopy.failed).font(.callout)
        case nil:
            Text("Refresh to discover owned principals.").font(.callout).foregroundStyle(.secondary)
        }
    }
}
