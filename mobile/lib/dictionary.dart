import 'dart:io';

import 'package:flutter/foundation.dart';

const _arrow = '->';
const _maxLength = 100;

/// A term to spell exactly, or a correction for a word Whisper keeps mishearing. The rules
/// match `src/dictionary.rs` on the desktop.
@immutable
class DictionaryEntry {
  const DictionaryEntry._(this.text, this.heard);

  /// Cleans both fields and throws a [FormatException] with a message for the user.
  factory DictionaryEntry(String text, {String? heard}) {
    final cleanText = _clean(text, 'word');
    final cleanHeard = heard == null || heard.trim().isEmpty ? null : _clean(heard, 'heard word');
    if (cleanHeard != null && cleanHeard.toLowerCase() == cleanText.toLowerCase()) {
      throw const FormatException('The heard word and the spelling are the same.');
    }
    return DictionaryEntry._(cleanText, cleanHeard);
  }

  /// The spelling to use.
  final String text;

  /// What Whisper heard instead, for a correction.
  final String? heard;

  bool get isCorrection => heard != null;

  static DictionaryEntry? parse(String line) {
    try {
      final at = line.indexOf(_arrow);
      if (at < 0) return DictionaryEntry(line);
      final heard = line.substring(0, at);
      // A correction line needs both sides, as on the desktop.
      if (heard.trim().isEmpty) return null;
      return DictionaryEntry(line.substring(at + _arrow.length), heard: heard);
    } on FormatException {
      return null;
    }
  }

  String get line => heard == null ? text : '$heard $_arrow $text';

  bool sameAs(DictionaryEntry other) =>
      text.toLowerCase() == other.text.toLowerCase() &&
      heard?.toLowerCase() == other.heard?.toLowerCase();

  static String _clean(String value, String name) {
    final clean = value.trim().split(RegExp(r'\s+')).where((part) => part.isNotEmpty).join(' ');
    if (clean.isEmpty) throw FormatException('The $name is empty.');
    if (clean.runes.any((rune) => rune < 0x20 || rune == 0x7f)) {
      throw FormatException('The $name cannot contain control characters.');
    }
    if (clean.contains(_arrow) || clean.startsWith('#')) {
      throw FormatException('The $name cannot contain "$_arrow" or start with "#".');
    }
    if (clean.runes.length > _maxLength) {
      throw FormatException('The $name is longer than $_maxLength characters.');
    }
    return clean;
  }
}

/// The dictionary entries, in the order they were added.
class Dictionary {
  Dictionary([List<DictionaryEntry>? entries]) : entries = entries ?? [];

  /// Lines that do not parse, such as comments, are skipped, so a typo never stops dictation.
  factory Dictionary.parse(String source) => Dictionary(
    source.split('\n').map(DictionaryEntry.parse).whereType<DictionaryEntry>().toList(),
  );

  final List<DictionaryEntry> entries;

  String serialize() => '$_header${entries.map((entry) => '${entry.line}\n').join()}';

  static const _header =
      '# PickScribe dictionary: one term per line, or "heard -> meant" for a correction.\n';

  /// Returns false when the entry was already there.
  bool add(DictionaryEntry entry) {
    if (entries.any(entry.sameAs)) return false;
    entries.add(entry);
    return true;
  }

  bool remove(DictionaryEntry entry) {
    final index = entries.indexWhere(entry.sameAs);
    if (index < 0) return false;
    entries.removeAt(index);
    return true;
  }

  /// Every distinct spelling, for the Whisper glossary.
  List<String> get terms {
    final terms = <String>[];
    for (final entry in entries) {
      if (!terms.any((term) => term.toLowerCase() == entry.text.toLowerCase())) {
        terms.add(entry.text);
      }
    }
    return terms;
  }

  /// Whisper copies the spelling it sees in the initial prompt.
  String get prompt => terms.isEmpty ? '' : 'Glossary: ${terms.join(', ')}.';

  /// Replaces each heard word with its correction, matching whole words and ignoring case.
  String apply(String text) => entries
      .where((entry) => entry.isCorrection)
      .fold(text, (text, entry) => replaceWord(text, entry.heard!, entry.text));
}

/// Replaces whole-word, case-insensitive matches of [heard] in [text] with [meant].
String replaceWord(String text, String heard, String meant) {
  final chars = text.runes.toList();
  final length = heard.runes.length;
  final lower = heard.toLowerCase();
  final out = StringBuffer();
  var i = 0;
  while (i < chars.length) {
    if (_wordAt(chars, i, length, lower)) {
      out.write(meant);
      i += length;
    } else {
      out.writeCharCode(chars[i]);
      i += 1;
    }
  }
  return out.toString();
}

bool _wordAt(List<int> chars, int at, int length, String lower) {
  final end = at + length;
  if (length == 0 || end > chars.length) return false;
  if (String.fromCharCodes(chars, at, end).toLowerCase() != lower) return false;
  return (at == 0 || !_isWordChar(chars[at - 1])) &&
      (end == chars.length || !_isWordChar(chars[end]));
}

final _wordChar = RegExp(r'[\p{L}\p{N}]', unicode: true);

bool _isWordChar(int rune) => _wordChar.hasMatch(String.fromCharCode(rune));

/// The dictionary file, `dictionary.txt` in the app's private storage, in the desktop format.
class DictionaryStore extends ChangeNotifier {
  DictionaryStore(Directory dir) : _file = File('${dir.path}/dictionary.txt');

  final File _file;
  Dictionary _dictionary = Dictionary();

  Dictionary get dictionary => _dictionary;

  Future<void> load() async {
    if (await _file.exists()) _dictionary = Dictionary.parse(await _file.readAsString());
    notifyListeners();
  }

  /// Returns false when the entry was already there.
  Future<bool> add(DictionaryEntry entry) async {
    if (!_dictionary.add(entry)) return false;
    await _save();
    return true;
  }

  Future<void> remove(DictionaryEntry entry) async {
    if (_dictionary.remove(entry)) await _save();
  }

  Future<void> _save() async {
    notifyListeners();
    await _file.writeAsString(_dictionary.serialize(), flush: true);
  }
}
