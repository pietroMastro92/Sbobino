import Cocoa

func cropAndRoundImage(inputPath: String, outputPath: String) {
    guard let image = NSImage(contentsOfFile: inputPath) else {
        print("Failed to load image at \(inputPath)")
        exit(1)
    }
    
    var rect = NSRect(origin: .zero, size: image.size)
    guard let cgImage = image.cgImage(forProposedRect: &rect, context: nil, hints: nil) else {
        print("Failed to get CGImage")
        exit(1)
    }
    
    let width = CGFloat(cgImage.width)
    let height = CGFloat(cgImage.height)
    
    // Icon takes up about 82% of the center in typical generations
    let cropSize: CGFloat = width * 0.82
    
    let cropRect = NSRect(
        x: (width - cropSize) / 2,
        y: (height - cropSize) / 2,
        width: cropSize,
        height: cropSize
    )
    
    let colorSpace = CGColorSpaceCreateDeviceRGB()
    let bitmapInfo = CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue)
    
    guard let context = CGContext(
        data: nil,
        width: Int(cropSize),
        height: Int(cropSize),
        bitsPerComponent: 8,
        bytesPerRow: 0,
        space: colorSpace,
        bitmapInfo: bitmapInfo.rawValue
    ) else {
        print("Failed to create context")
        exit(1)
    }
    
    let radius = cropSize * 0.225
    let path = CGPath(roundedRect: CGRect(origin: .zero, size: CGSize(width: cropSize, height: cropSize)), cornerWidth: radius, cornerHeight: radius, transform: nil)
    
    context.addPath(path)
    context.clip()
    
    let drawRect = CGRect(
        x: -cropRect.origin.x,
        y: -cropRect.origin.y,
        width: width,
        height: height
    )
    
    context.draw(cgImage, in: drawRect)
    
    guard let outputCGImage = context.makeImage() else {
        print("Failed to create output image")
        exit(1)
    }
    
    let outputImage = NSImage(cgImage: outputCGImage, size: CGSize(width: cropSize, height: cropSize))
    guard let tiffData = outputImage.tiffRepresentation,
          let bitmapRep = NSBitmapImageRep(data: tiffData),
          let pngData = bitmapRep.representation(using: .png, properties: [:]) else {
        print("Failed to create PNG data")
        exit(1)
    }
    
    let url = URL(fileURLWithPath: outputPath)
    do {
        try pngData.write(to: url)
        print("Successfully saved to \(outputPath)")
    } catch {
        print("Error saving: \(error)")
        exit(1)
    }
}

let args = CommandLine.arguments
if args.count < 3 {
    print("Usage: swift crop_icon.swift <input> <output>")
    exit(1)
}

cropAndRoundImage(inputPath: args[1], outputPath: args[2])
