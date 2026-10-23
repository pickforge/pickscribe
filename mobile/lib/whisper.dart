import 'dart:ffi';
import 'dart:isolate';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

typedef _LoadNative = Pointer<Void> Function(Pointer<Utf8>);
typedef _UnloadNative = Void Function(Pointer<Void>);
typedef _Unload = void Function(Pointer<Void>);
typedef _TranscribeNative = Int32 Function(
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
typedef _Transcribe = int Function(
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
typedef _SetAbortNative = Void Function(Pointer<Void>, Int32);
typedef _SetAbort = void Function(Pointer<Void>, int);
typedef _SupportedNative = Int32 Function();
typedef _Supported = int Function();

DynamicLibrary _open() => DynamicLibrary.open('libpickscribe_whisper.so');

final _setAbort = _open().lookupFunction<_SetAbortNative, _SetAbort>('ps_set_abort');

/// What `ps_transcribe` returns after an abort.
const _cancelled = 5;

class Transcription {
  const Transcription(this.text, this.language);

  final String text;
  final String language;
}

/// Thrown by [Whisper.transcribe] after [Whisper.abort].
class WhisperCancelled implements Exception {
  const WhisperCancelled();
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
    final supported = _open().lookupFunction<_SupportedNative, _Supported>('ps_supported');
    if (supported() == 0) {
      throw const WhisperException('This phone\'s processor is too old for on-device Whisper.');
    }
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

  /// Stops the transcription in progress, which then throws [WhisperCancelled]. Call
  /// [clearAbort] before the next one.
  void abort() => _setAbort(Pointer.fromAddress(_address), 1);

  void clearAbort() => _setAbort(Pointer.fromAddress(_address), 0);

  /// Transcribes 16 kHz mono audio. With more than one language, the most likely of them
  /// wins. A beam size of 1 decodes greedily, which is faster. Calls must not overlap, since
  /// they share the model's state.
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
      final transcribe = library.lookupFunction<_TranscribeNative, _Transcribe>('ps_transcribe');
      final freeString = library.lookupFunction<_FreeStringNative, _FreeString>('ps_free_string');
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
        if (status == _cancelled) throw const WhisperCancelled();
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
    _open().lookupFunction<_UnloadNative, _Unload>('ps_unload')(Pointer.fromAddress(_address));
  }
}
