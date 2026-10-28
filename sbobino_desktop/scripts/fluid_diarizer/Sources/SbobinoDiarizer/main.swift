@preconcurrency import CoreML
import AVFoundation
import Darwin
import FluidAudio
import Foundation

enum ComputeMode: String {
    case cpu
    case ane

    var modelComputeUnits: MLComputeUnits {
        switch self {
        case .cpu:
            return .cpuOnly
        case .ane:
            return .cpuAndNeuralEngine
        }
    }
}

struct CLIOptions {
    let audioPath: URL
    let modelsDirectory: URL
    let compute: ComputeMode
}

enum ParsedArguments {
    case help
    case run(CLIOptions)
    case benchmark(CLIOptions, URL)
}

enum CLIError: Error, CustomStringConvertible {
    case invalidArguments(String)
    case missingInput(String)
    case missingArtifact(URL)
    case invalidArtifact(URL)
    case invalidResult(String)

    var exitCode: Int32 {
        switch self {
        case .invalidArguments(_):
            return 2
        case .missingInput(_), .missingArtifact(_), .invalidArtifact(_), .invalidResult(_):
            return 1
        }
    }

    var description: String {
        switch self {
        case .invalidArguments(let message):
            return "invalid arguments: \(message)\n\n\(usageText)"
        case .missingInput(let message):
            return "input error: \(message)"
        case .missingArtifact(let url):
            return "missing offline diarizer artifact: \(url.path)"
        case .invalidArtifact(let url):
            return "invalid offline diarizer artifact: \(url.path)"
        case .invalidResult(let message):
            return "invalid diarizer result: \(message)"
        }
    }
}

let usageText = """
Usage:
  sbobino-diarizer offline --audio-path <wav> --models-dir <cache-root> --compute <cpu|ane>
  sbobino-diarizer benchmark --audio-path <wav> --long-audio-path <wav> --models-dir <cache-root> --compute <cpu|ane>
  sbobino-diarizer stream --model-path <mlmodelc>  # stdin: mono Float32 little-endian, 16 kHz; stdout: NDJSON
  sbobino-diarizer --help
"""

func parseArguments(_ arguments: [String]) throws -> ParsedArguments {
    if arguments == ["--help"] || arguments == ["-h"] {
        return .help
    }

    let benchmark = arguments.first == "benchmark"
    guard arguments.first == "offline" || benchmark else {
        throw CLIError.invalidArguments("expected offline or benchmark")
    }
    if arguments.count == 2, (arguments[1] == "--help" || arguments[1] == "-h") {
        return .help
    }

    var audioPath: URL?
    var modelsDirectory: URL?
    var compute: ComputeMode?
    var longAudioPath: URL?
    var index = 1

    while index < arguments.count {
        let option = arguments[index]
        guard index + 1 < arguments.count else {
            throw CLIError.invalidArguments("missing value for \(option)")
        }
        let value = arguments[index + 1]
        guard !value.hasPrefix("-") else {
            throw CLIError.invalidArguments("missing value for \(option)")
        }

        switch option {
        case "--long-audio-path":
            guard benchmark, longAudioPath == nil else {
                throw CLIError.invalidArguments("--long-audio-path is required once for benchmark only")
            }
            longAudioPath = URL(fileURLWithPath: value).standardizedFileURL
        case "--audio-path":
            guard audioPath == nil else {
                throw CLIError.invalidArguments("duplicate --audio-path")
            }
            audioPath = URL(fileURLWithPath: value, isDirectory: false)
        case "--models-dir":
            guard modelsDirectory == nil else {
                throw CLIError.invalidArguments("duplicate --models-dir")
            }
            modelsDirectory = URL(fileURLWithPath: value, isDirectory: true)
        case "--compute":
            guard compute == nil else {
                throw CLIError.invalidArguments("duplicate --compute")
            }
            guard let parsedCompute = ComputeMode(rawValue: value) else {
                throw CLIError.invalidArguments("--compute must be cpu or ane")
            }
            compute = parsedCompute
        default:
            throw CLIError.invalidArguments("unknown option \(option)")
        }
        index += 2
    }

    guard let audioPath, let modelsDirectory, let compute else {
        throw CLIError.invalidArguments("offline requires --audio-path, --models-dir, and --compute")
    }

    let options = CLIOptions(
            audioPath: audioPath.standardizedFileURL,
            modelsDirectory: modelsDirectory.standardizedFileURL,
            compute: compute
        )
    if benchmark {
        guard let longAudioPath else {
            throw CLIError.invalidArguments("benchmark requires --long-audio-path")
        }
        return .benchmark(options, longAudioPath)
    }
    return .run(options)
}

private struct RequiredArtifact {
    let name: String
    let isDirectory: Bool
}

