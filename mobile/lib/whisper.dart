import 'dart:ffi';
import 'dart:isolate';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

typedef _LoadNative = Pointer<Void> Function(Pointer<Utf8>);
typedef _UnloadNative = Void Function(Pointer<Void>);
typedef _Unload = void Function(Pointer<Void>);
typedef _TranscribeNative =
    Int32 Function(
      Pointer<Void>,
      Pointer<Float>,
      Int32,
      Pointer<Utf8>,
      Pointer<Utf8>,
      Int32,
      Int32,
      Pointer<Pointer<Utf8>>,
      Pointer<Pointer<Utf8>>,
    );
typedef _Transcribe =
    int Function(
      Pointer<Void>,
      Pointer<Float>,
      int,
      Pointer<Utf8>,
      Pointer<Utf8>,
      int,
      int,
      Pointer<Pointer<Utf8>>,
      Pointer<Pointer<Utf8>>,
    );
typedef _FreeStringNative = Void Function(Pointer<Utf8>);
typedef _FreeString = void Function(Pointer<Utf8>);

DynamicLibrary _open() => DynamicLibrary.open('libpickscribe_whisper.so');

class Transcription {
  const Transcription(this.text, this.language);

  final String text;
  final String language;
}

class WhisperException implements Exception {
  const WhisperException(this.message);

  final String message;

  @override
  String toString() => message;
}

/// A loaded Whisper model. Calls block for seconds, so each one runs on its own isolate,
/// and the native context travels between isolates as an address.
class Whisper {
  Whisper._(this._address);

  final int _address;

  static Future<Whisper> load(String modelPath) async {
    final address = await Isolate.run(() {
      final load = _open().lookupFunction<_LoadNative, _LoadNative>('ps_load');
      final path = modelPath.toNativeUtf8();
      try {
        return load(path).address;
      } finally {
        malloc.free(path);
      }
    });
    if (address == 0) throw WhisperException('Cannot load the model at $modelPath');
    return Whisper._(address);
  }

  /// Transcribes 16 kHz mono audio. With more than one language, the most likely of them
  /// wins. A beam size of 1 decodes greedily, which is faster.
  Future<Transcription> transcribe(
    Float32List pcm, {
    required List<String> languages,
    String prompt = '',
    int threads = 6,
    int beamSize = 5,
  }) {
    final address = _address;
    return Isolate.run(() {
      final library = _open();
      final transcribe = library.lookupFunction<_TranscribeNative, _Transcribe>(
        'ps_transcribe',
      );
      final freeString = library.lookupFunction<_FreeStringNative, _FreeString>(
        'ps_free_string',
      );
      final samples = malloc<Float>(pcm.length);
      final codes = languages.join(',').toNativeUtf8();
      final glossary = prompt.toNativeUtf8();
      final outText = malloc<Pointer<Utf8>>();
      final outLanguage = malloc<Pointer<Utf8>>();
      try {
        samples.asTypedList(pcm.length).setAll(0, pcm);
        final status = transcribe(
          Pointer.fromAddress(address),
          samples,
          pcm.length,
          codes,
          glossary,
          threads,
          beamSize,
          outText,
          outLanguage,
        );
        if (status != 0) throw WhisperException('Transcription failed with code $status');
        final text = outText.value.toDartString();
        freeString(outText.value);
        return Transcription(text.trim(), outLanguage.value.toDartString());
      } finally {
        malloc.free(samples);
        malloc.free(codes);
        malloc.free(glossary);
        malloc.free(outText);
        malloc.free(outLanguage);
      }
    });
  }

  void dispose() {
    _open().lookupFunction<_UnloadNative, _Unload>('ps_unload')(
      Pointer.fromAddress(_address),
    );
  }
}
