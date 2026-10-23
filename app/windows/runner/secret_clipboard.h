#ifndef RUNNER_SECRET_CLIPBOARD_H_
#define RUNNER_SECRET_CLIPBOARD_H_

#include <flutter/binary_messenger.h>
#include <flutter/encodable_value.h>
#include <flutter/method_channel.h>
#include <windows.h>

#include <memory>
#include <string>

// Copies secrets, such as the recovery key, so that Windows keeps them out
// of the clipboard history (Win+V) and the cloud clipboard, and clipboard
// monitors skip them. Driven from Dart over the "zelamar/clipboard" channel
// (lib/src/messenger/secret_clipboard.dart).
class SecretClipboard {
 public:
  SecretClipboard(flutter::BinaryMessenger* messenger, HWND window);

  SecretClipboard(const SecretClipboard&) = delete;
  SecretClipboard& operator=(const SecretClipboard&) = delete;

 private:
  bool Copy(const std::wstring& text);

  HWND window_;
  std::unique_ptr<flutter::MethodChannel<flutter::EncodableValue>> channel_;
};

#endif  // RUNNER_SECRET_CLIPBOARD_H_
