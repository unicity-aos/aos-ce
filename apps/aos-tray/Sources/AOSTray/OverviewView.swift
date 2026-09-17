import SwiftUI
import AOSTrayCore

struct OverviewView: View {
    @ObservedObject var session: TraySession

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    Label("Runtime", systemImage: "cpu").font(.headline)
                    Spacer()
                    if session.refreshingOverview { ProgressView().controlSize(.small) }
                    Button("Refresh") { Task { await session.refreshOverview() } }
                        .disabled(session.aosBinary == nil || session.refreshingOverview)
                }
                if let status = session.overview {
                    row("State", status.state == .running ? "Running" : "Stopped")
                    row(status.state == .running ? "Runtime version" : "Expected runtime version", status.runtimeVersion)
                    row("Connected clients", String(status.connectedClients))
                    row("Loaded capsules", String(status.loadedCapsules.count))
                } else {
                    Text(session.overviewError ?? "Choose an AOS installation at launch to inspect its runtime.")
                        .foregroundStyle(.secondary)
                }
                Divider()
                Label("Volume", systemImage: "externaldrive").font(.headline)
                if let volume = session.volume {
                    Text(volume.url.path).font(.caption).textSelection(.enabled)
                        .lineLimit(2).truncationMode(.middle)
                    row("Container file size", ByteCountFormatter.string(
                        fromByteCount: Int64(clamping: volume.fileBytes), countStyle: .file))
                    Button("Reveal volume file in Finder") { session.revealVolume() }
                    Text("Opens the container’s location, not a mounted filesystem.")
                        .font(.caption).foregroundStyle(.secondary)
                } else {
                    Text("Volume file unavailable.").foregroundStyle(.secondary)
                }
                HStack {
                    Button("Open files") { session.openFiles() }
                        .disabled(session.aosBinary == nil || session.volumeBusy)
                    Button("Eject") { session.ejectVolume() }
                        .disabled(session.aosBinary == nil || session.volumeBusy)
                    if session.filesBusy { ProgressView().controlSize(.small) }
                }
                Text(CommandCenterVolumeCopy.openCaption)
                    .font(.caption).foregroundStyle(.secondary)
                HStack {
                    Button("Open mounted volume…") { session.chooseMountedVolume() }
                        .disabled(session.aosBinary == nil || session.volumeBusy)
                    if session.checkingMount { ProgressView().controlSize(.small) }
                }
                if let error = session.mountError {
                    Text(error).font(.caption).foregroundStyle(.secondary)
                }
                DisclosureGroup("About these readings") {
                    Text("Loaded capsules are not the complete installed library. Container file size is not allocated disk space or available capacity. Open files mounts the Command Center folder if needed, then opens Finder. Eject unmounts that same folder. Open mounted volume verifies an existing macOS mount; it does not mount or start anything.")
                        .font(.caption).foregroundStyle(.secondary)
                }.font(.caption)
            }
        }.task { await session.refreshOverview() }
    }

    private func row(_ title: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Text(title).foregroundStyle(.secondary)
            Spacer(minLength: 12)
            Text(value).textSelection(.enabled)
        }
    }
}
