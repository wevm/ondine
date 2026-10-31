import Foundation
import Ondine
import Testing

/// `all.generated.wasm`, which `pnpm build:wasm` writes to `src/internal/`.
private let module = URL(filePath: #filePath)
  .deletingLastPathComponent()
  .appending(path: "../../../src/internal/all.generated.wasm")
  .standardizedFileURL

private let missing = "Run pnpm build:wasm to write src/internal/all.generated.wasm."

@Suite("Compiler.render", .enabled(if: FileManager.default.fileExists(atPath: module.path), Comment(rawValue: missing)))
struct CompilerTests {
  let compiler: Ondine.Compiler

  init() throws {
    compiler = try Ondine.Compiler(module: Data(contentsOf: module))
  }

  @Test("renders a flowchart")
  func flowchart() throws {
    let rendered = try compiler.render("flowchart LR\n  A --> B", options: .init(idPrefix: "d"))

    #expect(rendered.type == "flowchart")
    #expect(rendered.svg.hasPrefix("<svg"))
    #expect(rendered.width > 0)
    #expect(rendered.height > 0)
    #expect(rendered.links.isEmpty)
  }

  @Test("returns links")
  func links() throws {
    let rendered = try compiler.render(
      "flowchart LR\n  A --> B\n  click A \"https://example.com\""
    )

    // Values match render from ondine/compiler/wasm for the same source.
    #expect(rendered.links.map(\.href) == ["https://example.com/"])
    #expect(rendered.links.map(\.id) == ["A"])
    #expect(rendered.links.map(\.isInternal) == [false])
  }

  @Test("throws a diagnostic for a syntax error")
  func syntax() throws {
    let error = #expect(throws: Ondine.RenderError.self) {
      try compiler.render("flowchart LR\n  A --> B --> ]")
    }

    guard case .diagnostic(let diagnostic)? = error else {
      Issue.record("Expected a diagnostic, got \(String(describing: error))")
      return
    }
    // Values match RenderError from ondine/compiler/wasm for the same source.
    #expect(diagnostic.code == .syntax)
    #expect(diagnostic.type == "flowchart")
    #expect(diagnostic.line == 2)
    #expect(diagnostic.column == 15)
    #expect(diagnostic.snippet == "  A --> B --> ]\n--------------^")
    #expect(
      diagnostic.message == "Lexical error on line 2. Unrecognized text.\n  A --> B --> ]\n--------------^"
    )
  }

  @Test("throws fuelExhausted for a tiny budget, then renders")
  func fuel() throws {
    #expect(throws: Ondine.RenderError.fuelExhausted) {
      try compiler.render("flowchart LR\n  A --> B", options: .init(fuel: 1_000))
    }

    let rendered = try compiler.render("flowchart LR\n  A --> B")
    #expect(rendered.type == "flowchart")
  }

  @Test("sends options to the compiler")
  func options() throws {
    let source = "flowchart LR\n  A --> B"
    let wide = try compiler.render(source, options: .init(padding: 8))
    let padded = try compiler.render(source, options: .init(padding: 40))

    // render from ondine/compiler/wasm returns widths 160 and 224 for the same source.
    #expect(wide.width == 160)
    #expect(padded.width == 224)
    #expect(wide.svg.contains("max-width:100%"))
    #expect(!(try compiler.render(source, options: .init(responsive: false)).svg.contains("max-width:100%")))
  }

  @Test("throws a limit diagnostic")
  func limit() throws {
    let error = #expect(throws: Ondine.RenderError.self) {
      try compiler.render("flowchart LR\n  A --> B", options: .init(limit: 4))
    }

    guard case .diagnostic(let diagnostic)? = error else {
      Issue.record("Expected a diagnostic, got \(String(describing: error))")
      return
    }
    #expect(diagnostic.code == .limit)
  }
}

@Suite("Compiler.init")
struct CompilerInitTests {
  @Test("rejects bytes that are not a module")
  func invalid() {
    #expect {
      try Ondine.Compiler(module: Data("not wasm".utf8))
    } throws: { error in
      guard case .invalidModule? = error as? Ondine.RenderError else { return false }
      return true
    }
  }
}
