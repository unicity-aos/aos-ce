import Darwin
import Foundation

/// Command Center Open files / Eject for one app-owned mountpoint.
///
/// Spawns the explicitly selected `aos` binary. `storage` is an inherited
/// runtime root: AOS resolves `AOS_HOME` and then sets product workspace env
/// (`ASTRID_HOME`, `ASTRID_WORKSPACE_STATE_DIR`, `ASTRID_RUN_DIR`) on the
/// bundled runtime. This type does not pass `--workspace`, `--admin`, `--fleet`,
/// or `--read-write`, and it does not start a daemon.
public enum CommandCenterVolumeError: Equatable, Error, Sendable {
    case unavailable
    case runtimeStopped
    case invalidPrincipal
    case occupied
    case invalidMountpoint
    case commandFailed
    case unverified
    case notMounted
    case stillMounted
    case requiresMacOS26
}

public enum CommandCenterVolumeCopy {
    public static let stopped =
        "Runtime is stopped. Start it before opening files. Nothing was mounted."
    public static let invalidPrincipal =
        "Choose an owned principal before opening files. Nothing was mounted."
    public static let occupied =
        "The Command Center folder is already in use by something else. Nothing was mounted."
    public static let invalidMountpoint =
        "The Command Center folder could not be prepared. Nothing was mounted."
    public static let commandFailed =
        "Couldn’t mount the AOS folder. Finder was not opened."
    public static let unverified =
        "Couldn’t verify the mounted AOS folder. Finder was not opened."
    public static let unavailable =
        "AOS is not bound to this Command Center. Nothing was mounted."
    public static let notMounted =
        "No AOS folder is mounted at the Command Center location."
    public static let ejectFailed =
        "Couldn’t eject the AOS folder. It may still be mounted."
    public static let stillMounted =
        "Eject did not unmount the AOS folder."
    public static let ejectPrincipal =
        "Choose an owned principal before ejecting. Nothing was unmounted."
    public static let requiresMacOS26 = DarwinPlatform.finderVolumeMountUnavailable
    public static let openCaption =
        "Open files mounts this Command Center’s folder if needed, then opens it in Finder. Eject unmounts that same folder. This is the selected principal’s view, not admin or root."

    public static func message(for error: CommandCenterVolumeError, ejecting: Bool = false) -> String {
        switch error {
        case .unavailable: return unavailable
        case .runtimeStopped: return stopped
        case .invalidPrincipal: return ejecting ? ejectPrincipal : invalidPrincipal
        case .occupied: return occupied
        case .invalidMountpoint: return invalidMountpoint
        case .commandFailed: return ejecting ? ejectFailed : commandFailed
        case .unverified: return unverified
        case .notMounted: return notMounted
        case .stillMounted: return stillMounted
        case .requiresMacOS26: return requiresMacOS26
        }
    }
}

public enum VolumeMountState: Equatable, Sendable {
    case readyEmpty
    case alreadyAstridFS
    case occupied
    case invalid
}

public enum VolumeOpenPlan: Equatable, Sendable {
    case mountThenOpen
    case openExisting
    case refuseStopped
    case refuseOccupied
    case refuseInvalid
}

public enum CommandCenterVolume {
    public static let relativeMountPath = "mnt/files"

    public static func resolvePrincipal(
        selected: String,
        discovery: PrincipalDiscovery
    ) throws -> String {
        switch PrincipalPicker.choose(selected: selected, from: discovery) {
        case .selected(let principal):
            return principal.id
        default:
            throw CommandCenterVolumeError.invalidPrincipal
        }
    }

    public static func mountpoint(home: String) throws -> String {
        guard let canonical = NativeRuntimeSetup.realHome(home) else {
            throw CommandCenterVolumeError.invalidMountpoint
        }
        return canonical + "/" + relativeMountPath
    }

    public static func mountArguments(principal: String, mountpoint: String) -> [String] {
        ["--principal", principal, "storage", "mount", "--as", principal, mountpoint]
    }

    public static func unmountArguments(principal: String, mountpoint: String) -> [String] {
        ["--principal", principal, "storage", "unmount", mountpoint]
    }

    public static func plan(
        runtimeState: RuntimeOverview.State,
        mountState: VolumeMountState
    ) -> VolumeOpenPlan {
        guard runtimeState == .running else { return .refuseStopped }
        switch mountState {
        case .alreadyAstridFS: return .openExisting
        case .readyEmpty: return .mountThenOpen
        case .occupied: return .refuseOccupied
        case .invalid: return .refuseInvalid
        }
    }