private let requiredOfflineArtifacts = [
    RequiredArtifact(name: "Segmentation.mlmodelc", isDirectory: true),
    RequiredArtifact(name: "FBank.mlmodelc", isDirectory: true),
    RequiredArtifact(name: "Embedding.mlmodelc", isDirectory: true),
    RequiredArtifact(name: "PldaRho.mlmodelc", isDirectory: true),
    RequiredArtifact(name: "plda-parameters.json", isDirectory: false),
]

func preflightOfflineArtifacts(in modelsDirectory: URL, fileManager: FileManager = .default) throws {
    let diarizerRoot = modelsDirectory.appendingPathComponent(
        Repo.diarizer.folderName, isDirectory: true
    )

    for artifact in requiredOfflineArtifacts {
        let artifactURL = diarizerRoot.appendingPathComponent(artifact.name, isDirectory: artifact.isDirectory)
        var isDirectory = ObjCBool(false)
        guard fileManager.fileExists(atPath: artifactURL.path, isDirectory: &isDirectory) else {
            throw CLIError.missingArtifact(artifactURL)
        }
        guard isDirectory.boolValue == artifact.isDirectory else {
            throw CLIError.invalidArtifact(artifactURL)
        }
    }
    let revisionMarker = diarizerRoot.appendingPathComponent(".fluidaudio-revision")
    guard let revision = try? String(contentsOf: revisionMarker, encoding: .utf8),
        revision.trimmingCharacters(in: .whitespacesAndNewlines) == Repo.diarizer.revision
    else {
        throw CLIError.invalidArtifact(revisionMarker)
    }
}

private func preflightAudio(at audioURL: URL, fileManager: FileManager = .default) throws -> Double {
    var isDirectory = ObjCBool(false)
    guard fileManager.fileExists(atPath: audioURL.path, isDirectory: &isDirectory), !isDirectory.boolValue else {
        throw CLIError.missingInput("audio file not found: \(audioURL.path)")
    }
    let audio = try AVAudioFile(forReading: audioURL)
    let duration = Double(audio.length) / audio.processingFormat.sampleRate
    guard duration.isFinite, duration > 0 else {
        throw CLIError.missingInput("audio has no finite positive duration")
    }
    return duration
}

struct CandidateTurn {
    let speakerID: String
    let startSeconds: Double
    let endSeconds: Double
}

struct OutputSpeaker: Codable, Equatable {
    let speakerID: String
    let speakerLabel: String
    let startSeconds: Double
    let endSeconds: Double

    enum CodingKeys: String, CodingKey {
        case speakerID = "speaker_id"
        case speakerLabel = "speaker_label"
        case startSeconds = "start_seconds"
        case endSeconds = "end_seconds"
    }
}

struct OutputMetrics: Codable, Equatable {
    let loadSeconds: Double
    let processingSeconds: Double
    let elapsedSeconds: Double

    enum CodingKeys: String, CodingKey {
        case loadSeconds = "load_seconds"
        case processingSeconds = "processing_seconds"
        case elapsedSeconds = "elapsed_seconds"
    }
}

struct DiarizerOutput: Codable, Equatable {
    let version: Int
    let backend: String
    let speakers: [OutputSpeaker]
    let metrics: OutputMetrics
}

private func speakerLabel(for speakerID: String) -> String {
    guard speakerID.first == "S", let number = Int(speakerID.dropFirst()), number > 0 else {
        // Keep an upstream identifier that does not follow FluidAudio's S<N> form;
        // do not invent a different speaker number.
        return speakerID
    }
    return "Speaker \(number)"
}

func normalizedSpeakers(from candidates: [CandidateTurn], durationSeconds: Double) throws -> [OutputSpeaker] {
    guard durationSeconds.isFinite, durationSeconds > 0 else {
        throw CLIError.invalidResult("invalid audio duration")
    }
    let speakers = try candidates.compactMap { candidate -> OutputSpeaker? in
        let speakerID = candidate.speakerID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !speakerID.isEmpty,
            candidate.startSeconds.isFinite,
            candidate.endSeconds.isFinite,
            candidate.startSeconds >= 0,
            candidate.endSeconds > candidate.startSeconds
        else {
            throw CLIError.invalidResult("invalid speaker identifier or timestamp")
        }
        guard candidate.startSeconds < durationSeconds else { return nil }

        return OutputSpeaker(
            speakerID: speakerID,
            speakerLabel: speakerLabel(for: speakerID),
            startSeconds: candidate.startSeconds,
            endSeconds: min(candidate.endSeconds, durationSeconds)
        )
    }

    return speakers.sorted {
        if $0.startSeconds != $1.startSeconds {
            return $0.startSeconds < $1.startSeconds
        }
        if $0.endSeconds != $1.endSeconds {
            return $0.endSeconds < $1.endSeconds
        }
        return $0.speakerID < $1.speakerID
    }
}

