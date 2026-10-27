import 'dart:async';
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

  /// A call rings while Zelamar is out of sight: ring like a phone, over
  /// the lock screen where allowed (Android), or come to the front
  /// (Windows).
  Future<void> ring({required String title, required bool video});

  /// The call no longer rings.
  Future<void> stopRinging();

  /// A call goes on, or ended: on Android, the microphone (and camera)
  /// keep working while the screen is off.
  Future<void> callOngoing({
    required bool on,
    String title = '',
    bool video = false,
  });

  /// What the platform asks of calls: `accept` or `decline` (from a ringing
  /// call's notification), or `wake` (a call wake-up for a running app:
  /// fetch the mail now).
  Stream<String> get callActions;
}

/// The native notifier, over the `zelamar/notify` channel. A notification
/// that cannot be shown is not worth failing anything for.
class PlatformNotifier implements Notifier {
  const new();

  static const _channel = MethodChannel('zelamar/notify');
  static const _push = MethodChannel('zelamar/push');
  static const _calls = MethodChannel('zelamar/calls');
  static final _actions = StreamController<String>.broadcast();
  static bool _listening = false;

  @override
  Stream<String> get callActions {
    if (!_listening) {
      _listening = true;
      _calls.setMethodCallHandler((call) async => _actions.add(call.method));
    }
    return _actions.stream;
  }

  @override
  Future<void> ring({required String title, required bool video}) =>
      _quietly('ring', {'title': title, 'video': video});

  @override
  Future<void> stopRinging() =>
      Platform.isAndroid ? _quietly('stopRinging') : Future<void>.value();

  @override
  Future<void> callOngoing({
    required bool on,
    String title = '',
    bool video = false,
  }) => Platform.isAndroid
      ? _quietly('callOngoing', {'on': on, 'title': title, 'video': video})
      : Future<void>.value();

  /// Calls [method] on the native side; a failure only means the platform
  /// does less this time.
  Future<void> _quietly(String method, [Object? arguments]) async {
    try {
      await _channel.invokeMethod<void>(method, arguments);
    } on PlatformException {
      // Nothing more to do.
    } on MissingPluginException {
      // Not available in this build.
    }
  }

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

  @override
  Future<void> ring({required String title, required bool video}) async {}

  @override
  Future<void> stopRinging() async {}

  @override
  Future<void> callOngoing({
    required bool on,
    String title = '',
    bool video = false,
  }) async {}

  @override
  Stream<String> get callActions => const Stream.empty();
}
