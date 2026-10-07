// swift-tools-version: 6.0
// Out-of-process bridge to Apple's on-device engines (SpeechAnalyzer /
// SpeechTranscriber, Translation) for Nova's benchmark: JSON lines on stdout.
import PackageDescription

let package = Package(
    name: "nova-apple-helper",
    platforms: [.macOS("26.0")],
    targets: [
        .executableTarget(name: "nova-apple-helper", path: "Sources/nova-apple-helper")
    ]
)
