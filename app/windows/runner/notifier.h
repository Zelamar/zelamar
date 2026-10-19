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
#include <vector>

// Zelamar's place in the tray, and its notifications, driven from Dart over
// the "zelamar/notify" channel (lib/src/messenger/notifier.dart).
//
// The tray icon is there from the start. Closing the window only hides it,
// so messages keep arriving and notifying; the icon's menu opens Zelamar
// again or quits it. A notification is the icon's balloon, which Windows 10
// and 11 show as a regular toast, and the taskbar button flashes until the
// window comes to the front.
class Notifier {
 public:
  // `show_message` is the message another launch of the same profile posts
  // to bring this window back (see InstanceName).
  Notifier(flutter::BinaryMessenger* messenger, HWND window, UINT show_message);
  ~Notifier();

  Notifier(const Notifier&) = delete;
  Notifier& operator=(const Notifier&) = delete;

  // Handles closing, the tray icon and the show message; called from the
  // window procedure.
  std::optional<LRESULT> HandleMessage(UINT message, WPARAM wparam,
                                       LPARAM lparam);

 private:
  void AddIcon();
  void Balloon(const std::wstring& title, const std::wstring& body);
  void Show(const std::wstring& title, const std::wstring& body);
  bool InFront() const;
  void BringToFront();
  void ShowMenu();

  HWND window_;
  UINT show_message_;
  UINT taskbar_created_;
  NOTIFYICONDATAW icon_{};
  bool icon_added_ = false;
  bool told_about_tray_ = false;
  bool quitting_ = false;
  std::unique_ptr<flutter::MethodChannel<flutter::EncodableValue>> channel_;
};

// A name for this copy of the app and this profile (`--profile=`), so each
// profile runs once: "Zelamar-<profile>-<hash of the program's path>". The
// path keeps a test build apart from the installed app.
std::wstring InstanceName(const std::vector<std::string>& arguments);

#endif  // RUNNER_NOTIFIER_H_
