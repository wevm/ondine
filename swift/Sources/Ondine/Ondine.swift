/// Renders Mermaid source to monochrome SVG with caller-supplied Ondine WebAssembly modules.
///
/// The package mirrors `ondine/compiler/wasm`. The caller supplies the modules, such as
/// `all.generated.wasm` from the npm package, and `Ondine.Compiler` renders source with them.
/// WasmKit interprets the modules, so rendering needs no JIT and runs on iOS.
///
/// ```swift
/// // Renders a flowchart with the module for every diagram type
/// let compiler = try Ondine.Compiler(module: Data(contentsOf: url))
/// let rendered = try compiler.render("flowchart LR\n  A --> B")
/// ```
public enum Ondine {}
