import 'dart:io';

/// One file that changes one write at a time, in order. A full write goes to a temporary file
/// that then replaces the original, so a crash or a full disk never leaves half a file.
class SafeFile {
  SafeFile(this.file);

  final File file;
  Future<void> _writes = Future.value();

  /// Replaces the whole file with [text].
  Future<void> write(String text) => _queue(() async {
    final temp = File('${file.path}.tmp');
    await temp.writeAsString(text, flush: true);
    await temp.rename(file.path);
  });

  /// Adds [text] to the end of the file. A crash can cut only this text short.
  Future<void> append(String text) =>
      _queue(() => file.writeAsString(text, mode: FileMode.append, flush: true));

  // A failed write reaches its caller and does not block the ones after it.
  Future<void> _queue(Future<void> Function() write) {
    final next = _writes.then((_) => write());
    _writes = next.catchError((Object _) {});
    return next;
  }
}
