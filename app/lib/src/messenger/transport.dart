import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/rust/api/session.dart';

/// Moves bytes between the core and the delivery service. It decides
/// nothing: the core builds every request and reads every answer.
class ServiceTransport {
  new(this.base, {http.Client? client}) : _client = client ?? http.Client();

  /// The service's origin, such as `https://api.example.com`.
  final Uri base;
  final http.Client _client;

  static const _timeout = Duration(seconds: 20);

  /// Sends [call]; returns the status and body. Network failures surface as
  /// a [MessengerFailure] with code `offline`.
  Future<(int, Uint8List)> send(HttpCall call) async {
    final request = http.Request(call.method, base.resolve(call.path));
    for (final header in call.headers) {
      request.headers[header.name] = header.value;
    }
    request.bodyBytes = call.body;
    try {
      final streamed = await _client.send(request).timeout(_timeout);
      final response = await http.Response.fromStream(streamed)
          .timeout(_timeout);
      return (response.statusCode, response.bodyBytes);
    } on Object catch (error) {
      if (error is SocketException ||
          error is http.ClientException ||
          error is TimeoutException ||
          error is HandshakeException) {
        throw MessengerFailure('offline', '$error');
      }
      rethrow;
    }
  }

  /// Opens the live connection to a mailbox.
  Future<WebSocket> connect(MailboxLink link) {
    final scheme = base.scheme == 'https' ? 'wss' : 'ws';
    final uri = base.replace(scheme: scheme).resolve(link.path);
    return WebSocket.connect(
      uri.toString(),
      headers: {'authorization': link.authorization},
    ).timeout(_timeout);
  }

  void close() => _client.close();
}
