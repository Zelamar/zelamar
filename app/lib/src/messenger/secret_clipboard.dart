import 'dart:io';

import 'package:flutter/services.dart';

const _channel = MethodChannel('zelamar/clipboard');

/// Copies a secret, such as the recovery key, so the system treats it as
/// one: kept out of Windows' clipboard history and cloud clipboard, shown
/// as dots in Android's clipboard preview. Elsewhere, or if the platform
/// cannot, it is an ordinary copy.
Future<void> copySecret(String text) async {
  if (Platform.isWindows || Platform.isAndroid) {
    try {
      if (await _channel.invokeMethod<bool>('copySecret', text) ?? false) {
        return;
      }
    } on PlatformException {
      // An ordinary copy below.
    } on MissingPluginException {
      // An ordinary copy below.
    }
  }
  await Clipboard.setData(ClipboardData(text: text));
}
