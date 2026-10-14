#include "notifier.h"

#include <flutter/standard_method_codec.h>
#include <strsafe.h>

#include "resource.h"

namespace {

// Window message the tray icon sends back to us.
constexpr UINT kTrayMessage = WM_APP + 1;
constexpr UINT kTrayId = 1;

// UTF-8 (from Dart) to the UTF-16 Windows wants.
std::wstring Widen(const std::string& text) {
  if (text.empty()) {
    return std::wstring();
  }
  const int size = MultiByteToWideChar(CP_UTF8, 0, text.data(),
                                       static_cast<int>(text.size()), nullptr, 0);
  std::wstring wide(size, L'\0');
  MultiByteToWideChar(CP_UTF8, 0, text.data(), static_cast<int>(text.size()),
                      wide.data(), size);
  return wide;
}

std::string Argument(const flutter::EncodableMap& arguments, const char* key) {
  const auto found = arguments.find(flutter::EncodableValue(key));
  if (found == arguments.end()) {
    return std::string();
  }
  const auto* text = std::get_if<std::string>(&found->second);
  return text ? *text : std::string();
}

}  // namespace

Notifier::Notifier(flutter::BinaryMessenger* messenger, HWND window)
    : window_(window) {
  channel_ = std::make_unique<flutter::MethodChannel<flutter::EncodableValue>>(
      messenger, "zelamar/notify",
      &flutter::StandardMethodCodec::GetInstance());
  channel_->SetMethodCallHandler([this](const auto& call, auto result) {
    if (call.method_name() == "show") {
      const auto* arguments =
          std::get_if<flutter::EncodableMap>(call.arguments());
      if (!arguments) {
        result->Error("bad_arguments", "expected title and body");
        return;
      }
      Show(Widen(Argument(*arguments, "title")),
           Widen(Argument(*arguments, "body")));
      result->Success();
    } else if (call.method_name() == "inFront") {
      result->Success(flutter::EncodableValue(InFront()));
    } else {
      result->NotImplemented();
    }
  });
}

Notifier::~Notifier() {
  if (icon_added_) {
    Shell_NotifyIconW(NIM_DELETE, &icon_);
  }
}

bool Notifier::InFront() const {
  return GetForegroundWindow() == window_ && !IsIconic(window_);
}

void Notifier::Show(const std::wstring& title, const std::wstring& body) {
  if (!icon_added_) {
    icon_.cbSize = sizeof(icon_);
    icon_.hWnd = window_;
    icon_.uID = kTrayId;
    icon_.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE | NIF_SHOWTIP;
    icon_.uCallbackMessage = kTrayMessage;
    icon_.hIcon = LoadIconW(GetModuleHandleW(nullptr),
                            MAKEINTRESOURCEW(IDI_APP_ICON));
    StringCchCopyW(icon_.szTip, ARRAYSIZE(icon_.szTip), L"Zelamar");
    icon_added_ = Shell_NotifyIconW(NIM_ADD, &icon_) != FALSE;
    if (icon_added_) {
      icon_.uVersion = NOTIFYICON_VERSION_4;
      Shell_NotifyIconW(NIM_SETVERSION, &icon_);
    }
  }
  if (icon_added_) {
    NOTIFYICONDATAW balloon = icon_;
    balloon.uFlags = NIF_INFO | NIF_SHOWTIP;
    balloon.dwInfoFlags = NIIF_USER | NIIF_LARGE_ICON;
    balloon.hBalloonIcon = icon_.hIcon;
    StringCchCopyW(balloon.szInfoTitle, ARRAYSIZE(balloon.szInfoTitle),
                   title.c_str());
    StringCchCopyW(balloon.szInfo, ARRAYSIZE(balloon.szInfo), body.c_str());
    Shell_NotifyIconW(NIM_MODIFY, &balloon);
  }

  // Flash the taskbar button until the window comes to the front.
  FLASHWINFO flash{};
  flash.cbSize = sizeof(flash);
  flash.hwnd = window_;
  flash.dwFlags = FLASHW_TRAY | FLASHW_TIMERNOFG;
  FlashWindowEx(&flash);
}

void Notifier::BringToFront() {
  if (IsIconic(window_)) {
    ShowWindow(window_, SW_RESTORE);
  }
  SetForegroundWindow(window_);
}

std::optional<LRESULT> Notifier::HandleMessage(UINT message, WPARAM wparam,
                                               LPARAM lparam) {
  if (message != kTrayMessage) {
    return std::nullopt;
  }
  // With NOTIFYICON_VERSION_4 the event is in the low word of lparam.
  switch (LOWORD(lparam)) {
    case NIN_BALLOONUSERCLICK:
    case NIN_SELECT:
    case NIN_KEYSELECT:
    case WM_LBUTTONUP:
      BringToFront();
      break;
  }
  return 0;
}
