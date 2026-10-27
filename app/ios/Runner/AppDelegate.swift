import CryptoKit
import Flutter
import Security
import UIKit
import UserNotifications

@main
@objc class AppDelegate: FlutterAppDelegate, FlutterImplicitEngineDelegate {
  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  func didInitializeImplicitFlutterEngine(_ engineBridge: FlutterImplicitEngineBridge) {
    GeneratedPluginRegistrant.register(with: engineBridge.pluginRegistry)
    if let registrar = engineBridge.pluginRegistry.registrar(forPlugin: "ZelamarPlatform") {
      ZelamarPlatform.register(with: registrar)
    }
  }
}

/// What the Dart side asks of iOS, on the same channels as on Android:
/// - the master key, sealed with a key kept in the Keychain on this device
///   only ("zelamar/keystore", lib/src/messenger/os_vault.dart). Sealed
///   bytes are `nonce (12) || ciphertext || tag (16)`, as on Android;
/// - notifications for new messages ("zelamar/notify");
/// - copying a secret that stays on this device and soon expires
///   ("zelamar/clipboard").
enum ZelamarPlatform {
  static func register(with registrar: FlutterPluginRegistrar) {
    let messenger = registrar.messenger()

    FlutterMethodChannel(name: "zelamar/keystore", binaryMessenger: messenger)
      .setMethodCallHandler { call, result in
        guard
          let arguments = call.arguments as? [String: Any],
          let alias = arguments["alias"] as? String,
          let data = (arguments["data"] as? FlutterStandardTypedData)?.data
        else {
          result(FlutterError(code: "keystore", message: "expected alias and data", details: nil))
          return
        }
        do {
          switch call.method {
          case "seal":
            result(FlutterStandardTypedData(bytes: try Keystore.seal(data, alias: alias)))
          case "open":
            result(FlutterStandardTypedData(bytes: try Keystore.open(data, alias: alias)))
          default:
            result(FlutterMethodNotImplemented)
          }
        } catch {
          result(FlutterError(code: "keystore", message: "\(error)", details: nil))
        }
      }

    FlutterMethodChannel(name: "zelamar/clipboard", binaryMessenger: messenger)
      .setMethodCallHandler { call, result in
        guard call.method == "copySecret", let text = call.arguments as? String else {
          result(FlutterMethodNotImplemented)
          return
        }
        // Never to other devices through Universal Clipboard, and gone in
        // two minutes.
        UIPasteboard.general.setItems(
          [["public.utf8-plain-text": text]],
          options: [.localOnly: true, .expirationDate: Date().addingTimeInterval(120)]
        )
        result(true)
      }

    FlutterMethodChannel(name: "zelamar/notify", binaryMessenger: messenger)
      .setMethodCallHandler { call, result in
        switch call.method {
        case "inFront":
          result(UIApplication.shared.applicationState == .active)
        case "requestPermission":
          UNUserNotificationCenter.current()
            .requestAuthorization(options: [.alert, .sound, .badge]) { _, _ in }
          result(nil)
        case "show":
          let arguments = call.arguments as? [String: Any]
          let content = UNMutableNotificationContent()
          content.title = arguments?["title"] as? String ?? "Zelamar"
          content.body = arguments?["body"] as? String ?? ""
          content.sound = .default
          UNUserNotificationCenter.current().add(
            UNNotificationRequest(identifier: "message", content: content, trigger: nil)
          )
          result(nil)
        default:
          result(FlutterMethodNotImplemented)
        }
      }
  }
}

/// The key that seals Zelamar's master key: 256 random bits in the
/// Keychain, readable after the first unlock, never synced or backed up.
enum Keystore {
  private static let service = "com.zelamar.app.keystore"

  enum Failure: Error {
    case missing(String)
    case keychain(OSStatus)
  }

  static func seal(_ secret: Data, alias: String) throws -> Data {
    let box = try AES.GCM.seal(secret, using: key(alias: alias, create: true))
    guard let combined = box.combined else {
      throw Failure.missing("sealed box")
    }
    return combined
  }

  static func open(_ sealed: Data, alias: String) throws -> Data {
    try AES.GCM.open(AES.GCM.SealedBox(combined: sealed), using: key(alias: alias, create: false))
  }

  /// The existing key for `alias`, creating it only when sealing.
  private static func key(alias: String, create: Bool) throws -> SymmetricKey {
    let query: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: service,
      kSecAttrAccount as String: alias,
      kSecReturnData as String: true,
      kSecMatchLimit as String: kSecMatchLimitOne,
    ]
    var found: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &found)
    if status == errSecSuccess, let data = found as? Data {
      return SymmetricKey(data: data)
    }
    guard status == errSecItemNotFound else {
      throw Failure.keychain(status)
    }
    guard create else {
      throw Failure.missing("no key named \(alias) on this device")
    }
    let key = SymmetricKey(size: .bits256)
    let add: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: service,
      kSecAttrAccount as String: alias,
      kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
      kSecValueData as String: key.withUnsafeBytes { Data($0) },
    ]
    let added = SecItemAdd(add as CFDictionary, nil)
    guard added == errSecSuccess else {
      throw Failure.keychain(added)
    }
    return key
  }
}
