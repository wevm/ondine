import Foundation
import Synchronization
import WasmKit

extension Ondine {
  /// Renders Mermaid source through a caller-supplied WebAssembly module.
  ///
  /// The module is `all.generated.wasm` for every diagram type, or the module of one type, which
  /// renders only that type. A compiler instantiates the module on first use and keeps the
  /// instance for later renders. After a trap, the next render uses a fresh instance.
  ///
  /// A compiler is `Sendable`. A lock serializes its renders, so concurrent calls on one compiler
  /// run one at a time. Create one compiler per thread to render in parallel.
  public final class Compiler: Sendable {
    /// The parsed module and its instance, behind the lock.
    private let state: Mutex<State>

    /// Creates a compiler from one module.
    ///
    /// - Parameter module: The bytes of an Ondine WebAssembly module, such as `all.generated.wasm`
    ///   for every diagram type. The module of one type, such as `flowchart.generated.wasm`,
    ///   renders only that type and throws an `unsupportedDiagram` diagnostic for others.
    /// - Throws: `RenderError.invalidModule` when the bytes are not a WebAssembly module.
    public init(module: Data) throws(RenderError) {
      let parsed: Module
      do {
        parsed = try parseWasm(bytes: Array(module))
      } catch {
        throw .invalidModule("The module does not parse: \(error)")
      }

      state = Mutex(State(module: parsed))
    }

    /// Renders `source` with the module.
    ///
    /// Rendering is CPU-bound and synchronous. Call it off the main thread for large diagrams.
    ///
    /// - Parameters:
    ///   - source: The Mermaid source.
    ///   - options: Rendering options. Defaults to the compiler's defaults.
    /// - Returns: The SVG document, its detected type, size, and links.
    /// - Throws: `RenderError.diagnostic` when the source cannot be rendered,
    ///   `RenderError.fuelExhausted` when the render uses all of `options.fuel`, and
    ///   `RenderError.trap` or `RenderError.invalidModule` when the module fails.
    public func render(_ source: String, options: Options = .init()) throws(RenderError) -> Rendered {
      let input = Input(
        fuel: options.fuel,
        // The compiler reads no clock, so the time is passed apart from the options.
        now: (options.now ?? Date()).timeIntervalSince1970 * 1000,
        options: Array(options.json().utf8),
        source: Array(source.utf8)
      )

      return try state.withLock { (state: inout State) throws(RenderError) -> Rendered in
        try state.render(input)
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

/// The parsed module and the instance made from it.
private struct State {
  var instance: Instance?
  let module: Module

  /// Renders `input`, instantiating the module on first use.
  mutating func render(_ input: Input) throws(Ondine.RenderError) -> Ondine.Rendered {
    let current: Instance
    if let instance {
      current = instance
    } else {
      current = try Instance(module: module)
      instance = current
    }

    do throws(Ondine.RenderError) {
      return try current.render(input)
    } catch .diagnostic(let diagnostic) {
      // The module returns a diagnostic without a trap, so the instance stays usable.
      throw .diagnostic(diagnostic)
    } catch {
      // A trap can leave linear memory inconsistent, so the next render instantiates again.
      instance = nil
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
  func render(_ input: Input) throws(Ondine.RenderError) -> Ondine.Rendered {
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
