//! Integration tests for the atomic write layer.

use std::ffi::OsStr;
use std::fs;
use std::io::Write;

use tabmux::io::SafeWriter;

#[test]
fn writes_new_file_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("out.csv");

    let mut writer = SafeWriter::create(&destination, false, false).unwrap();
    writer.write_all(b"a,b\n1,2\n").unwrap();
    writer.commit().unwrap();

    assert_eq!(fs::read_to_string(&destination).unwrap(), "a,b\n1,2\n");
}

#[test]
fn refuses_to_overwrite_without_force() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("out.csv");
    fs::write(&destination, "old").unwrap();

    let error = SafeWriter::create(&destination, false, false).unwrap_err();
    assert_eq!(error.exit_code(), tabmux::error::ExitCode::UserData);
    assert_eq!(fs::read_to_string(&destination).unwrap(), "old");
}

#[test]
fn force_overwrites_and_backup_keeps_old_contents() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("out.csv");
    fs::write(&destination, "old").unwrap();

    let mut writer = SafeWriter::create(&destination, true, true).unwrap();
    writer.write_all(b"new").unwrap();
    writer.commit().unwrap();

    assert_eq!(fs::read_to_string(&destination).unwrap(), "new");
    let backups: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.path().extension() == Some(OsStr::new("bak")))
        .collect();
    assert_eq!(backups.len(), 1);
    assert_eq!(fs::read_to_string(backups[0].path()).unwrap(), "old");
}

#[test]
fn dropping_without_commit_leaves_no_output() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("out.csv");

    {
        let mut writer = SafeWriter::create(&destination, false, false).unwrap();
        writer.write_all(b"partial").unwrap();
    }

    assert!(!destination.exists());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}
