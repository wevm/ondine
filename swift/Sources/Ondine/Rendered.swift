extension Ondine {
  /// A rendered diagram, as `render.ReturnType` from `ondine/compiler/wasm`.
  public struct Rendered: Hashable, Sendable {
    /// Height in pixels.
    public let height: Double
    /// Links in drawing order.
    public let links: [Link]
    /// The SVG document.
    public let svg: String
    /// The detected diagram type, such as `flowchart` or `sequence`.
    public let type: String
    /// Width in pixels.
    public let width: Double
  }

  /// A link drawn in a diagram.
  public struct Link: Hashable, Sendable {
    /// The link target.
    public let href: String
    /// The ID of the element that carries the link.
    public let id: String
    /// Whether the link targets an element of the same diagram, as `internal` in TypeScript.
    public let isInternal: Bool
  }
}
