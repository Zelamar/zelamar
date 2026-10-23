#ifndef RUNNER_FLUTTER_WINDOW_H_
#define RUNNER_FLUTTER_WINDOW_H_

#include <flutter/dart_project.h>
#include <flutter/flutter_view_controller.h>

#include <memory>

#include "notifier.h"
#include "secret_clipboard.h"
#include "win32_window.h"

// A window that does nothing but host a Flutter view.
class FlutterWindow : public Win32Window {
 public:
  // Creates a new FlutterWindow hosting a Flutter view running |project|.
  // `show_message` brings this window back when the same profile is
  // launched again (see InstanceName in notifier.h).
  // `start_hidden` keeps the window in the tray at start (`--tray`, as
  // when Windows starts Zelamar at sign-in).
  FlutterWindow(const flutter::DartProject& project, UINT show_message,
                bool start_hidden);
  virtual ~FlutterWindow();

 protected:
  // Win32Window:
  bool OnCreate() override;
  void OnDestroy() override;
  LRESULT MessageHandler(HWND window, UINT const message, WPARAM const wparam,
                         LPARAM const lparam) noexcept override;

 private:
  // The project to run.
  flutter::DartProject project_;

  UINT show_message_;

  bool start_hidden_;

  // The Flutter instance hosted by this window.
  std::unique_ptr<flutter::FlutterViewController> flutter_controller_;

  // Native notifications (tray toast, taskbar flash).
  std::unique_ptr<Notifier> notifier_;
  std::unique_ptr<SecretClipboard> clipboard_;
};

#endif  // RUNNER_FLUTTER_WINDOW_H_
