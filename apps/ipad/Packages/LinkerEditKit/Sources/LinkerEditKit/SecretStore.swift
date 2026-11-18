import Foundation
import Security

/// Where sign-ins and sessions are kept: the Keychain in the app, memory in tests.
public protocol SecretStore: AnyObject, Sendable {
    func read(_ key: String) -> Data?
    func write(_ data: Data, for key: String)
    func delete(_ key: String)
}

/// Generic passwords in this device's Keychain, readable after the first unlock and never synced.
public final class KeychainStore: SecretStore, @unchecked Sendable {
    private let service: String

    public init(service: String = "cool.linker.edit.ipad") {
        self.service = service
    }

    private func query(_ key: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: key,
        ]
    }

    public func read(_ key: String) -> Data? {
        var request = query(key)
        request[kSecReturnData as String] = true
        request[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: AnyObject?
        guard SecItemCopyMatching(request as CFDictionary, &result) == errSecSuccess else { return nil }
        return result as? Data
    }

    public func write(_ data: Data, for key: String) {
        SecItemDelete(query(key) as CFDictionary)
        var item = query(key)
        item[kSecValueData as String] = data
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        SecItemAdd(item as CFDictionary, nil)
    }

    public func delete(_ key: String) {
        SecItemDelete(query(key) as CFDictionary)
    }
}

public final class MemorySecretStore: SecretStore, @unchecked Sendable {
    private let lock = NSLock()
    private var values: [String: Data] = [:]

    public init() {}

    public var keys: [String] {
        lock.lock()
        defer { lock.unlock() }
        return Array(values.keys)
    }

    public func read(_ key: String) -> Data? {
        lock.lock()
        defer { lock.unlock() }
        return values[key]
    }

    public func write(_ data: Data, for key: String) {
        lock.lock()
        values[key] = data
        lock.unlock()
    }

    public func delete(_ key: String) {
        lock.lock()
        values[key] = nil
        lock.unlock()
    }
}
