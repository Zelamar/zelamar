#include "notifier.h"

#include <flutter/standard_method_codec.h>
#include <strsafe.h>

#include <cstdint>
#include <cwctype>

#include "resource.h"
#include "utils.h"

namespace {

// Window message the tray icon sends back to us.
constexpr UINT kTrayMessage = WM_APP + 1;
constexpr UINT kTrayId = 1;

// Where Windows keeps what starts at sign-in, per user.
constexpr wchar_t kRunKey[] =
    L"Software\\Microsoft\\Windows\\CurrentVersion\\Run";

// Commands of the tray icon's menu.
constexpr UINT kOpenCommand = 1;
constexpr UINT kQuitCommand = 2;

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

std::wstring ProfileArgument(const std::vector<std::string>& arguments) {
  const std::string option = "--profile=";
  std::wstring profile;
  for (const auto& argument : arguments) {
    if (argument.rfind(option, 0) != 0) {
      continue;
    }
    profile.clear();
    for (const char c : argument.substr(option.size())) {
      if ((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
          (c >= '0' && c <= '9') || c == '-' || c == '_') {
        profile.push_back(static_cast<wchar_t>(c));
      }
    }
  }
  return profile;
}

std::wstring InstanceName(const std::vector<std::string>& arguments) {
  std::wstring name = L"Zelamar";
  wchar_t path[MAX_PATH] = {};
  const DWORD length = GetModuleFileNameW(nullptr, path, MAX_PATH);
  uint32_t hash = 2166136261u;  // FNV-1a
  for (DWORD at = 0; at < length; ++at) {
    hash = (hash ^ static_cast<uint32_t>(towlower(path[at]))) * 16777619u;
  }
  wchar_t hex[9] = {};
  StringCchPrintfW(hex, ARRAYSIZE(hex), L"%08x", hash);
  const std::wstring profile = ProfileArgument(arguments);
  if (!profile.empty()) {
    name += L"-" + profile;
  }
  return name + L"-" + hex;
}

Notifier::Notifier(flutter::BinaryMessenger* messenger, HWND window,
                   UINT show_message)
    : window_(window),
      show_message_(show_message),
      taskbar_created_(RegisterWindowMessageW(L"TaskbarCreated")) {
  wchar_t path[MAX_PATH] = {};
  GetModuleFileNameW(nullptr, path, MAX_PATH);
  const std::wstring profile = ProfileArgument(GetCommandLineArguments());
  run_name_ = profile.empty() ? L"Zelamar" : L"Zelamar (" + profile + L")";
  run_command_ = L"\"" + std::wstring(path) + L"\" --tray";
  if (!profile.empty()) {
    run_command_ += L" --profile=" + profile;
  }
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
    } else if (call.method_name() == "ring") {
      // A call rings while the window is hidden: say who, and come to the
      // front so the call can be answered (ADR-0019).
      const auto* arguments =
          std::get_if<flutter::EncodableMap>(call.arguments());
      bool video = false;
      if (arguments) {
        const auto found = arguments->find(flutter::EncodableValue("video"));
        if (found != arguments->end()) {
          if (const auto* flag = std::get_if<bool>(&found->second)) {
            video = *flag;
          }
        }
      }
      Show(arguments ? Widen(Argument(*arguments, "title")) : L"Zelamar",
           video ? L"Incoming video call" : L"Incoming call");
      BringToFront();
      result->Success();
    } else if (call.method_name() == "inFront") {
      result->Success(flutter::EncodableValue(InFront()));
    } else if (call.method_name() == "startsAtSignIn") {
      result->Success(flutter::EncodableValue(StartsAtSignIn()));
    } else if (call.method_name() == "setStartsAtSignIn") {
      const auto* on = std::get_if<bool>(call.arguments());
      if (!on) {
        result->Error("bad_arguments", "expected a bool");
        return;
      }
      SetStartsAtSignIn(*on);
      result->Success(flutter::EncodableValue(StartsAtSignIn()));
    } else {
      result->NotImplemented();
    }
  });
  AddIcon();
}

Notifier::~Notifier() {
  if (icon_added_) {
    Shell_NotifyIconW(NIM_DELETE, &icon_);
  }
}

void Notifier::AddIcon() {
  icon_ = NOTIFYICONDATAW{};
  icon_.cbSize = sizeof(icon_);
  icon_.hWnd = window_;
  icon_.uID = kTrayId;
  icon_.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE | NIF_SHOWTIP;
  icon_.uCallbackMessage = kTrayMessage;
  icon_.hIcon =
      LoadIconW(GetModuleHandleW(nullptr), MAKEINTRESOURCEW(IDI_APP_ICON));
  StringCchCopyW(icon_.szTip, ARRAYSIZE(icon_.szTip), L"Zelamar");
  icon_added_ = Shell_NotifyIconW(NIM_ADD, &icon_) != FALSE;
  if (icon_added_) {
    icon_.uVersion = NOTIFYICON_VERSION_4;
    Shell_NotifyIconW(NIM_SETVERSION, &icon_);
  }
}

