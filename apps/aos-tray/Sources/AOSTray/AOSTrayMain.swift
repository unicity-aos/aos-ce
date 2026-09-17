import Darwin
import Foundation
import AOSTrayCore

@main
enum AOSTrayMain {
    static func main() {
#if DEBUG
        if CommandLine.arguments.count == 3, CommandLine.arguments[1] == "--preview-runtime-input" {
            NativeRuntimeInputPreview.run(path: CommandLine.arguments[2])
            return
        }
        if CommandLine.arguments.count == 3, CommandLine.arguments[1] == "--preview-input" {
            NativeInputPreview.run(kind: CommandLine.arguments[2])
            return
        }
#endif
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

            let launch = InstalledRuntimeLaunch.apply(
                arguments: arguments,
                environment: ProcessInfo.processInfo.environment,
                userHome: FileManager.default.homeDirectoryForCurrentUser.path
            )
            TrayApp.run(store: store, socketPath: arguments.socketPath,
                        aosBinary: launch.aosBinary, aosHome: launch.aosHome,
                        expectedAosBinary: launch.expectedBinary,
                        launchError: launch.launchError,
                        nativeInputConfig: arguments.nativeInputConfig,
                        openOverview: arguments.openOverview)
        }
    }

    private static func write(_ handle: FileHandle, _ text: String) {
        try? handle.write(contentsOf: Data(text.utf8))
    }
}
