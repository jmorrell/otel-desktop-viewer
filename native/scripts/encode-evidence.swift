#!/usr/bin/env swift
import AVFoundation
import CoreGraphics
import CoreVideo
import Foundation
import ImageIO

func fail(_ message: String) -> Never {
    fputs("encode-evidence: \(message)\n", stderr)
    exit(2)
}

let arguments = Array(CommandLine.arguments.dropFirst())
guard arguments.count == 2 || arguments.count == 3 else {
    fail("usage: swift encode-evidence.swift FRAME_DIRECTORY OUTPUT.mp4 [FPS]")
}
let frameDirectory = URL(fileURLWithPath: arguments[0], isDirectory: true)
let outputURL = URL(fileURLWithPath: arguments[1])
let fps = arguments.count == 3 ? Int(arguments[2]) ?? 0 : 10
guard fps > 0 && fps <= 60 else { fail("FPS must be an integer from 1 to 60") }

do {
    let urls = try FileManager.default.contentsOfDirectory(
        at: frameDirectory,
        includingPropertiesForKeys: nil
    )
    .filter { $0.lastPathComponent.hasPrefix("frame-") && $0.pathExtension.lowercased() == "png" }
    .sorted { $0.lastPathComponent < $1.lastPathComponent }
    guard !urls.isEmpty else { fail("no frame-*.png files in \(frameDirectory.path)") }

    func image(_ url: URL) -> CGImage? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        return CGImageSourceCreateImageAtIndex(source, 0, nil)
    }

    guard let first = image(urls[0]) else { fail("cannot read \(urls[0].lastPathComponent)") }
    let width = first.width
    let height = first.height
    for url in urls.dropFirst() {
        guard let frame = image(url), frame.width == width, frame.height == height else {
            fail("all PNG frames must be readable and have the same pixel dimensions")
        }
    }

    try FileManager.default.createDirectory(
        at: outputURL.deletingLastPathComponent(),
        withIntermediateDirectories: true
    )
    try? FileManager.default.removeItem(at: outputURL)
    let writer = try AVAssetWriter(outputURL: outputURL, fileType: .mp4)
    writer.metadata = {
        let item = AVMutableMetadataItem()
        item.identifier = .commonIdentifierDescription
        item.value = "Offscreen Metal-rendered native UI frames; not an OS desktop screen recording." as NSString
        item.dataType = kCMMetadataBaseDataType_UTF8 as String
        return [item]
    }()

    let settings: [String: Any] = [
        AVVideoCodecKey: AVVideoCodecType.h264,
        AVVideoWidthKey: width,
        AVVideoHeightKey: height,
        AVVideoCompressionPropertiesKey: [
            AVVideoAverageBitRateKey: max(2_000_000, width * height * fps / 12),
            AVVideoExpectedSourceFrameRateKey: fps,
            AVVideoMaxKeyFrameIntervalKey: fps * 2,
        ],
    ]
    let input = AVAssetWriterInput(mediaType: .video, outputSettings: settings)
    input.expectsMediaDataInRealTime = false
    let attributes: [String: Any] = [
        kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
        kCVPixelBufferWidthKey as String: width,
        kCVPixelBufferHeightKey as String: height,
        kCVPixelBufferCGImageCompatibilityKey as String: true,
        kCVPixelBufferCGBitmapContextCompatibilityKey as String: true,
    ]
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(
        assetWriterInput: input,
        sourcePixelBufferAttributes: attributes
    )
    guard writer.canAdd(input) else { fail("H.264 video input is not supported") }
    writer.add(input)
    guard writer.startWriting() else { fail(writer.error?.localizedDescription ?? "cannot start video writer") }
    writer.startSession(atSourceTime: .zero)

    for (index, url) in urls.enumerated() {
        while !input.isReadyForMoreMediaData {
            Thread.sleep(forTimeInterval: 0.002)
        }
        guard let cgImage = image(url) else { fail("cannot read \(url.lastPathComponent)") }
        var optionalBuffer: CVPixelBuffer?
        guard let pool = adaptor.pixelBufferPool,
              CVPixelBufferPoolCreatePixelBuffer(kCFAllocatorDefault, pool, &optionalBuffer) == kCVReturnSuccess,
              let buffer = optionalBuffer else {
            fail("cannot allocate video frame buffer")
        }
        CVPixelBufferLockBaseAddress(buffer, [])
        defer { CVPixelBufferUnlockBaseAddress(buffer, []) }
        guard let context = CGContext(
            data: CVPixelBufferGetBaseAddress(buffer),
            width: width,
            height: height,
            bitsPerComponent: 8,
            bytesPerRow: CVPixelBufferGetBytesPerRow(buffer),
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGBitmapInfo.byteOrder32Little.rawValue | CGImageAlphaInfo.premultipliedFirst.rawValue
        ) else { fail("cannot create frame drawing context") }
        // CGImage drawing already writes the PNG's rows in the order expected
        // by the video buffer. Applying a UIKit-style Y flip inverts the video.
        context.draw(cgImage, in: CGRect(x: 0, y: 0, width: width, height: height))
        let presentationTime = CMTime(value: CMTimeValue(index), timescale: CMTimeScale(fps))
        guard adaptor.append(buffer, withPresentationTime: presentationTime) else {
            fail(writer.error?.localizedDescription ?? "cannot append frame \(index)")
        }
        if (index + 1) % fps == 0 { fputs("encoded \(index + 1)/\(urls.count) frames\n", stderr) }
    }

    input.markAsFinished()
    let finished = DispatchSemaphore(value: 0)
    writer.finishWriting { finished.signal() }
    finished.wait()
    guard writer.status == .completed else {
        fail(writer.error?.localizedDescription ?? "video writer did not complete")
    }
    print("Wrote \(urls.count) frames at \(fps) fps to \(outputURL.path)")
} catch {
    fail(error.localizedDescription)
}
