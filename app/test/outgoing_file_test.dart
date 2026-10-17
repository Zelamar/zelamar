import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as img;
import 'package:zelamar/src/messenger/outgoing_file.dart';
import 'package:zelamar/src/ui/words.dart';

void main() {
  test('big photos are scaled down and get a tiny preview', () async {
    final photo = img.Image(width: 4000, height: 3000);
    img.fill(photo, color: img.ColorRgb8(200, 150, 90));
    final file = await OutgoingFile.prepare(
      'pier.png',
      Uint8List.fromList(img.encodePng(photo)),
    );
    expect(file.isImage, isTrue);
    expect((file.width, file.height), (maxSide, 1920));
    expect(file.mime, 'image/jpeg');
    expect(file.name, 'pier.jpg');
    expect(file.thumb, isNotNull);
    expect(file.thumb!.length, lessThanOrEqualTo(maxThumb));
    final preview = img.decodeJpg(file.thumb!)!;
    expect(preview.width, thumbSide);
  });

  test('small photos keep their bytes; other files pass through', () async {
    final photo = img.Image(width: 300, height: 600);
    final bytes = Uint8List.fromList(img.encodePng(photo));
    final small = await OutgoingFile.prepare('tall.png', bytes);
    expect(small.bytes, bytes, reason: 'sent as picked');
    expect((small.width, small.height), (300, 600));
    expect(small.mime, 'image/png');

    final pdf = await OutgoingFile.prepare('Plan.PDF', Uint8List(10));
    expect(pdf.mime, 'application/pdf');
    expect(pdf.thumb, isNull);
    expect(pdf.isImage, isFalse);

    final broken = await OutgoingFile.prepare('fake.jpg', Uint8List(10));
    expect(broken.thumb, isNull, reason: 'not really an image');
    expect(mimeFor('notes'), 'application/octet-stream');
  });

  test('sizes read like people say them', () {
    expect(describeSize(512), '512 B');
    expect(describeSize(48 * 1024), '48 KB');
    expect(describeSize(2516582), '2.4 MB');
    expect(describeSize(25 * 1024 * 1024), '25 MB');
  });

  test('lengths read like a clock', () {
    expect(describeDuration(const Duration(milliseconds: 7400)), '0:07');
    expect(describeDuration(const Duration(seconds: 83)), '1:23');
    expect(describeDuration(const Duration(minutes: 5)), '5:00');
  });
}
