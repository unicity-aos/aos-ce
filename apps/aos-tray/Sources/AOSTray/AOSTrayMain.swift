import Darwin
import Foundation
import AOSTrayCore

@main
enum AOSTrayMain {
    static func main() {
        switch LaunchArguments.parse(CommandLine.arguments) {
        case .failure(let error):
            write(FileHandle.standardError, "aos-tray: \(error.message)\n")
            write(FileHandle.standardError, LaunchArguments.helpText)
            write(FileHandle.standardError, "\n")
            exit(2)
        case .success(let arguments):
            if arguments.help {
                write(FileHandle.standardOutput, LaunchArguments.helpText)
                write(FileHandle.standardOutput, "\n")
                return
            }

            let store = TrayStore.make(mode: arguments.mode)
            if arguments.snapshot {
                do {
                    write(FileHandle.standardOutput, try TrayPresentation.make(from: store).json())
                } catch {
                    write(FileHandle.standardError, "aos-tray: failed to encode snapshot\n")
                    exit(1)
                }
                return
            }

            TrayApp.run(store: store)
        }
    }

    private static func write(_ handle: FileHandle, _ text: String) {
        try? handle.write(contentsOf: Data(text.utf8))
    }
}