    public static func inspect(path: String) -> VolumeMountState {
        if MountedVolume.isExactAstridFS(path) { return .alreadyAstridFS }
        var info = stat()
        let existed = path.withCString { lstat($0, &info) } == 0
        if !existed {
            return errno == ENOENT ? .readyEmpty : .invalid
        }
        guard (info.st_mode & S_IFMT) == S_IFDIR else { return .occupied }
        guard info.st_uid == getuid(), (info.st_mode & 0o002) == 0 else { return .occupied }
        if isMountRoot(path) { return .occupied }
        do {
            let contents = try FileManager.default.contentsOfDirectory(atPath: path)
            return contents.isEmpty ? .readyEmpty : .occupied
        } catch {
            return .invalid
        }
    }

    public static func prepare(home: String) throws -> (path: String, state: VolumeMountState) {
        let path = try mountpoint(home: home)
        let state = inspect(path: path)
        switch state {
        case .alreadyAstridFS:
            return (path, state)
        case .readyEmpty:
            try ensureEmptyDirectory(path)
            let prepared = inspect(path: path)
            guard prepared == .readyEmpty || prepared == .alreadyAstridFS else {
                throw CommandCenterVolumeError.occupied
            }
            return (path, prepared)
        case .occupied:
            throw CommandCenterVolumeError.occupied
        case .invalid:
            throw CommandCenterVolumeError.invalidMountpoint
        }
    }

    public static func openFiles(
        binary: String,
        home: String,
        selectedPrincipal: String,
        discovery: PrincipalDiscovery,
        runtimeState: RuntimeOverview.State
    ) async throws -> URL {
        try await Task.detached {
            try openFilesBlocking(
                binary: binary, home: home, selectedPrincipal: selectedPrincipal,
                discovery: discovery, runtimeState: runtimeState,
                finderMountAvailable: DarwinPlatform.finderVolumeMountAvailable()
            )
        }.value
    }

    static func openFilesBlocking(
        binary: String,
        home: String,
        selectedPrincipal: String,
        discovery: PrincipalDiscovery,
        runtimeState: RuntimeOverview.State,
        finderMountAvailable: Bool = true
    ) throws -> URL {
        let principal = try resolvePrincipal(selected: selectedPrincipal, discovery: discovery)
        guard runtimeState == .running else { throw CommandCenterVolumeError.runtimeStopped }
        guard let canonicalHome = NativeRuntimeSetup.realHome(home) else {
            throw CommandCenterVolumeError.invalidMountpoint
        }
        let prepared = try prepare(home: canonicalHome)
        switch plan(runtimeState: runtimeState, mountState: prepared.state) {
        case .refuseStopped:
            throw CommandCenterVolumeError.runtimeStopped
        case .refuseOccupied:
            throw CommandCenterVolumeError.occupied
        case .refuseInvalid:
            throw CommandCenterVolumeError.invalidMountpoint
        case .openExisting:
            break
        case .mountThenOpen:
            guard finderMountAvailable else {
                throw CommandCenterVolumeError.requiresMacOS26
            }
            try run(
                binary: binary, home: canonicalHome, principal: principal,
                arguments: mountArguments(principal: principal, mountpoint: prepared.path),
                timeout: 60
            )
        }
        let status: RuntimeOverview
        do {
            status = try StatusCommandReader.readBlocking(
                binary: binary, home: canonicalHome, timeout: 25,
                principal: principal, mountpoint: prepared.path
            )
        } catch {
            throw CommandCenterVolumeError.unverified
        }
        guard let mount = status.mountedVolume else { throw CommandCenterVolumeError.unverified }
        do {
            return try mount.verifiedNativeRoot()
        } catch {
            throw CommandCenterVolumeError.unverified
        }
    }

    public static func eject(
        binary: String,
        home: String,
        selectedPrincipal: String,
        discovery: PrincipalDiscovery
    ) async throws {
        try await Task.detached {
            try ejectBlocking(
                binary: binary, home: home, selectedPrincipal: selectedPrincipal,
                discovery: discovery
            )
        }.value
    }

    static func ejectBlocking(
        binary: String,
        home: String,
        selectedPrincipal: String,
        discovery: PrincipalDiscovery
    ) throws {
        let principal = try resolvePrincipal(selected: selectedPrincipal, discovery: discovery)
        guard let canonicalHome = NativeRuntimeSetup.realHome(home) else {
            throw CommandCenterVolumeError.invalidMountpoint
        }
        let path = try mountpoint(home: canonicalHome)
        guard inspect(path: path) == .alreadyAstridFS else { throw CommandCenterVolumeError.notMounted }
        do {
            try run(
                binary: binary, home: canonicalHome, principal: principal,
                arguments: unmountArguments(principal: principal, mountpoint: path),
                timeout: 30
            )
        } catch {
            throw CommandCenterVolumeError.commandFailed
        }
        if MountedVolume.isExactAstridFS(path) {
            throw CommandCenterVolumeError.stillMounted
        }
    }

