import Foundation

// The server adds fields and message types over time, and an iPad app can lag a release behind. So decoding
// here never fails on what it doesn't know: unknown fields are ignored, unknown values kept as strings,
// missing or mistyped fields fall back to a default, and one broken element doesn't sink its list.

/// A string from a closed set on the server ("claude" | "codex"), kept as is when it's a new one.
public protocol OpenEnum: Codable, Hashable, Sendable {
    var rawValue: String { get }
    init(rawValue: String)
}

extension OpenEnum {
    public init(from decoder: Decoder) throws {
        self.init(rawValue: try decoder.singleValueContainer().decode(String.self))
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(rawValue)
    }
}

struct Lossy<Value: Decodable>: Decodable {
    let value: Value?

    init(from decoder: Decoder) {
        value = try? Value(from: decoder)
    }
}

/// Any coding key, for reading a message's "type" or an item's "kind" before the rest.
struct AnyKey: CodingKey {
    var stringValue: String
    var intValue: Int? { nil }

    init(_ string: String) { stringValue = string }
    init?(stringValue: String) { self.stringValue = stringValue }
    init?(intValue: Int) { nil }
}

extension KeyedDecodingContainer {
    /// The value, or fallback when it's missing, null or of another type.
    func value<T: Decodable>(_ key: Key, _ fallback: T) -> T {
        let decoded = try? decodeIfPresent(T.self, forKey: key)
        return decoded ?? fallback
    }

    /// The value, or nil when it's missing, null or of another type.
    func optional<T: Decodable>(_ key: Key) -> T? {
        let decoded = try? decodeIfPresent(T.self, forKey: key)
        return decoded
    }

    /// The elements that decode; the others are left out.
    func list<T: Decodable>(_ key: Key) -> [T] {
        let decoded = try? decodeIfPresent([Lossy<T>].self, forKey: key)
        return decoded?.compactMap { $0.value } ?? []
    }
}

extension Decoder {
    func keyed() throws -> KeyedDecodingContainer<AnyKey> {
        try container(keyedBy: AnyKey.self)
    }
}

extension KeyedDecodingContainer where Key == AnyKey {
    func value<T: Decodable>(_ key: String, _ fallback: T) -> T { value(AnyKey(key), fallback) }
    func optional<T: Decodable>(_ key: String) -> T? { optional(AnyKey(key)) }
    func list<T: Decodable>(_ key: String) -> [T] { list(AnyKey(key)) }

    /// A field the value can't be without: throws when it's missing.
    func required<T: Decodable>(_ key: String) throws -> T {
        try decode(T.self, forKey: AnyKey(key))
    }
}
