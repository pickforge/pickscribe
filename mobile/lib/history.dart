import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';

import 'storage.dart';

/// One dictation. History is the only place dictated text is stored, and audio is never kept.
@immutable
class HistoryItem {
  const HistoryItem({
    required this.id,
    required this.at,
    required this.text,
    required this.transcript,
    required this.audioMs,
    this.app,
    this.delivered,
  });

  final String id;
  final DateTime at;

  /// The text that was inserted, or would have been.
  final String text;

  /// What Whisper heard, before the dictionary corrections.
  final String transcript;
  final int audioMs;

  /// The package of the app that had the text field.
  final String? app;

  /// False when the text could not be inserted, null until the bubble reports back.
  final bool? delivered;

  HistoryItem copyWith({bool? delivered}) => HistoryItem(
    id: id,
    at: at,
    text: text,
    transcript: transcript,
    audioMs: audioMs,
    app: app,
    delivered: delivered ?? this.delivered,
  );

  Map<String, Object?> toJson() => {
    'id': id,
    'at': at.millisecondsSinceEpoch ~/ 1000,
    'text': text,
    'transcript': transcript,
    'audio_ms': audioMs,
    'app': ?app,
    'delivered': ?delivered,
  };

  static HistoryItem? fromJson(Object? json) {
    if (json is! Map<String, Object?>) return null;
    final id = json['id'];
    final at = json['at'];
    final text = json['text'];
    if (id is! String || at is! int || text is! String) return null;
    return HistoryItem(
      id: id,
      at: DateTime.fromMillisecondsSinceEpoch(at * 1000),
      text: text,
      transcript: json['transcript'] as String? ?? text,
      audioMs: json['audio_ms'] as int? ?? 0,
      app: json['app'] as String?,
      delivered: json['delivered'] as bool?,
    );
  }
}

/// Recent dictations in `history.jsonl` in the app's private storage, oldest first on disk
/// and newest first in [items]. Lines that do not parse are skipped.
class HistoryStore extends ChangeNotifier {
  HistoryStore(Directory dir, {this.limit = 1000})
    : _file = SafeFile(File('${dir.path}/history.jsonl'));

  final SafeFile _file;
  final int limit;
  List<HistoryItem> _items = [];

  List<HistoryItem> get items => List.unmodifiable(_items);

  Future<void> load() async {
    if (await _file.file.exists()) {
      final text = await _file.file.readAsString();
      _items = const LineSplitter()
          .convert(text)
          .map((line) {
            try {
              return HistoryItem.fromJson(jsonDecode(line));
            } on FormatException {
              return null;
            }
          })
          .whereType<HistoryItem>()
          .toList()
          .reversed
          .toList();
      // A crash during an append can leave a last line without its newline. The rewrite drops
      // it, so the next append starts on a new line.
      if (text.isNotEmpty && !text.endsWith('\n')) return _rewrite();
    }
    notifyListeners();
  }

  Future<void> add(HistoryItem item) {
    _items.insert(0, item);
    if (_items.length > limit) {
      _items.removeRange(limit, _items.length);
      return _rewrite();
    }
    notifyListeners();
    return _file.append('${jsonEncode(item.toJson())}\n');
  }

  Future<void> markDelivered(String id, bool delivered) {
    final index = _items.indexWhere((item) => item.id == id);
    if (index < 0) return Future.value();
    _items[index] = _items[index].copyWith(delivered: delivered);
    return _rewrite();
  }

  Future<void> remove(String id) {
    _items.removeWhere((item) => item.id == id);
    return _rewrite();
  }

  /// Puts back an item that was just removed, in its place by time.
  Future<void> restore(HistoryItem item) {
    final index = _items.indexWhere((other) => other.at.isBefore(item.at));
    _items.insert(index < 0 ? _items.length : index, item);
    return _rewrite();
  }

  Future<void> clear() {
    _items = [];
    return _rewrite();
  }

  Future<void> _rewrite() {
    notifyListeners();
    final text = _items.reversed.map((item) => '${jsonEncode(item.toJson())}\n').join();
    return _file.write(text);
  }
}