    static func run(
        binary: String,
        home: String,
        principal: String,
        arguments: [String],
        timeout: TimeInterval
    ) throws {
        guard binary.hasPrefix("/"), timeout > 0, timeout.isFinite else {
            throw CommandCenterVolumeError.unavailable
        }
        guard let canonicalHome = NativeRuntimeSetup.realHome(home) else {
            throw CommandCenterVolumeError.invalidMountpoint
        }
        guard OwnedPrincipal.isValidID(principal), principal != "anonymous" else {
            throw CommandCenterVolumeError.invalidPrincipal
        }
        guard arguments.allSatisfy({ !$0.isEmpty && !$0.contains("\0") }) else {
            throw CommandCenterVolumeError.commandFailed
        }
        let stdout = Pipe()
        let stderr = Pipe()
        defer {
            try? stdout.fileHandleForReading.close()
            try? stdout.fileHandleForWriting.close()
            try? stderr.fileHandleForReading.close()
            try? stderr.fileHandleForWriting.close()
        }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: binary)
        process.arguments = arguments
        var environment = ProcessInfo.processInfo.environment
        environment["AOS_HOME"] = canonicalHome
        environment["ASTRID_PRINCIPAL"] = principal
        process.environment = environment
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = stdout
        process.standardError = stderr
        try process.run()
        defer {
            if process.isRunning { _ = Darwin.kill(process.processIdentifier, SIGKILL) }
            process.waitUntilExit()
        }
        try stdout.fileHandleForWriting.close()
        try stderr.fileHandleForWriting.close()
        let started = ProcessInfo.processInfo.systemUptime
        while process.isRunning {
            guard ProcessInfo.processInfo.systemUptime - started < timeout else {
                throw CommandCenterVolumeError.commandFailed
            }
            Thread.sleep(forTimeInterval: 0.01)
        }
        let out = stdout.fileHandleForReading.readDataToEndOfFile()
        let err = stderr.fileHandleForReading.readDataToEndOfFile()
        guard out.count <= 65_536, err.count <= 16_384 else {
            throw CommandCenterVolumeError.commandFailed
        }
        guard process.terminationStatus == 0 else {
            throw CommandCenterVolumeError.commandFailed
        }
    }

    private static func ensureEmptyDirectory(_ path: String) throws {
        var info = stat()
        if path.withCString({ lstat($0, &info) }) == 0 {
            guard (info.st_mode & S_IFMT) == S_IFDIR else {
                throw CommandCenterVolumeError.occupied
            }
            return
        }
        guard errno == ENOENT else { throw CommandCenterVolumeError.invalidMountpoint }
        let parent = URL(fileURLWithPath: path, isDirectory: true).deletingLastPathComponent().path
        try ensurePrivateDirectory(parent)
        try mkdirPrivate(path)
    }

    private static func ensurePrivateDirectory(_ path: String) throws {
        var info = stat()
        if path.withCString({ lstat($0, &info) }) == 0 {
            guard (info.st_mode & S_IFMT) == S_IFDIR else {
                throw CommandCenterVolumeError.occupied
            }
            guard info.st_uid == getuid() else { throw CommandCenterVolumeError.occupied }
            return
        }
        guard errno == ENOENT else { throw CommandCenterVolumeError.invalidMountpoint }
        try mkdirPrivate(path)
    }

    private static func mkdirPrivate(_ path: String) throws {
        let result = path.withCString { Darwin.mkdir($0, 0o700) }
        guard result == 0 else { throw CommandCenterVolumeError.invalidMountpoint }
        var info = stat()
        guard path.withCString({ lstat($0, &info) }) == 0,
              (info.st_mode & S_IFMT) == S_IFDIR,
              info.st_uid == getuid() else {
            throw CommandCenterVolumeError.invalidMountpoint
        }
    }

    private static func isMountRoot(_ path: String) -> Bool {
        guard let canonical = NativeRuntimeSetup.realExistingDirectory(path) else { return false }
        var info = statfs()
        guard canonical.withCString({ statfs($0, &info) }) == 0 else { return false }
        let root = withUnsafeBytes(of: info.f_mntonname) {
            String(decoding: $0.prefix(while: { $0 != 0 }), as: UTF8.self)
        }
        return root == canonical
    }
}
