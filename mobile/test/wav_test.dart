import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:pickscribe/wav.dart';

Uint8List _wav({required int bits, required List<int> samples, bool extraChunk = false}) {
  final builder = BytesBuilder();
  void text(String s) => builder.add(s.codeUnits);
  void u32(int v) =>
      builder.add((ByteData(4)..setUint32(0, v, Endian.little)).buffer.asUint8List());
  void u16(int v) =>
      builder.add((ByteData(2)..setUint16(0, v, Endian.little)).buffer.asUint8List());
  text('RIFF');
  u32(0);
  text('WAVE');
  text('fmt ');
  u32(16);
  u16(1);
  u16(1);
  u32(16000);
  u32(32000);
  u16(2);
  u16(bits);
  if (extraChunk) {
    text('LIST');
    u32(3);
    builder.add([1, 2, 3, 0]);
  }
  text('data');
  u32(samples.length * 2);
  for (final s in samples) {
    builder.add((ByteData(2)..setInt16(0, s, Endian.little)).buffer.asUint8List());
  }
  return builder.toBytes();
}

void main() {
  test('decodes 16-bit samples to floats', () {
    final pcm = decodePcm16Wav(_wav(bits: 16, samples: [0, 16384, -32768]));
    expect(pcm, [0, 0.5, -1]);
  });

  test('skips unknown chunks with odd sizes', () {
    final pcm = decodePcm16Wav(_wav(bits: 16, samples: [16384], extraChunk: true));
    expect(pcm, [0.5]);
  });

  test('rejects other sample formats', () {
    expect(() => decodePcm16Wav(_wav(bits: 8, samples: [1])), throwsFormatException);
    expect(() => decodePcm16Wav(Uint8List.fromList([1, 2, 3])), throwsFormatException);
  });
}
