use std::{fs, os::unix::fs::PermissionsExt};

use private::{Dir, Mode, Replace};
use rustix::{fs::Mode as Bits, process::umask};

// umask is the whole process's, so no test harness with its threads.
fn main() {
    let home = tempfile::tempdir().unwrap();
    umask(Bits::from_raw_mode(0o277));
    let dir = Dir::open(home.path(), &["made"]).unwrap();
    dir.write("private", b"", Mode::Private, Replace::No).unwrap();
    dir.write("readable", b"", Mode::Readable, Replace::No).unwrap();
    dir.append("log", b"a").unwrap();
    dir.append("log", b"b").unwrap();
    dir.create_dir("run").unwrap().unwrap().write("a", b"", Mode::Private, Replace::No).unwrap();
    let mode = |name| fs::metadata(home.path().join(name)).unwrap().permissions().mode() & 0o777;
    let modes = ["made", "made/private", "made/readable", "made/log", "made/run"].map(mode);
    assert_eq!(modes, [0o700, 0o600, 0o644, 0o600, 0o700]);
}
