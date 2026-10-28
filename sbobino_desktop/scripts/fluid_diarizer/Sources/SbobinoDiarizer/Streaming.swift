@preconcurrency import CoreML
import FluidAudio
import Foundation

func parseStreamingArguments(_ arguments: [String]) throws -> URL {
    guard arguments.count == 3, arguments[0] == "stream", arguments[1] == "--model-path",
        !arguments[2].hasPrefix("-")
    else { throw CLIError.invalidArguments("stream requires --model-path <local mlmodelc>") }
    return URL(fileURLWithPath: arguments[2]).standardizedFileURL
}

struct PCMFloatDecoder {
    private var pending = Data()

    mutating func consume(_ bytes: Data) throws -> [Float] {
        pending.append(bytes)
        guard pending.count >= 6_400 else { return [] }
        return try decode(byteCount: 6_400)
    }

    private mutating func decode(byteCount: Int) throws -> [Float] {
        let samples = pending.withUnsafeBytes { raw in
            stride(from: 0, to: byteCount, by: 4).map { offset in
                Float(bitPattern: UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: offset, as: UInt32.self)))
            }
        }
        pending = Data(pending.suffix(pending.count - byteCount))
        guard samples.allSatisfy(\.isFinite) else {
            throw CLIError.missingInput("stream contains non-finite PCM samples")
        }
        return samples
    }

    mutating func finish() throws -> [Float] {
        guard pending.count % 4 == 0 else { throw CLIError.missingInput("truncated Float32 PCM sample") }
        return try decode(byteCount: pending.count)
    }
}

func streamingSpeakers(
    _ chunk: DiarizerChunkResult, speakerCount: Int, frameSeconds: Double, duration: Double
) throws -> [OutputSpeaker] {
    guard speakerCount > 0, speakerCount <= 32, chunk.startFrame >= 0,
        chunk.finalizedFrameCount >= 0, frameSeconds.isFinite, frameSeconds > 0,
        chunk.finalizedPredictions.count == chunk.finalizedFrameCount * speakerCount,
        chunk.finalizedPredictions.allSatisfy(\.isFinite)
    else { throw CLIError.invalidResult("invalid LS-EEND frame dimensions or probabilities") }

    var candidates: [CandidateTurn] = []
    for speaker in 0..<speakerCount {
        var start: Int?
        for frame in 0...chunk.finalizedFrameCount {
            let active = frame < chunk.finalizedFrameCount
                && chunk.finalizedPredictions[frame * speakerCount + speaker] > 0.5
            if active, start == nil { start = frame }
            if !active, let first = start {
                candidates.append(CandidateTurn(
                    speakerID: "S\(speaker + 1)",
                    startSeconds: Double(chunk.startFrame + first) * frameSeconds,
                    endSeconds: Double(chunk.startFrame + frame) * frameSeconds
                ))
                start = nil
            }
        }
    }
    return try normalizedSpeakers(from: candidates, durationSeconds: duration)
}

private func writeStreamEvent(_ event: [String: Any]) throws {
    var event = event
    event["version"] = 1
    event["backend"] = "fluid_lseend"
    var bytes = try JSONSerialization.data(withJSONObject: event, options: [.sortedKeys])
    bytes.append(0x0A)
    try FileHandle.standardOutput.write(contentsOf: bytes)
}

func clippedStreamingFrameCount(_ processed: Int, samples: Int, frameSeconds: Double) -> Int {
    min(processed, Int(ceil(Double(samples) / 16_000 / frameSeconds)))
}

func runStreaming(modelURL: URL) throws {
    guard modelURL.pathExtension == "mlmodelc",
        FileManager.default.fileExists(atPath: modelURL.path)
    else { throw CLIError.missingArtifact(modelURL) }
    let start = ProcessInfo.processInfo.systemUptime
    // Direct local loader: streaming must never download, purge, or change model files.
    let model = try LSEENDModel(modelURL: modelURL, computeUnits: .cpuOnly)
    var config = DiarizerTimelineConfig.default(
        numSpeakers: model.metadata.maxSpeakers, frameDurationSeconds: model.metadata.frameDurationSeconds
    )
    config.maxStoredFrames = 0
    config.storeSegments = false
    let manager = try LSEENDDiarizer(model: model, timelineConfig: config)
    let frameSeconds = Double(model.metadata.frameDurationSeconds)
    var decoder = PCMFloatDecoder()
    var received = 0
    var processedFrames = 0
    var processingSeconds = 0.0
    try writeStreamEvent([
        "type": "ready", "load_seconds": ProcessInfo.processInfo.systemUptime - start,
        "input_format": "mono-f32le-16000", "speaker_capacity": model.metadata.maxSpeakers,
        "frame_seconds": frameSeconds,
    ])

    func emit(_ update: DiarizerTimelineUpdate?) throws {
        guard let chunk = update?.chunkResult else { return }
        guard chunk.startFrame == processedFrames else {
            throw CLIError.invalidResult("non-contiguous streaming frames")
        }
        processedFrames += chunk.finalizedFrameCount
        let turns = try streamingSpeakers(
            chunk, speakerCount: model.metadata.maxSpeakers,
            frameSeconds: frameSeconds, duration: Double(received) / 16_000
        )
        let speakers = try JSONSerialization.jsonObject(with: encodeOutput(turns))
        try writeStreamEvent([
            "type": "update", "samples_received": received,
            "finalized_frames": clippedStreamingFrameCount(processedFrames, samples: received, frameSeconds: frameSeconds),
            "processed_frames": processedFrames,
            "speakers": speakers, "processing_seconds": processingSeconds,
        ])
    }

    // Read at most 100 ms per step; neither PCM nor prediction history grows with recording length.
    while let bytes = try FileHandle.standardInput.read(upToCount: 6_400), !bytes.isEmpty {
        let samples = try decoder.consume(bytes)
        guard !samples.isEmpty else { continue }
        received += samples.count
        let processingStart = ProcessInfo.processInfo.systemUptime
        let update = try manager.process(samples: samples, sourceSampleRate: 16_000)
        processingSeconds += ProcessInfo.processInfo.systemUptime - processingStart
        try emit(update)
    }
    let tail = try decoder.finish()
    if !tail.isEmpty {
        received += tail.count
        let processingStart = ProcessInfo.processInfo.systemUptime
        let update = try manager.process(samples: tail, sourceSampleRate: 16_000)
        processingSeconds += ProcessInfo.processInfo.systemUptime - processingStart
        try emit(update)
    }
    let drainStart = ProcessInfo.processInfo.systemUptime
    let last = try manager.finalizeSession()
    processingSeconds += ProcessInfo.processInfo.systemUptime - drainStart
    try emit(last)
    // The SDK drains padded right context; report recording frames separately from model frames.
    let finalizedFrames = clippedStreamingFrameCount(processedFrames, samples: received, frameSeconds: frameSeconds)
    try writeStreamEvent([
        "type": "done", "samples_received": received, "finalized_frames": finalizedFrames,
        "processed_frames": processedFrames, "padding_frames": processedFrames - finalizedFrames,
        "processing_seconds": processingSeconds,
        "drain_seconds": ProcessInfo.processInfo.systemUptime - drainStart,
    ])
}
