#ifndef RUNNER_NOTIFIER_H_
#define RUNNER_NOTIFIER_H_

#include <flutter/binary_messenger.h>
#include <flutter/encodable_value.h>
#include <flutter/method_channel.h>
#include <windows.h>
#include <shellapi.h>

#include <memory>
#include <optional>
#include <string>

// Notifications for Zelamar on Windows, driven from Dart over the
// "zelamar/notify" channel (lib/src/messenger/notifier.dart).
//
// A tray icon appears with the first notification; its balloon is shown by
// Windows 10 and 11 as a regular toast. The taskbar button flashes until the
// window comes to the front. Clicking either brings the window back.
class Notifier {
 public:
  Notifier(flutter::BinaryMessenger* messenger, HWND window);
  ~Notifier();

  Notifier(const Notifier&) = delete;
  Notifier& operator=(const Notifier&) = delete;

  // Handles the tray icon's messages; called from the window procedure.
  std::optional<LRESULT> HandleMessage(UINT message, WPARAM wparam,
                                       LPARAM lparam);

 private:
  void Show(const std::wstring& title, const std::wstring& body);
  bool InFront() const;
  void BringToFront();

  HWND window_;
  NOTIFYICONDATAW icon_{};
  bool icon_added_ = false;
  std::unique_ptr<flutter::MethodChannel<flutter::EncodableValue>> channel_;
};

#endif  // RUNNER_NOTIFIER_H_
