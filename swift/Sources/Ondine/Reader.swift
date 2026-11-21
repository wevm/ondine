/// Decodes the result of the module's `render` export: a status byte, then fields that are each
/// a little-endian `u32` length and its bytes.
struct Reader {
  let bytes: UnsafeRawBufferPointer
  private var at = 0

  init(bytes: UnsafeRawBufferPointer) {
    self.bytes = bytes
  }

  /// Decodes the result at the start of `bytes`. A rejection throws `RenderError.diagnostic`.
  mutating func decode() throws(Ondine.RenderError) -> Ondine.Rendered {
    guard let status = bytes.first else { throw malformed() }
    at = 1

    // Field order is the module's wire format.
    if status == 1 {
      let code = try text()
      let type = try text()
      let line = try u32()
      let column = try u32()
      let snippet = try text()
      let message = try text()

      throw .diagnostic(
        Ondine.Diagnostic(
          code: Ondine.Diagnostic.Code(rawValue: code),
          column: column,
          line: line,
          message: message,
          snippet: snippet,
          type: type.isEmpty ? nil : type
        )
      )
    }

    let type = try text()
    let width = try f64()
    let height = try f64()
    let count = try u32()
    var links: [Ondine.Link] = []
    for _ in 0..<count {
      let href = try text()
      let id = try text()
      let isInternal = try field().first == 1
      links.append(Ondine.Link(href: href, id: id, isInternal: isInternal))
    }
    let svg = try text()

    return Ondine.Rendered(height: height, links: links, svg: svg, type: type, width: width)
  }

  /// The next field's bytes.
  private mutating func field() throws(Ondine.RenderError) -> UnsafeRawBufferPointer {
    guard at + 4 <= bytes.count else { throw malformed() }
    let length = Int(bytes.loadUnaligned(fromByteOffset: at, as: UInt32.self).littleEndian)
    at += 4

    guard length <= bytes.count - at else { throw malformed() }
    defer { at += length }
    return UnsafeRawBufferPointer(rebasing: bytes[at..<at + length])
  }

  /// The next field as a little-endian `f64`.
  private mutating func f64() throws(Ondine.RenderError) -> Double {
    let value = try field()
    guard value.count >= 8 else { throw malformed() }
    return Double(bitPattern: value.loadUnaligned(as: UInt64.self).littleEndian)
  }

  /// The next field as UTF-8 text.
  private mutating func text() throws(Ondine.RenderError) -> String {
    String(decoding: try field(), as: UTF8.self)
  }

  /// The next field as a little-endian `u32`.
  private mutating func u32() throws(Ondine.RenderError) -> Int {
    let value = try field()
    guard value.count >= 4 else { throw malformed() }
    return Int(value.loadUnaligned(as: UInt32.self).littleEndian)
  }

  private func malformed() -> Ondine.RenderError {
    .trap("The module returned a malformed result.")
  }
}
