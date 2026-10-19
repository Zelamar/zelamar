import 'dart:io';

import 'package:flutter/services.dart';

/// Shows notifications about new messages through the platform: a toast
/// and a flashing taskbar button on Windows (`windows/runner/notifier.cpp`),
/// a system notification on Android (`MainActivity.kt`).
abstract interface class Notifier {
  /// The notifier of the platform the app runs on, or a silent one.
  static Notifier forPlatform() => Platform.isWindows || Platform.isAndroid
      ? const PlatformNotifier()
      : const SilentNotifier();

  /// Shows a notification.
  Future<void> show({required String title, required String body});

  /// Whether the app is in front of the user right now: then the chat list
  /// and badges say enough, and nothing pops up.
  Future<bool> inFront();

  /// Asks for permission to notify, where the platform requires it.
  Future<void> requestPermission();

  /// The token that wakes this app while it is closed (ADR-0017), or `null`
  /// where there is no such thing.
  Future<String?> pushToken();

  /// Whether the platform starts Zelamar in the tray at sign-in; `null`
  /// where that is not a thing (everywhere but Windows).
  Future<bool?> startsAtSignIn();

  /// Turns starting at sign-in on or off; returns the new state.
  Future<bool?> setStartsAtSignIn({required bool on});
}

/// The native notifier, over the `zelamar/notify` channel. A notification
/// that cannot be shown is not worth failing anything for.
class PlatformNotifier implements Notifier {
  const new();

  static const _channel = MethodChannel('zelamar/notify');
  static const _push = MethodChannel('zelamar/push');

  @override
  Future<bool?> startsAtSignIn() async {
    if (!Platform.isWindows) {
      return null;
    }
    try {
      return await _channel.invokeMethod<bool>('startsAtSignIn');
    } on PlatformException {
      return null;
    } on MissingPluginException {
      return null;
    }
  }

  @override
  Future<bool?> setStartsAtSignIn({required bool on}) async {
    if (!Platform.isWindows) {
      return null;
    }
    try {
      return await _channel.invokeMethod<bool>('setStartsAtSignIn', on);
    } on PlatformException {
      return null;
    } on MissingPluginException {
      return null;
    }
  }

  @override
  Future<String?> pushToken() async {
    if (!Platform.isAndroid) {
      return null;
    }
    try {
      return await _push.invokeMethod<String>('token');
    } on PlatformException {
      return null;
    } on MissingPluginException {
      return null;
    }
  }

  @override
  Future<void> show({required String title, required String body}) async {
    try {
      await _channel.invokeMethod<void>('show', {'title': title, 'body': body});
    } on PlatformException {
      // No notification this time.
    } on MissingPluginException {
      // Not available in this build.
    }
  }

  @override
  Future<bool> inFront() async {
    try {
      return await _channel.invokeMethod<bool>('inFront') ?? true;
    } on PlatformException {
      return true;
    } on MissingPluginException {
      return true;
    }
  }

  @override
  Future<void> requestPermission() async {
    if (!Platform.isAndroid) {
      return;
    }
    try {
      await _channel.invokeMethod<void>('requestPermission');
    } on PlatformException {
      // The user can still allow notifications in system settings.
    } on MissingPluginException {
      // Not available in this build.
    }
  }
}

/// Never notifies: tests, and platforms without a notifier yet.
class SilentNotifier implements Notifier {
  const new();

  @override
  Future<void> show({required String title, required String body}) async {}

  @override
  Future<bool> inFront() async => true;

  @override
  Future<void> requestPermission() async {}

  @override
  Future<String?> pushToken() async => null;

  @override
  Future<bool?> startsAtSignIn() async => null;

  @override
  Future<bool?> setStartsAtSignIn({required bool on}) async => null;
}
