use std::{fs, os::unix::fs::PermissionsExt};

use private::{Dir, Mode, Replace};
use rustix::{fs::Mode as Bits, process::umask};

// umask is the whole process's, so no test harness with its threads.
fn main() {
    let home = tempfile::tempdir().unwrap();
    umask(Bits::from_raw_mode(0o277));
    let dir = Dir::open(home.path(), &[]).unwrap();
    dir.write("private", b"", Mode::Private, Replace::No).unwrap();
    dir.write("readable", b"", Mode::Readable, Replace::No).unwrap();
    let mode = |name| fs::metadata(home.path().join(name)).unwrap().permissions().mode() & 0o777;
    assert_eq!((mode("private"), mode("readable")), (0o600, 0o644));
}
