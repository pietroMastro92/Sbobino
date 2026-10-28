import Foundation
import FluidAudio
import XCTest

@testable import SbobinoDiarizer

final class StreamingTests: XCTestCase {
    func testFinalFramesExcludeModelPaddingOnObservedLongRecording() {
        let frameSeconds = Double(Float(0.1))
        XCTAssertEqual(clippedStreamingFrameCount(10_495, samples: 16_789_675, frameSeconds: frameSeconds), 10_494)
        XCTAssertEqual(clippedStreamingFrameCount(1_201, samples: 1_920_000, frameSeconds: frameSeconds), 1_200)
        XCTAssertEqual(clippedStreamingFrameCount(5, samples: 5_600, frameSeconds: frameSeconds), 4)
        XCTAssertEqual(clippedStreamingFrameCount(0, samples: 0, frameSeconds: frameSeconds), 0)
    }

    func testPCMFragmentBoundariesAndTruncatedEOF() throws {
        var bytes = Data()
        for value in [Float(0.25), -0.5] {
            var bits = value.bitPattern.littleEndian
            withUnsafeBytes(of: &bits) { bytes.append(contentsOf: $0) }
        }
        var decoder = PCMFloatDecoder()
        XCTAssertEqual(try decoder.consume(bytes.prefix(3)), [])
        XCTAssertThrowsError(try decoder.finish())
        XCTAssertEqual(try decoder.consume(bytes.dropFirst(3)), [])
        XCTAssertEqual(try decoder.finish(), [0.25, -0.5])
        var nan = Float.nan.bitPattern.littleEndian
        _ = try withUnsafeBytes(of: &nan) { try decoder.consume(Data($0)) }
        XCTAssertThrowsError(try decoder.finish())
    }

    func testByteAtATimeInputStillEmitsOnlyFullChunks() throws {
        var decoder = PCMFloatDecoder()
        for _ in 0..<6_399 {
            XCTAssertEqual(try decoder.consume(Data([0])), [])
        }
        XCTAssertEqual(try decoder.consume(Data([0])), [Float](repeating: 0, count: 1_600))
        XCTAssertEqual(try decoder.finish(), [])
    }

    func testFrameLabelsIncludeActiveTailOverlapAndAbsoluteTime() throws {
        let chunk = DiarizerChunkResult(startFrame: 5,
                                       finalizedPredictions: [0.9, 0.1, 0.9, 0.9, 0.9, 0.9],
                                       finalizedFrameCount: 3)
        let turns = try streamingSpeakers(chunk, speakerCount: 2, frameSeconds: 0.1, duration: 0.75)
        XCTAssertEqual(turns.count, 2)
        XCTAssertEqual(turns[0].speakerID, "S1")
        XCTAssertEqual(turns[0].startSeconds, 0.5)
        XCTAssertEqual(turns[0].endSeconds, 0.75)
        XCTAssertEqual(turns[1].speakerID, "S2")
        XCTAssertEqual(turns[1].startSeconds, 0.6, accuracy: 0.000001)
        XCTAssertEqual(turns[1].endSeconds, 0.75)
        XCTAssertThrowsError(try streamingSpeakers(chunk, speakerCount: 3, frameSeconds: 0.1, duration: 1))
    }

    func testStreamingParserDoesNotAcceptDownloadsOrExtraOptions() throws {
        XCTAssertEqual(try parseStreamingArguments(["stream", "--model-path", "/tmp/a.mlmodelc"]).path,
                       "/tmp/a.mlmodelc")
        for arguments in [["stream"], ["stream", "--model-path", "--download"],
                          ["stream", "--model-path", "/tmp/a", "--download", "true"]] {
            XCTAssertThrowsError(try parseStreamingArguments(arguments))
        }
    }
}
