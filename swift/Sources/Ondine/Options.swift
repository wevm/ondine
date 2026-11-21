import Foundation

extension Ondine {
  /// Rendering options, as `render.Options` from `ondine/compiler/wasm`.
  ///
  /// A `nil` field uses the compiler's default. `fuel` and `now` are not sent as options.
  public struct Options: Equatable, Sendable {
    /// The fuel budget when none is given: one billion units. It stops a render after about one
    /// second of interpretation in an optimized build. Every sample in `test/vectors/samples.json`
    /// uses less than ten million units. Pass a larger `fuel` to render larger diagrams.
    public static let defaultFuel: UInt64 = 1_000_000_000

    /// Lets a diagram that runs across be drawn running down when it does not fit `maxWidth`.
    /// Defaults to `true`.
    public var autoDirection: Bool?
    /// The width under which `autoDirection` turns a diagram. Defaults to `640`.
    public var directionBreakpoint: Double?
    /// Font size in pixels. Defaults to `16`.
    public var fontSize: Double?
    /// The most WebAssembly fuel one render may use. One unit is about one executed instruction.
    /// When a render uses all of it, `render` throws `RenderError.fuelExhausted`.
    /// Defaults to `defaultFuel`.
    public var fuel: UInt64
    /// Prefix for element IDs, to keep several diagrams on one page apart.
    public var idPrefix: String?
    /// The most characters of source to accept. Defaults to `50000`.
    public var limit: Int?
    /// The most edges a flowchart may have. Defaults to `5000`.
    public var maxEdges: Int?
    /// The width available to the diagram, in pixels.
    public var maxWidth: Double?
    /// The time a gantt chart schedules against. Defaults to the current time.
    public var now: Date?
    /// The most characters of SVG to produce. Defaults to `4000000`.
    public var outputLimit: Int?
    /// Padding around the diagram, in pixels. Defaults to `8`.
    public var padding: Double?
    /// Whether the SVG scales to its container. Defaults to `true`.
    public var responsive: Bool?

    /// Creates options. Every parameter is optional.
    public init(
      autoDirection: Bool? = nil,
      directionBreakpoint: Double? = nil,
      fontSize: Double? = nil,
      fuel: UInt64 = Options.defaultFuel,
      idPrefix: String? = nil,
      limit: Int? = nil,
      maxEdges: Int? = nil,
      maxWidth: Double? = nil,
      now: Date? = nil,
      outputLimit: Int? = nil,
      padding: Double? = nil,
      responsive: Bool? = nil
    ) {
      self.autoDirection = autoDirection
      self.directionBreakpoint = directionBreakpoint
      self.fontSize = fontSize
      self.fuel = fuel
      self.idPrefix = idPrefix
      self.limit = limit
      self.maxEdges = maxEdges
      self.maxWidth = maxWidth
      self.now = now
      self.outputLimit = outputLimit
      self.padding = padding
      self.responsive = responsive
    }

    /// The options as the compiler reads them: the JSON the TypeScript wrapper sends, or empty
    /// for none. As in `JSON.stringify`, a number that is not finite becomes `null`.
    func json() -> String {
      var fields: [String] = []
      if let autoDirection { fields.append(#""autoDirection":\#(autoDirection)"#) }
      if let directionBreakpoint { fields.append(#""directionBreakpoint":"# + number(directionBreakpoint)) }
      if let fontSize { fields.append(#""fontSize":"# + number(fontSize)) }
      if let idPrefix { fields.append(#""idPrefix":"# + string(idPrefix)) }
      if let limit { fields.append(#""limit":\#(limit)"#) }
      if let maxEdges { fields.append(#""maxEdges":\#(maxEdges)"#) }
      if let maxWidth { fields.append(#""maxWidth":"# + number(maxWidth)) }
      if let outputLimit { fields.append(#""outputLimit":\#(outputLimit)"#) }
      if let padding { fields.append(#""padding":"# + number(padding)) }
      if let responsive { fields.append(#""responsive":\#(responsive)"#) }
      return fields.isEmpty ? "" : "{" + fields.joined(separator: ",") + "}"
    }
  }
}

/// A JSON number, or `null` when `value` is not finite.
private func number(_ value: Double) -> String {
  guard value.isFinite else { return "null" }
  if value == value.rounded(), abs(value) < 1e15 { return String(Int64(value)) }
  return String(value)
}

/// A JSON string literal.
private func string(_ value: String) -> String {
  var out = "\""
  for scalar in value.unicodeScalars {
    switch scalar {
    case "\"": out += "\\\""
    case "\\": out += "\\\\"
    case "\n": out += "\\n"
    case "\r": out += "\\r"
    case "\t": out += "\\t"
    case _ where scalar.value < 0x20:
      let hex = String(scalar.value, radix: 16)
      out += "\\u" + String(repeating: "0", count: 4 - hex.count) + hex
    default: out.unicodeScalars.append(scalar)
    }
  }
  return out + "\""
}
