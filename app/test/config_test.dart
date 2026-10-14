import 'package:flutter_test/flutter_test.dart';
import 'package:zelamar/src/messenger/config.dart';

void main() {
  test('defaults apply without arguments', () {
    final config = MessengerConfig.from(const []);
    expect(config.profile, MessengerConfig.defaultProfile);
    expect(config.service, Uri.parse(MessengerConfig.defaultService));
  });

  test('a desktop launch can choose a profile and a service', () {
    final config = MessengerConfig.from(const [
      '--profile=Bob',
      '--service=https://example.com',
    ]);
    expect(config.profile, 'bob');
    expect(config.service, Uri.parse('https://example.com'));
  });

  test('a profile never smuggles a path', () {
    expect(MessengerConfig.from(const [r'--profile=..\..\x']).profile, 'x');
    expect(
      MessengerConfig.from(const ['--profile=../']).profile,
      MessengerConfig.defaultProfile,
    );
  });
}
