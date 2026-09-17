import Foundation
import Darwin

/// Runs only an explicitly selected AOS executable's read-only status command.
public enum StatusCommandReader {
    public static func read(binary: String, home: String, includeCapsules: Bool = false,
                            principal: String? = nil, mountpoint: String? = nil) async throws -> RuntimeOverview {
        try await Task.detached {
            try readBlocking(binary: binary, home: home, timeout: mountpoint != nil ? 25 : (includeCapsules ? 20 : 15),
                             includeCapsules: includeCapsules, principal: principal, mountpoint: mountpoint)
        }.value
    }

    // Internal deadline injection keeps timeout regressions fast without a UI/config knob.
    static func readBlocking(binary: String, home: String, timeout: TimeInterval = 15,
                             includeCapsules: Bool = false, principal: String? = nil,
                             mountpoint: String? = nil) throws -> RuntimeOverview {
        guard binary.hasPrefix("/"), home.hasPrefix("/"), timeout > 0, timeout.isFinite else {
            throw OverviewError.invalidStatus
        }
        let pipe = Pipe()
        defer {
            try? pipe.fileHandleForReading.close()
            try? pipe.fileHandleForWriting.close()
        }
        let descriptor = pipe.fileHandleForReading.fileDescriptor
        let flags = fcntl(descriptor, F_GETFL)
        guard flags >= 0, fcntl(descriptor, F_SETFL, flags | O_NONBLOCK) == 0 else {
            throw OverviewError.invalidStatus
        }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: binary)
        process.arguments = ["status", "--json"]
        if includeCapsules { process.arguments?.append("--include-capsules") }
        if let mountpoint {
            guard mountpoint.hasPrefix("/"), !includeCapsules else { throw OverviewError.invalidStatus }
            process.arguments?.append("--mountpoint=\(mountpoint)")
        }
        if let principal {
            guard !principal.isEmpty else { throw OverviewError.invalidStatus }
            process.arguments?.append("--principal=\(principal)")
        }
        var environment = ProcessInfo.processInfo.environment
        environment["AOS_HOME"] = home
        process.environment = environment
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = pipe
        process.standardError = FileHandle.nullDevice
        try process.run()
        // Never signal a daemon: this is solely the child we launched for status.
        defer {
            if process.isRunning { _ = Darwin.kill(process.processIdentifier, SIGKILL) }
            process.waitUntilExit()
        }
        try pipe.fileHandleForWriting.close()
        let started = ProcessInfo.processInfo.systemUptime
        var bytes = Data()
        var buffer = [UInt8](repeating: 0, count: 8192)
        while true {
            guard ProcessInfo.processInfo.systemUptime - started < timeout else {
                throw OverviewError.invalidStatus
            }
            let count = Darwin.read(descriptor, &buffer, buffer.count)
            if count > 0 {
                guard bytes.count + count <= 65_536 else { throw OverviewError.invalidStatus }
                bytes.append(contentsOf: buffer.prefix(count))
            } else if count == 0 {
                // EOF alone is not success: the command may close stdout and keep running.
                if !process.isRunning { break }
                Thread.sleep(forTimeInterval: 0.01)
            } else if errno == EINTR {
                continue
            } else if errno == EAGAIN || errno == EWOULDBLOCK {
                Thread.sleep(forTimeInterval: 0.01)
            } else {
                throw OverviewError.invalidStatus
            }
        }
        guard process.terminationStatus == 0 else { throw OverviewError.invalidStatus }
        let status = try RuntimeOverview.decode(bytes)
        if let mountpoint {
            guard status.mountedVolume?.mountpoint == mountpoint else { throw OverviewError.invalidStatus }
        }
        if includeCapsules, let principal {
            guard status.capsuleInventory?.principal == principal else { throw OverviewError.invalidStatus }
        }
        return status
    }
}