bool Notifier::InFront() const {
  return IsWindowVisible(window_) && GetForegroundWindow() == window_ &&
         !IsIconic(window_);
}

void Notifier::Balloon(const std::wstring& title, const std::wstring& body) {
  if (!icon_added_) {
    AddIcon();
  }
  if (!icon_added_) {
    return;
  }
  NOTIFYICONDATAW balloon = icon_;
  balloon.uFlags = NIF_INFO | NIF_SHOWTIP;
  balloon.dwInfoFlags = NIIF_USER | NIIF_LARGE_ICON;
  balloon.hBalloonIcon = icon_.hIcon;
  StringCchCopyW(balloon.szInfoTitle, ARRAYSIZE(balloon.szInfoTitle),
                 title.c_str());
  StringCchCopyW(balloon.szInfo, ARRAYSIZE(balloon.szInfo), body.c_str());
  Shell_NotifyIconW(NIM_MODIFY, &balloon);
}

void Notifier::Show(const std::wstring& title, const std::wstring& body) {
  Balloon(title, body);

  // Flash the taskbar button until the window comes to the front.
  FLASHWINFO flash{};
  flash.cbSize = sizeof(flash);
  flash.hwnd = window_;
  flash.dwFlags = FLASHW_TRAY | FLASHW_TIMERNOFG;
  FlashWindowEx(&flash);
}

void Notifier::BringToFront() {
  ShowWindow(window_, IsIconic(window_) ? SW_RESTORE : SW_SHOW);
  SetForegroundWindow(window_);
}

bool Notifier::StartsAtSignIn() const {
  wchar_t value[2 * MAX_PATH] = {};
  DWORD size = sizeof(value);
  if (RegGetValueW(HKEY_CURRENT_USER, kRunKey, run_name_.c_str(), RRF_RT_REG_SZ,
                   nullptr, value, &size) != ERROR_SUCCESS) {
    return false;
  }
  // Only this copy of the app counts: a moved app must be set again.
  return run_command_ == value;
}

void Notifier::SetStartsAtSignIn(bool on) {
  HKEY key = nullptr;
  if (RegCreateKeyExW(HKEY_CURRENT_USER, kRunKey, 0, nullptr, 0, KEY_SET_VALUE,
                      nullptr, &key, nullptr) != ERROR_SUCCESS) {
    return;
  }
  if (on) {
    RegSetValueExW(
        key, run_name_.c_str(), 0, REG_SZ,
        reinterpret_cast<const BYTE*>(run_command_.c_str()),
        static_cast<DWORD>((run_command_.size() + 1) * sizeof(wchar_t)));
  } else {
    RegDeleteValueW(key, run_name_.c_str());
  }
  RegCloseKey(key);
}

void Notifier::ShowMenu() {
  HMENU menu = CreatePopupMenu();
  if (!menu) {
    return;
  }
  AppendMenuW(menu, MF_STRING, kOpenCommand, L"Open Zelamar");
  AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
  AppendMenuW(menu, MF_STRING, kQuitCommand, L"Quit Zelamar");
  SetMenuDefaultItem(menu, kOpenCommand, FALSE);
  POINT point{};
  GetCursorPos(&point);
  // The menu closes on a click elsewhere only if the window is in front.
  SetForegroundWindow(window_);
  const UINT chosen = static_cast<UINT>(TrackPopupMenu(
      menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, point.x, point.y,
      0, window_, nullptr));
  DestroyMenu(menu);
  if (chosen == kOpenCommand) {
    BringToFront();
  } else if (chosen == kQuitCommand) {
    quitting_ = true;
    DestroyWindow(window_);
  }
}

std::optional<LRESULT> Notifier::HandleMessage(UINT message, WPARAM wparam,
                                               LPARAM lparam) {
  if (show_message_ != 0 && message == show_message_) {
    // Another launch of this profile: show this window instead.
    BringToFront();
    return 0;
  }
  if (message == taskbar_created_) {
    // Explorer restarted and forgot every tray icon.
    AddIcon();
    return 0;
  }
  if (message == WM_CLOSE && !quitting_ && icon_added_) {
    ShowWindow(window_, SW_HIDE);
    if (!told_about_tray_) {
      told_about_tray_ = true;
      Balloon(L"Zelamar is still running",
              L"New messages still reach you. To quit, right-click the "
              L"Zelamar icon here.");
    }
    return 0;
  }
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
    case WM_CONTEXTMENU:
      ShowMenu();
      break;
  }
  return 0;
}
