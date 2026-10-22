import 'dart:typed_data';

/// Reads the samples of a 16-bit PCM mono WAV file as floats from -1 to 1.
Float32List decodePcm16Wav(Uint8List bytes) {
  final data = ByteData.sublistView(bytes);
  if (bytes.length < 12 ||
      String.fromCharCodes(bytes, 0, 4) != 'RIFF' ||
      String.fromCharCodes(bytes, 8, 12) != 'WAVE') {
    throw const FormatException('Not a WAV file');
  }
  var offset = 12;
  int? bits;
  int? channels;
  while (offset + 8 <= bytes.length) {
    final id = String.fromCharCodes(bytes, offset, offset + 4);
    final size = data.getUint32(offset + 4, Endian.little);
    final body = offset + 8;
    if (id == 'fmt ') {
      channels = data.getUint16(body + 2, Endian.little);
      bits = data.getUint16(body + 14, Endian.little);
    } else if (id == 'data') {
      if (bits != 16 || channels != 1) {
        throw const FormatException('Only 16-bit mono WAV is supported');
      }
      final end = (body + size).clamp(body, bytes.length);
      final out = Float32List((end - body) ~/ 2);
      for (var i = 0; i < out.length; i++) {
        out[i] = data.getInt16(body + i * 2, Endian.little) / 32768;
      }
      return out;
    }
    // Chunks are padded to an even size.
    offset = body + size + (size.isOdd ? 1 : 0);
  }
  throw const FormatException('The WAV file has no data chunk');
}