func encodeOutput<T: Encodable>(_ output: T) throws -> Data {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys]
    var data = try encoder.encode(output)
    data.append(0x0A)
    return data
}

private func loadOfflineManager(_ options: CLIOptions) async throws -> OfflineDiarizerManager {
    #if arch(x86_64)
    if options.compute == .ane {
        throw CLIError.invalidArguments("--compute ane requires Apple Silicon")
    }
    #endif

    try preflightOfflineArtifacts(in: options.modelsDirectory)

    // Set this before entering any FluidAudio model loader. No download, purge,
    // or online fallback is allowed by this helper.
    ModelHub.offlineMode = true

    var diarizerConfig = OfflineDiarizerConfig()
    diarizerConfig.segmentation.batchSize = 8
    diarizerConfig.embedding.batchSize = 8

    let modelConfiguration = MLModelConfiguration()
    modelConfiguration.computeUnits = options.compute.modelComputeUnits

    let models = try await OfflineDiarizerModels.load(
        from: options.modelsDirectory,
        configuration: modelConfiguration
    )
    let manager = OfflineDiarizerManager(config: diarizerConfig)
    manager.initialize(models: models)
    return manager
}

private func processOffline(
    _ manager: OfflineDiarizerManager, audioPath: URL, loadSeconds: Double, elapsedStart: Date
) async throws -> DiarizerOutput {
    let duration = try preflightAudio(at: audioPath)

    let processingStart = Date()
    let result = try await manager.process(audioPath)
    let processingSeconds = max(0, Date().timeIntervalSince(processingStart))

    let candidates = result.segments.map {
        CandidateTurn(
            speakerID: $0.speakerId,
            startSeconds: Double($0.startTimeSeconds),
            endSeconds: Double($0.endTimeSeconds)
        )
    }
    // Core ML windows include right padding; never persist labels past the recording.
    let speakers = try normalizedSpeakers(from: candidates, durationSeconds: duration)

    return DiarizerOutput(
        version: 1,
        backend: "fluid_offline",
        speakers: speakers,
        metrics: OutputMetrics(
            loadSeconds: loadSeconds,
            processingSeconds: processingSeconds,
            elapsedSeconds: max(0, Date().timeIntervalSince(elapsedStart))
        )
    )
}

func runOffline(_ options: CLIOptions) async throws -> DiarizerOutput {
    _ = try preflightAudio(at: options.audioPath)
    let start = Date()
    let manager = try await loadOfflineManager(options)
    return try await processOffline(
        manager, audioPath: options.audioPath,
        loadSeconds: Date().timeIntervalSince(start), elapsedStart: start
    )
}

struct BenchmarkRun: Encodable {
    let phase: String
    let audioPath: String
    let output: DiarizerOutput
}

struct BenchmarkOutput: Encodable {
    let version = 1
    let coldDefinition = "first model load and inference in this process; OS file cache is not flushed"
    let runs: [BenchmarkRun]
}

func runBenchmark(_ options: CLIOptions, longAudioPath: URL) async throws -> BenchmarkOutput {
    _ = try preflightAudio(at: options.audioPath)
    _ = try preflightAudio(at: longAudioPath)
    let start = Date()
    let manager = try await loadOfflineManager(options)
    let loadSeconds = Date().timeIntervalSince(start)
    var runs: [BenchmarkRun] = []
    for (index, phase) in ["cold", "warm_1", "warm_2", "warm_3", "long"].enumerated() {
        let audio = phase == "long" ? longAudioPath : options.audioPath
        let output = try await processOffline(
            manager, audioPath: audio, loadSeconds: index == 0 ? loadSeconds : 0,
            elapsedStart: index == 0 ? start : Date()
        )
        runs.append(BenchmarkRun(phase: phase, audioPath: audio.path, output: output))
    }
    return BenchmarkOutput(runs: runs)
}

private func writeStandardError(_ message: String) {
    FileHandle.standardError.write(Data((message + "\n").utf8))
}

private func writeStandardOutput(_ data: Data) {
    FileHandle.standardOutput.write(data)
}

@main
struct SbobinoDiarizerMain {
    static func main() async {
        do {
            let arguments = Array(CommandLine.arguments.dropFirst())
            if arguments.first == "stream" {
                try runStreaming(modelURL: parseStreamingArguments(arguments))
                return
            }
            switch try parseArguments(arguments) {
            case .help:
                writeStandardOutput(Data(usageText.utf8))
            case .run(let options):
                writeStandardOutput(try encodeOutput(await runOffline(options)))
            case .benchmark(let options, let longAudioPath):
                writeStandardOutput(try encodeOutput(await runBenchmark(options, longAudioPath: longAudioPath)))
            }
        } catch {
            writeStandardError(String(describing: error))
            if let cliError = error as? CLIError {
                exit(cliError.exitCode)
            }
            exit(1)
        }
    }
}
