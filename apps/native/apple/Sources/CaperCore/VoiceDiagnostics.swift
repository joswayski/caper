import Foundation

// Only aggregate numeric media counters and a route classification leave this parser.
// Candidate addresses, IDs, device identifiers, and credentials are never retained.
struct VoiceStatistic {
    let type: String
    let values: [String: Any]
}

struct VoiceStatisticsSample {
    let timestampUs: Double
    let receivedBytes: Int64
    let sentBytes: Int64
}

/// One join's measured stages, as web's ConnectionDiagnostics rows.
public struct VoiceJoinTiming: Equatable {
    public let joinedMs: Int
    public let sessionMs: Int
    public let transportMs: Int
    public let rosterMs: Int
}

public struct VoiceDiagnostics {
    public let receivedBytes: Int64
    public let sentBytes: Int64
    public let receiveBitrate: Int?
    public let sendBitrate: Int?
    public let packetsLost: Int64
    public let maxJitterMs: Int?
    public let roundTripMs: Int?
    public let route: String
    /// Connectivity checks on the selected pair, e.g. "4 sent · 4 answered".
    public var checks: String? = nil
    public var timing: VoiceJoinTiming? = nil

    static func read(_ stats: [String: VoiceStatistic], timestampUs: Double, previous: VoiceStatisticsSample?) -> (VoiceDiagnostics, VoiceStatisticsSample) {
        var received: Int64 = 0, sent: Int64 = 0, lost: Int64 = 0
        var jitter: Double?, rtt: Double?
        for stat in stats.values {
            guard (stat.values["kind"] as? String ?? stat.values["mediaType"] as? String) == "audio" else { continue }
            switch stat.type {
            case "inbound-rtp":
                received += max(0, (stat.values["bytesReceived"] as? NSNumber)?.int64Value ?? 0)
                lost += max(0, (stat.values["packetsLost"] as? NSNumber)?.int64Value ?? 0)
                if let value = (stat.values["jitter"] as? NSNumber)?.doubleValue, value.isFinite, value >= 0 {
                    jitter = max(jitter ?? 0, value)
                }
            case "outbound-rtp": sent += max(0, (stat.values["bytesSent"] as? NSNumber)?.int64Value ?? 0)
            default: break
            }
        }
        var route = "unknown", checks: String?
        for stat in stats.values where stat.type == "transport" {
            guard let pairID = stat.values["selectedCandidatePairId"] as? String,
                  let pair = stats[pairID], pair.type == "candidate-pair",
                  let localID = pair.values["localCandidateId"] as? String,
                  let local = stats[localID], local.type == "local-candidate",
                  let kind = local.values["candidateType"] as? String else { continue }
            route = kind == "relay" ? "relay" : "direct"
            if let sentChecks = (pair.values["requestsSent"] as? NSNumber)?.int64Value {
                checks = "\(sentChecks) sent · \((pair.values["responsesReceived"] as? NSNumber)?.int64Value ?? 0) answered"
            }
            if let value = (pair.values["currentRoundTripTime"] as? NSNumber)?.doubleValue, value.isFinite, value >= 0 {
                rtt = value
            }
            break
        }
        let sample = VoiceStatisticsSample(timestampUs: timestampUs, receivedBytes: received, sentBytes: sent)
        let elapsed = previous.map { (timestampUs - $0.timestampUs) / 1_000_000 } ?? 0
        func bitrate(_ current: Int64, _ old: Int64?) -> Int? {
            guard let old, elapsed > 0, current >= old else { return nil }
            return Int(Double(current - old) * 8 / elapsed)
        }
        return (VoiceDiagnostics(receivedBytes: received, sentBytes: sent,
                                 receiveBitrate: bitrate(received, previous?.receivedBytes),
                                 sendBitrate: bitrate(sent, previous?.sentBytes), packetsLost: lost,
                                 maxJitterMs: jitter.map { Int($0 * 1_000) },
                                 roundTripMs: rtt.map { Int($0 * 1_000) }, route: route, checks: checks), sample)
    }
}
