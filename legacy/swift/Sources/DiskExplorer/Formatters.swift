import Foundation

enum Fmt {
    private static let byteFormatter: ByteCountFormatter = {
        let f = ByteCountFormatter()
        f.countStyle = .file
        f.allowsNonnumericFormatting = false
        return f
    }()

    private static let relativeFormatter: RelativeDateTimeFormatter = {
        let f = RelativeDateTimeFormatter()
        f.unitsStyle = .full
        f.dateTimeStyle = .named
        return f
    }()

    static func bytes(_ n: Int64) -> String { byteFormatter.string(fromByteCount: n) }

    static func exactBytes(_ n: Int64) -> String { n.formatted() + " bytes" }

    static func relative(_ date: Date) -> String {
        if date == .distantPast || date.timeIntervalSince1970 <= 0 { return "—" }
        return relativeFormatter.localizedString(for: date, relativeTo: Date())
    }

    static func absolute(_ date: Date) -> String {
        if date == .distantPast || date.timeIntervalSince1970 <= 0 { return "—" }
        return date.formatted(date: .abbreviated, time: .shortened)
    }

    static func count(_ n: Int) -> String { n.formatted() }

    static func duration(_ t: TimeInterval) -> String {
        if t < 60 { return String(format: "%.1f s", t) }
        let m = Int(t) / 60, s = Int(t) % 60
        return "\(m) min \(s) s"
    }

    static func percent(_ v: Double) -> String {
        v.formatted(.percent.precision(.fractionLength(v < 0.1 ? 1 : 0)))
    }
}
