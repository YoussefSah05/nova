// nova-apple-helper: Apple on-device speech and translation for Nova's
// benchmark. Every output line is one JSON object.
//
//   nova-apple-helper locales
//   nova-apple-helper asr <wav> <locale> [--realtime] [--install] [--fast]
//   nova-apple-helper translate <source> <target>   (lines on stdin)

import AVFoundation
import Foundation
import Speech
import Translation

func emit(_ object: [String: Any]) {
    if let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]),
       let line = String(data: data, encoding: .utf8) {
        print(line)
        fflush(stdout)
    }
}

func fail(_ message: String) -> Never {
    emit(["error": message])
    exit(1)
}

func locales() async {
    let supported = await SpeechTranscriber.supportedLocales.map { $0.identifier(.bcp47) }.sorted()
    let installed = await SpeechTranscriber.installedLocales.map { $0.identifier(.bcp47) }.sorted()
    var translation: [String: String] = [:]
    let availability = LanguageAvailability()
    for (a, b) in [("en", "fr"), ("fr", "en"), ("en", "ar"), ("ar", "en"), ("fr", "ar"), ("ar", "fr")] {
        let status = await availability.status(from: Locale.Language(identifier: a), to: Locale.Language(identifier: b))
        translation["\(a)-\(b)"] = switch status {
        case .installed: "installed"
        case .supported: "supported"
        case .unsupported: "unsupported"
        @unknown default: "unknown"
        }
    }
    emit(["speech_supported": supported, "speech_installed": installed, "translation": translation])
}

/// Stream a WAV through SpeechTranscriber. With `--realtime` the audio is fed
/// at playback speed, so result times are what a live viewer would see.
func asr(path: String, localeId: String, realtime: Bool, install: Bool, fast: Bool) async throws {
    let locale = Locale(identifier: localeId)
    let transcriber = SpeechTranscriber(
        locale: locale,
        transcriptionOptions: [],
        reportingOptions: fast ? [.volatileResults, .fastResults] : [.volatileResults],
        attributeOptions: [.audioTimeRange])
    if let request = try await AssetInventory.assetInstallationRequest(supporting: [transcriber]) {
        guard install else { fail("speech assets for \(localeId) are not installed (pass --install)") }
        let began = Date()
        try await request.downloadAndInstall()
        emit(["installed": localeId, "install_sec": Date().timeIntervalSince(began)])
    }
    let analyzer = SpeechAnalyzer(modules: [transcriber])
    let file = try AVAudioFile(forReading: URL(fileURLWithPath: path))
    guard let format = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [transcriber]) else {
        fail("no compatible audio format")
    }
    guard let converter = AVAudioConverter(from: file.processingFormat, to: format) else {
        fail("cannot convert \(file.processingFormat) to \(format)")
    }
    let (inputs, builder) = AsyncStream.makeStream(of: AnalyzerInput.self)
    let started = ContinuousClock.now
    func elapsed() -> Double {
        let d = ContinuousClock.now - started
        return Double(d.components.seconds) + Double(d.components.attoseconds) / 1e18
    }
    let collector = Task {
        for try await result in transcriber.results {
            // Per-word timings from the attributed runs (audioTimeRange).
            var words: [[Any]] = []
            for run in result.text.runs {
                let text = String(result.text[run.range].characters)
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                guard !text.isEmpty, let range = run.audioTimeRange else { continue }
                words.append([text, range.start.seconds, range.end.seconds])
            }
            emit([
                "t": elapsed(),
                "final": result.isFinal,
                "start": result.range.start.seconds,
                "end": result.range.end.seconds,
                "text": String(result.text.characters),
                "words": words,
            ])
        }
    }
    try await analyzer.start(inputSequence: inputs)

    // Feed 100 ms buffers, paced at real time when asked.
    let chunk = AVAudioFrameCount(file.processingFormat.sampleRate / 10)
    var fed = 0.0
    while true {
        guard let source = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: chunk) else { break }
        // Reading at the end of the file throws on some formats: end of input.
        if file.framePosition >= file.length { break }
        do {
            try file.read(into: source, frameCount: min(chunk, AVAudioFrameCount(file.length - file.framePosition)))
        } catch {
            break
        }
        if source.frameLength == 0 { break }
        let ratio = format.sampleRate / file.processingFormat.sampleRate
        let capacity = AVAudioFrameCount(Double(source.frameLength) * ratio) + 32
        guard let converted = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: capacity) else { break }
        var supplied = false
        var error: NSError?
        let status = converter.convert(to: converted, error: &error) { _, status in
            if supplied {
                status.pointee = .noDataNow
                return nil
            }
            supplied = true
            status.pointee = .haveData
            return source
        }
        if status == .error {
            fail("audio conversion failed: \(error?.localizedDescription ?? "unknown")")
        }
        if converted.frameLength > 0 {
            builder.yield(AnalyzerInput(buffer: converted))
        }
        fed += Double(source.frameLength) / file.processingFormat.sampleRate
        if realtime {
            let ahead = fed - elapsed()
            if ahead > 0 { try await Task.sleep(for: .seconds(ahead)) }
        }
    }
    builder.finish()
    do {
        try await analyzer.finalizeAndFinishThroughEndOfInput()
    } catch {
        emit(["warning": "finalize: \(error)"])
    }
    do {
        try await collector.value
    } catch {
        emit(["warning": "results: \(error)"])
    }
    emit(["done": true, "audio_sec": fed, "wall_sec": elapsed()])
}

func translate(source: String, target: String) async throws {
    let session = TranslationSession(
        installedSource: Locale.Language(identifier: source),
        target: Locale.Language(identifier: target))
    while let line = readLine() {
        let began = ContinuousClock.now
        let response = try await session.translate(line)
        let d = ContinuousClock.now - began
        let ms = Double(d.components.seconds) * 1000 + Double(d.components.attoseconds) / 1e15
        emit(["ms": ms, "text": response.targetText])
    }
}

let args = CommandLine.arguments
do {
    switch args.count > 1 ? args[1] : "" {
    case "locales":
        await locales()
    case "asr" where args.count >= 4:
        try await asr(path: args[2], localeId: args[3],
                      realtime: args.contains("--realtime"), install: args.contains("--install"),
                      fast: args.contains("--fast"))
    case "translate" where args.count >= 4:
        try await translate(source: args[2], target: args[3])
    default:
        fail("usage: locales | asr <wav> <locale> [--realtime] [--install] [--fast] | translate <src> <tgt>")
    }
} catch {
    fail("\(error)")
}
