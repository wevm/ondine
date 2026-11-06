// swift-tools-version:6.0
import PackageDescription

// The Swift package renders with Ondine's WebAssembly modules through WasmKit's interpreter.
// SwiftPM reads this manifest from the repository root, so its sources live under `swift/`.
let package = Package(
  name: "Ondine",
  platforms: [.iOS(.v18), .macOS(.v15)],
  products: [.library(name: "Ondine", targets: ["Ondine"])],
  dependencies: [
    .package(url: "https://github.com/swiftwasm/WasmKit.git", .upToNextMinor(from: "0.4.2"))
  ],
  targets: [
    .target(
      name: "Ondine",
      dependencies: [.product(name: "WasmKit", package: "WasmKit")],
      path: "swift/Sources/Ondine"
    ),
    .testTarget(name: "OndineTests", dependencies: ["Ondine"], path: "swift/Tests/OndineTests"),
  ]
)
