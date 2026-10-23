#include "secret_clipboard.h"

#include <flutter/standard_method_codec.h>

#include <cstring>

namespace {

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

// Puts `bytes` on the open clipboard in `format`; false if that failed.
bool Put(UINT format, const void* bytes, size_t size) {
  HGLOBAL memory = GlobalAlloc(GMEM_MOVEABLE, size);
  if (!memory) {
    return false;
  }
  void* target = GlobalLock(memory);
  if (!target) {
    GlobalFree(memory);
    return false;
  }
  std::memcpy(target, bytes, size);
  GlobalUnlock(memory);
  if (!SetClipboardData(format, memory)) {
    GlobalFree(memory);
    return false;
  }
  return true;
}

// Sets one of the registered formats Windows reads as an instruction about
// the rest of the clipboard.
bool Mark(const wchar_t* name, DWORD value) {
  const UINT format = RegisterClipboardFormatW(name);
  return format != 0 && Put(format, &value, sizeof(value));
}

}  // namespace

SecretClipboard::SecretClipboard(flutter::BinaryMessenger* messenger,
                                 HWND window)
    : window_(window) {
  channel_ = std::make_unique<flutter::MethodChannel<flutter::EncodableValue>>(
      messenger, "zelamar/clipboard",
      &flutter::StandardMethodCodec::GetInstance());
  channel_->SetMethodCallHandler([this](const auto& call, auto result) {
    if (call.method_name() != "copySecret") {
      result->NotImplemented();
      return;
    }
    const auto* text = std::get_if<std::string>(call.arguments());
    if (!text) {
      result->Error("bad_arguments", "expected the text");
      return;
    }
    result->Success(flutter::EncodableValue(Copy(Widen(*text))));
  });
}

bool SecretClipboard::Copy(const std::wstring& text) {
  if (!OpenClipboard(window_)) {
    return false;
  }
  EmptyClipboard();
  const bool copied =
      Put(CF_UNICODETEXT, text.c_str(), (text.size() + 1) * sizeof(wchar_t)) &&
      Mark(L"CanIncludeInClipboardHistory", 0) &&
      Mark(L"CanUploadToCloudClipboard", 0) &&
      Mark(L"ExcludeClipboardContentFromMonitorProcessing", 0);
  if (!copied) {
    // Better nothing than a secret on its way to the history.
    EmptyClipboard();
  }
  CloseClipboard();
  return copied;
}
