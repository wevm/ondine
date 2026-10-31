/// Renders Mermaid source to monochrome SVG with a caller-supplied Ondine WebAssembly module.
///
/// The package mirrors `ondine/compiler/wasm`. The caller supplies a module, such as
/// `all.generated.wasm` from the npm package, and `Ondine.Compiler` renders source with it.
/// WasmKit interprets the module, so rendering needs no JIT and runs on iOS.
///
/// ```swift
/// // Renders a flowchart with the module for every diagram type
/// let compiler = try Ondine.Compiler(module: Data(contentsOf: url))
/// let rendered = try compiler.render("flowchart LR\n  A --> B")
/// ```
public enum Ondine {}
