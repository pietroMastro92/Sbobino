import Foundation
import FluidAudio
import XCTest

@testable import SbobinoDiarizer

final class OutputTests: XCTestCase {
    func testPreflightUsesSDKRepositoryDirectory() throws {
        let base = ProcessInfo.processInfo.environment["SBOBINO_TEST_TMPDIR"]
            .map { URL(fileURLWithPath: $0, isDirectory: true) }
            ?? FileManager.default.temporaryDirectory
        let root = base.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let repository = root.appendingPathComponent(Repo.diarizer.folderName)
        try FileManager.default.createDirectory(at: repository, withIntermediateDirectories: true)
        for artifact in ModelNames.OfflineDiarizer.requiredModels {
            let path = repository.appendingPathComponent(artifact)
            if artifact.hasSuffix(".json") {
                try Data("{}".utf8).write(to: path)
            } else {
                try FileManager.default.createDirectory(at: path, withIntermediateDirectories: true)
            }
        }
        let marker = repository.appendingPathComponent(".fluidaudio-revision")
        XCTAssertThrowsError(try preflightOfflineArtifacts(in: root))
        try Data("wrong-revision\n".utf8).write(to: marker)
        XCTAssertThrowsError(try preflightOfflineArtifacts(in: root))
        try Data((Repo.diarizer.revision + "\n").utf8).write(to: marker)
        XCTAssertNoThrow(try preflightOfflineArtifacts(in: root))
        try FileManager.default.removeItem(at: repository.appendingPathComponent("plda-parameters.json"))
        XCTAssertThrowsError(try preflightOfflineArtifacts(in: root))
    }

    func testParserRequiresOfflineInputs() {
        XCTAssertThrowsError(try parseArguments(["offline", "--compute", "cpu"]))
        let inputs = ["--audio-path", "/tmp/audio.wav", "--models-dir", "/tmp/models", "--compute", "cpu"]
        XCTAssertThrowsError(try parseArguments(["benchmark"] + inputs))
        XCTAssertThrowsError(try parseArguments(["offline"] + inputs + ["--long-audio-path", "/tmp/long.wav"]))
        guard case .benchmark(_, let longAudio) = try? parseArguments(
            ["benchmark"] + inputs + ["--long-audio-path", "/tmp/long.wav"]
        ) else { return XCTFail("benchmark command not parsed") }
        XCTAssertEqual(longAudio.path, "/tmp/long.wav")
    }

    func testNormalizationRejectsInvalidTurnsAndDerivesStableLabel() throws {
        XCTAssertThrowsError(try normalizedSpeakers(
            from: [
                CandidateTurn(speakerID: "", startSeconds: 0, endSeconds: 1),
                CandidateTurn(speakerID: "S1", startSeconds: -1, endSeconds: 1),
                CandidateTurn(speakerID: "S1", startSeconds: 2, endSeconds: 2),
                CandidateTurn(speakerID: "S1", startSeconds: 1, endSeconds: 2),
            ], durationSeconds: 2
        ))
        let speakers = try normalizedSpeakers(
            from: [CandidateTurn(speakerID: "S1", startSeconds: 1, endSeconds: 2)], durationSeconds: 2
        )

        XCTAssertEqual(
            speakers,
            [
                OutputSpeaker(
                    speakerID: "S1",
                    speakerLabel: "Speaker 1",
                    startSeconds: 1,
                    endSeconds: 2
                ),
            ]
        )
    }

    func testAllInvalidTurnsDoNotBecomeEmptySuccess() {
        XCTAssertThrowsError(
            try normalizedSpeakers(
                from: [CandidateTurn(speakerID: "S1", startSeconds: 4, endSeconds: 3)], durationSeconds: 5
            )
        )
    }

    func testPaddingNeverExtendsBeyondAudio() throws {
        let turns = try normalizedSpeakers(from: [
            CandidateTurn(speakerID: "S1", startSeconds: 114.091, endSeconds: 120.0679),
            CandidateTurn(speakerID: "S1", startSeconds: 120.01, endSeconds: 120.1),
        ], durationSeconds: 120)
        XCTAssertEqual(turns.count, 1)
        XCTAssertEqual(turns.first?.endSeconds, 120)
        XCTAssertEqual(try normalizedSpeakers(
            from: [CandidateTurn(speakerID: "S1", startSeconds: 120, endSeconds: 120.1)],
            durationSeconds: 120
        ), [])
        XCTAssertThrowsError(try normalizedSpeakers(from: [], durationSeconds: .nan))
    }

    func testSerializationUsesStablePublicKeys() throws {
        let output = DiarizerOutput(
            version: 1,
            backend: "fluid_offline",
            speakers: [
                OutputSpeaker(
                    speakerID: "S1",
                    speakerLabel: "Speaker 1",
                    startSeconds: 0,
                    endSeconds: 1.5
                ),
            ],
            metrics: OutputMetrics(loadSeconds: 1, processingSeconds: 2, elapsedSeconds: 3)
        )

        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: encodeOutput(output)) as? [String: Any]
        )
        XCTAssertEqual(object["version"] as? Int, 1)
        XCTAssertEqual(object["backend"] as? String, "fluid_offline")
        XCTAssertNotNil(object["speakers"])
        XCTAssertNotNil(object["metrics"])
        XCTAssertNil(object["speaker_id"])
    }
}
