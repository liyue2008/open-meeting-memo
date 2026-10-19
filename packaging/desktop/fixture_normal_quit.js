// Called only after the original Python Popen owner verifies UID/start/path/hash.
ObjC.import('AppKit');
function run(args) {
  const pid = Number(args[0]), path = args[1];
  const app = $.NSRunningApplication.runningApplicationWithProcessIdentifier(pid);
  if (!app || app.isNil() || Number(app.processIdentifier) !== pid || ObjC.unwrap(app.executableURL.path) !== path) throw Error('owned Cocoa identity mismatch');
  return Boolean(app.terminate) ? 'accepted' : 'refused';
}
