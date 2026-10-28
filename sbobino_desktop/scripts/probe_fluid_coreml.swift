// Isolate native CPU model compatibility; synthetic tensors do not measure diarization quality.
import CoreML
import Foundation

let args = Array(CommandLine.arguments.dropFirst())
guard args.count == 3, let batch = Int(args[2]), [1, 8, 32].contains(batch) else {
    fatalError("usage: probe_fluid_coreml.swift <verified model directory> <model> <1|8|32>")
}
let shapes: [String: [String: [Int]]] = [
    "Segmentation": ["audio": [batch, 1, 160000]],
    "FBank": ["audio": [batch, 1, 160000]],
    "Embedding": ["fbank_features": [batch, 1, 80, 998], "weights": [batch, 589]],
    "PldaRho": ["embeddings": [batch, 256]],
]
guard let inputs = shapes[args[1]] else { fatalError("unknown pinned model") }
func phase(_ value: String) {
    FileHandle.standardError.write(Data((value + "\n").utf8))
}
do {
    phase("loading \(args[1]) cpu batch=\(batch)")
    let config = MLModelConfiguration()
    config.computeUnits = .cpuOnly
    let model = try MLModel(contentsOf: URL(fileURLWithPath: args[0])
        .appendingPathComponent(args[1] + ".mlmodelc"), configuration: config)
    precondition(Set(inputs.keys) == Set(model.modelDescription.inputDescriptionsByName.keys))
    var features: [String: MLFeatureValue] = [:]
    for (name, shape) in inputs {
        let array = try MLMultiArray(shape: shape.map(NSNumber.init(value:)), dataType: .float32)
        let pointer = array.dataPointer.assumingMemoryBound(to: Float.self)
        for index in 0..<array.count { pointer[index] = name == "weights" ? 1 : 0.125 }
        features[name] = MLFeatureValue(multiArray: array)
    }
    phase("predicting \(args[1]) cpu batch=\(batch)")
    let output = try model.prediction(from: MLDictionaryFeatureProvider(dictionary: features))
    precondition(!output.featureNames.isEmpty)
    phase("prediction completed \(args[1])")
} catch {
    phase(String(describing: error))
    exit(1)
}
