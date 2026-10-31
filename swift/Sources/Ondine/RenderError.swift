import Foundation

extension Ondine {
  /// Thrown when `Compiler` cannot load a module or render source.
  public enum RenderError: Error, Hashable, Sendable {
    /// The compiler rejected the source. The diagnostic matches `RenderError` from
    /// `ondine/compiler/wasm` and the diagnostics of Mermaid.
    case diagnostic(Diagnostic)
    /// The render used all of `Options.fuel`. The next render uses a fresh instance.
    case fuelExhausted
    /// The bytes are not an Ondine module, or the module lacks an export.
    case invalidModule(String)
    /// The module trapped or returned a malformed result. The next render uses a fresh instance.
    case trap(String)
  }

  /// A diagnostic for source that cannot be rendered. Messages match Mermaid's diagnostics.
  public struct Diagnostic: Hashable, Sendable, CustomStringConvertible {
    /// The kind of failure.
    public let code: Code
    /// One-based UTF-16 column, or 0 when the diagnostic has no position.
    public let column: Int
    /// One-based line, or 0 when the diagnostic has no position.
    public let line: Int
    /// The diagnostic message.
    public let message: String
    /// The offending source line with a caret under the column, or empty.
    public let snippet: String
    /// The detected diagram type, when known.
    public let type: String?

    /// The message.
    public var description: String { message }

    /// The kind of failure, as `RenderError.Code` from `ondine/compiler/wasm`.
    public struct Code: Hashable, RawRepresentable, Sendable {
      /// The source or output exceeds a limit, such as `Options.limit`.
      public static let limit = Code(rawValue: "limit")
      /// The source parses but describes an invalid diagram.
      public static let semantic = Code(rawValue: "semantic")
      /// The source does not parse.
      public static let syntax = Code(rawValue: "syntax")
      /// The module does not include the detected diagram type.
      public static let unsupportedDiagram = Code(rawValue: "unsupported-diagram")

      /// The code as the module reports it, such as `syntax`.
      public let rawValue: String

      /// Creates a code from the module's value.
      public init(rawValue: String) {
        self.rawValue = rawValue
      }
    }
  }
}

extension Ondine.RenderError: LocalizedError {
  /// A message for display.
  public var errorDescription: String? {
    switch self {
    case .diagnostic(let diagnostic): diagnostic.message
    case .fuelExhausted: "The render used its whole fuel budget."
    case .invalidModule(let message), .trap(let message): message
    }
  }
}
