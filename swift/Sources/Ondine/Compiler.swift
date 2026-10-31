import Foundation
import Synchronization
import WasmKit

extension Ondine {
  /// Renders Mermaid source through caller-supplied WebAssembly modules.
  ///
  /// Each module holds one diagram type, or `all` of them, as in `create` from
  /// `ondine/compiler/wasm`. A compiler instantiates each module on first use and keeps the
  /// instance for later renders. After a trap, the next render uses a fresh instance.
  ///
  /// A compiler is `Sendable`. A lock serializes its renders, so concurrent calls on one compiler
  /// run one at a time. Create one compiler per thread to render in parallel.
  public final class Compiler: Sendable {
    /// The diagram type of the module that renders first.
    private let first: String
    /// Parsed modules and their instances, behind the lock.
    private let state: Mutex<State>

    /// Creates a compiler from one module, such as `all.generated.wasm`.
    ///
    /// - Parameter module: The bytes of an Ondine WebAssembly module.
    /// - Throws: `RenderError.invalidModule` when the bytes are not a WebAssembly module.
    public convenience init(module: Data) throws(RenderError) {
      try self.init(modules: ["all": module])
    }

    /// Creates a compiler from modules by diagram type, such as `all` or `flowchart`.
    ///
    /// The `all` module renders first when present. Otherwise the first type in alphabetical
    /// order renders first. If that module does not include the detected diagram type, the module
    /// named after the type renders the source.
    ///
    /// - Parameter modules: The bytes of Ondine WebAssembly modules by diagram type.
    /// - Throws: `RenderError.invalidModule` when `modules` is empty or holds bytes that are not
    ///   a WebAssembly module.
    public init(modules: [String: Data]) throws(RenderError) {
      var parsed: [String: Module] = [:]
      for (type, bytes) in modules {
        do {
          parsed[type] = try parseWasm(bytes: Array(bytes))
        } catch {
          throw .invalidModule("The \(type) module does not parse: \(error)")
        }
      }

      guard let first = modules["all"] == nil ? modules.keys.sorted().first : "all" else {
        throw .invalidModule("Ondine.Compiler needs at least one module.")
      }
      self.first = first
      state = Mutex(State(modules: parsed))
    }

    /// Renders `source`, choosing the module for its diagram type.
    ///
    /// Rendering is CPU-bound and synchronous. Call it off the main thread for large diagrams.
    ///
    /// - Parameters:
    ///   - source: The Mermaid source.
    ///   - options: Rendering options. Defaults to the compiler's defaults.
    /// - Returns: The SVG document, its detected type, size, and links.
    /// - Throws: `RenderError.diagnostic` when the source cannot be rendered,
    ///   `RenderError.fuelExhausted` when the render uses all of `options.fuel`, and
    ///   `RenderError.trap` or `RenderError.invalidModule` when a module fails.
    public func render(_ source: String, options: Options = .init()) throws(RenderError) -> Rendered {
      let input = Input(
        fuel: options.fuel,
        // The compiler reads no clock, so the time is passed apart from the options.
        now: (options.now ?? Date()).timeIntervalSince1970 * 1000,
        options: Array(options.json().utf8),
        source: Array(source.utf8)
      )
      let first = first

      return try state.withLock { (state: inout State) throws(RenderError) -> Rendered in
        let outcome = try state.render(type: first, input: input)

        // A module reports the detected type when it does not include it.
        if case .rejected(let diagnostic) = outcome, diagnostic.code == .unsupportedDiagram,
          let type = diagnostic.type, type != first, state.modules[type] != nil
        {
          return try state.render(type: type, input: input).get()
        }
        return try outcome.get()
      }
    }
  }
}

/// The encoded arguments of one render.
private struct Input {
  let fuel: UInt64
  let now: Double
  let options: [UInt8]
  let source: [UInt8]
}

/// Parsed modules and the instances made from them.
private struct State {
  var instances: [String: Instance] = [:]
  let modules: [String: Module]

  /// Renders `input` with the module for `type`, instantiating it on first use.
  mutating func render(type: String, input: Input) throws(Ondine.RenderError) -> Outcome {
    guard let module = modules[type] else {
      throw .invalidModule("No module for the \(type) diagram type.")
    }

    let instance: Instance
    if let found = instances[type] {
      instance = found
    } else {
      instance = try Instance(module: module)
      instances[type] = instance
    }

    do throws(Ondine.RenderError) {
      return try instance.render(input)
    } catch {
      // A trap can leave linear memory inconsistent, so the next render instantiates again.
      instances[type] = nil
      throw error
    }
  }
}

/// One instance of a module with its exports.
private final class Instance {
  let alloc: Function
  let memory: Memory
  let render: Function
  let reset: Function
  let store: Store

  init(module: Module) throws(Ondine.RenderError) {
    var configuration = EngineConfiguration()
    configuration.fuelMetering = true
    store = Store(engine: Engine(configuration: configuration))

    let instance: WasmKit.Instance
    do {
      instance = try module.instantiate(store: store)
    } catch {
      throw .invalidModule("The module does not instantiate: \(error)")
    }

    guard let alloc = instance.exports[function: "alloc"],
      let memory = instance.exports[memory: "memory"],
      let render = instance.exports[function: "render"],
      let reset = instance.exports[function: "reset"]
    else { throw .invalidModule("The module lacks the alloc, memory, render, or reset export.") }

    self.alloc = alloc
    self.memory = memory
    self.render = render
    self.reset = reset
  }

  /// Copies the source and options into linear memory, renders, and decodes the result.
  func render(_ input: Input) throws(Ondine.RenderError) -> Outcome {
    store.fuel = Fuel(remaining: input.fuel)

    do {
      _ = try reset([])

      // Source and options share one allocation.
      let length = input.source.count + input.options.count
      guard let size = UInt32(exactly: length), let sourceLength = UInt32(exactly: input.source.count) else {
        throw Ondine.RenderError.trap("The source and options exceed 32-bit linear memory.")
      }
      let at = try pointer(alloc([.i32(size)]))
      guard Int(at) + length <= memory.byteCount else {
        throw Ondine.RenderError.trap("alloc returned memory out of bounds.")
      }
      if length > 0 {
        memory.withUnsafeMutableBufferPointer(offset: UInt(at), count: length) { buffer in
          buffer.copyBytes(from: input.source)
          UnsafeMutableRawBufferPointer(rebasing: buffer[input.source.count...]).copyBytes(from: input.options)
        }
      }

      let result = try pointer(
        render([
          .i32(at),
          .i32(sourceLength),
          .i32(at + sourceLength),
          .i32(size - sourceLength),
          .f64(input.now.bitPattern),
        ])
      )
      guard Int(result) < memory.byteCount else {
        throw Ondine.RenderError.trap("render returned a result out of bounds.")
      }

      return try memory.withUnsafeMutableBufferPointer(
        offset: UInt(result),
        count: memory.byteCount - Int(result)
      ) { buffer in
        var reader = Reader(bytes: UnsafeRawBufferPointer(buffer))
        return try reader.decode()
      }
    } catch let error as Ondine.RenderError {
      throw error
    } catch let trap as Trap {
      // WasmKit keeps the trap reason internal, so the description identifies fuel exhaustion.
      throw trap.description.hasPrefix("Trap: out of fuel") ? .fuelExhausted : .trap(trap.description)
    } catch {
      throw .trap(String(describing: error))
    }
  }

  /// The pointer an export returns.
  private func pointer(_ values: [Value]) throws(Ondine.RenderError) -> UInt32 {
    guard case .i32(let value)? = values.first else {
      throw .trap("The module returned no pointer.")
    }
    return value
  }
}
