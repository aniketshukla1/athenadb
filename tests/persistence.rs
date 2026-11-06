//! Phase 1.2 exit criteria: durability across restarts, snapshots, and torn
//! WAL writes (the `kill -9` mid-write signature).

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use athenadb::core::{DbError, Entry};
use athenadb::db::Database;
use athenadb::snapshot::SNAPSHOT_FILE;
use athenadb::wal::LOCK_FILE;

fn entry(id: &str, v: Vec<f32>) -> Entry {
    Entry {
        id: id.into(),
        vector: v,
        metadata: serde_json::json!({ "id": id }),
        namespace: "default".into(),
        embed_model_id: "m1".into(),
        inv_norm: 0.0,
    }
}

#[test]
fn scan_over_database_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut db = Database::open(dir.path()).unwrap();
        db.add(entry("a", vec![1.0, 0.0])).unwrap();
        db.add(entry("b", vec![0.0, 1.0])).unwrap();
        assert_eq!(db.scan(None, None, None, 0).len(), 2);
        assert_eq!(db.scan(Some("default"), None, None, 0).len(), 2);
        assert!(db.scan(Some("other"), None, None, 0).is_empty());
        assert_eq!(db.scan(None, None, Some(1), 0).len(), 1); // pagination
    }
    // scan works the same after recovery from the WAL
    let db = Database::open(dir.path()).unwrap();
    let rows = db.scan(Some("default"), None, None, 0);
    assert_eq!(rows.len(), 2);
    let ids: Vec<&str> = rows.iter().map(|e| e.id.as_str()).collect();
    assert!(ids.contains(&"a") && ids.contains(&"b"));
}

#[test]
fn reopen_recovers_from_wal() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut db = Database::open(dir.path()).unwrap();
        db.add(entry("a", vec![1.0, 0.0, 0.0])).unwrap();
        db.add(entry("b", vec![0.0, 1.0, 0.0])).unwrap();
        assert_eq!(db.len(), 2);
    } // dropped without an explicit snapshot

    let db = Database::open(dir.path()).unwrap();
    assert_eq!(db.len(), 2);
    let hits = db.search(&[1.0, 0.0, 0.0], 1, None, None).unwrap();
    assert_eq!(hits[0].id, "a");
}

#[test]
fn snapshot_then_recover() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut db = Database::open(dir.path()).unwrap();
        db.add(entry("a", vec![1.0, 0.0])).unwrap();
        db.snapshot().unwrap(); // capture + truncate WAL
        db.add(entry("b", vec![0.0, 1.0])).unwrap(); // post-snapshot, lives in WAL
    }
    let db = Database::open(dir.path()).unwrap();
    assert_eq!(db.len(), 2);
    assert_eq!(
        db.search(&[0.0, 1.0], 1, None, None).unwrap()[0].id,
        "b"
    );
}

#[test]
fn delete_persists() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut db = Database::open(dir.path()).unwrap();
        db.add(entry("a", vec![1.0, 0.0])).unwrap();
        db.add(entry("b", vec![0.0, 1.0])).unwrap();
        assert!(db.delete("a").unwrap());
    }
    let db = Database::open(dir.path()).unwrap();
    assert_eq!(db.len(), 1);
    let hits = db.search(&[1.0, 0.0], 5, None, None).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "b");
}

#[test]
fn torn_wal_tail_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut db = Database::open(dir.path()).unwrap();
        db.add(entry("a", vec![1.0, 0.0, 0.0])).unwrap();
    }
    // Simulate a crash mid-write: a frame header claiming a payload that was
    // never flushed.
    let mut f = OpenOptions::new()
        .append(true)
        .open(dir.path().join("wal.log"))
        .unwrap();
    f.write_all(&(9999u64).to_le_bytes()).unwrap();
    f.sync_all().unwrap();

    let db = Database::open(dir.path()).unwrap();
    assert_eq!(db.len(), 1); // torn frame ignored, "a" intact
}

#[test]
fn dim_mismatch_does_not_pollute_wal() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut db = Database::open(dir.path()).unwrap();
        db.add(entry("a", vec![1.0, 0.0])).unwrap();
        assert!(db.add(entry("b", vec![1.0, 0.0, 0.0])).is_err());
    }
    // Reopen must succeed and contain only the valid entry.
    let db = Database::open(dir.path()).unwrap();
    assert_eq!(db.len(), 1);
}

#[test]
fn snapshot_interval_triggers_automatically() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut db = Database::open(dir.path()).unwrap().with_snapshot_interval(3);
        for i in 0..5 {
            db.add(entry(&format!("e{i}"), vec![i as f32, 1.0])).unwrap();
        }
        assert_eq!(db.len(), 5);
    }
    // A snapshot file should exist, and reopen recovers all 5.
    assert!(dir.path().join("snapshot.bin").exists());
    let db = Database::open(dir.path()).unwrap();
    assert_eq!(db.len(), 5);
}

#[test]
fn batch_add_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut db = Database::open(dir.path()).unwrap();
        db.add_batch(vec![entry("a", vec![1.0, 0.0]), entry("b", vec![0.0, 1.0]), entry("c", vec![1.0, 1.0])])
            .unwrap();
        assert_eq!(db.len(), 3);
    }
    let db = Database::open(dir.path()).unwrap();
    assert_eq!(db.len(), 3);
    assert_eq!(db.search(&[0.0, 1.0], 1, None, None).unwrap()[0].id, "b");
}

#[test]
fn batch_with_a_bad_dimension_logs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut db = Database::open(dir.path()).unwrap();
        let err = db.add_batch(vec![entry("a", vec![1.0, 0.0]), entry("b", vec![1.0, 0.0, 0.0])]);
        assert!(err.is_err());
        assert_eq!(db.len(), 0);
    }
    assert_eq!(Database::open(dir.path()).unwrap().len(), 0);
}

#[test]
fn torn_tail_after_a_batch_keeps_the_intact_frames() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut db = Database::open(dir.path()).unwrap();
        db.add_batch(vec![entry("a", vec![1.0, 0.0]), entry("b", vec![0.0, 1.0])]).unwrap();
    }
    let mut wal = OpenOptions::new().append(true).open(dir.path().join("wal.log")).unwrap();
    wal.write_all(&[255, 0, 0, 0, 0, 0, 0, 0, 9, 9]).unwrap();
    drop(wal);
    assert_eq!(Database::open(dir.path()).unwrap().len(), 2);
}

#[test]
fn read_only_sees_the_data_and_refuses_writes() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut db = Database::open(dir.path()).unwrap();
        db.add(entry("a", vec![1.0, 0.0])).unwrap();
    }
    let mut db = Database::open_read_only(dir.path()).unwrap();
    assert!(db.is_read_only());
    assert_eq!(db.search(&[1.0, 0.0], 1, None, None).unwrap()[0].id, "a");
    assert!(matches!(db.add(entry("b", vec![0.0, 1.0])), Err(DbError::ReadOnly)));
    assert!(matches!(db.delete("a"), Err(DbError::ReadOnly)));
    assert!(matches!(db.snapshot(), Err(DbError::ReadOnly)));
    assert_eq!(db.len(), 1);
}

#[test]
fn read_only_never_creates_a_store() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope");
    assert!(Database::open_read_only(&missing).is_err());
    assert!(!missing.exists());

    // An existing empty directory opens, and stays empty.
    let empty = dir.path().join("empty");
    std::fs::create_dir(&empty).unwrap();
    assert_eq!(Database::open_read_only(&empty).unwrap().len(), 0);
    assert_eq!(std::fs::read_dir(&empty).unwrap().count(), 0);
}

#[test]
fn read_only_notices_a_writer_moving_on() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = Database::open(dir.path()).unwrap();
    writer.add(entry("a", vec![1.0, 0.0])).unwrap();

    let reader = Database::open_read_only(dir.path()).unwrap();
    assert!(!reader.stats().disk_changed);
    writer.add(entry("b", vec![0.0, 1.0])).unwrap();
    assert!(reader.stats().disk_changed);
    assert_eq!(reader.len(), 1);
    assert_eq!(Database::open_read_only(dir.path()).unwrap().len(), 2);
}

#[test]
fn stats_describe_groups_and_disk() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Database::open(dir.path()).unwrap();
    let mut other = entry("c", vec![0.0, 0.0]);
    other.namespace = "other".into();
    db.add_batch(vec![entry("a", vec![1.0, 0.0]), entry("b", vec![0.0, 1.0]), other]).unwrap();

    let stats = db.stats();
    assert_eq!(stats.vectors, 3);
    assert_eq!(stats.dimensions, Some(2));
    assert_eq!(stats.zero_norm_vectors, 1);
    assert_eq!(stats.groups[0].namespace, "default");
    assert_eq!(stats.groups[0].vectors, 2);
    assert_eq!(stats.groups[1].namespace, "other");
    assert!(stats.disk.wal_bytes > 0);
    assert_eq!(stats.disk.snapshot_bytes, 0);
    assert_eq!(stats.ops_since_snapshot, 3);
    assert!(!stats.read_only);

    db.snapshot().unwrap();
    let stats = db.stats();
    assert_eq!(stats.disk.wal_bytes, 0);
    assert!(stats.disk.snapshot_bytes > 0);
    assert!(stats.disk.snapshot_modified_unix.is_some());
    assert_eq!(stats.ops_since_snapshot, 0);
}

#[test]
fn second_writer_is_refused_and_the_first_keeps_working() {
    let dir = tempfile::tempdir().unwrap();
    let mut first = Database::open(dir.path()).unwrap();
    first.add(entry("a", vec![1.0, 0.0])).unwrap();

    let err = Database::open(dir.path()).err().expect("a second writer must be refused");
    assert!(matches!(err, DbError::Locked { .. }), "{err}");
    assert!(err.to_string().contains(&dir.path().display().to_string()), "names the store: {err}");

    first.add(entry("b", vec![0.0, 1.0])).unwrap();
    first.snapshot().unwrap();
    assert!(
        matches!(Database::open(dir.path()), Err(DbError::Locked { .. })),
        "a snapshot must not give up the lock"
    );
    first.add(entry("c", vec![1.0, 1.0])).unwrap();
    drop(first);
    assert_eq!(Database::open(dir.path()).unwrap().len(), 3);
}

#[test]
fn a_refused_writer_reads_nothing_from_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let _first = Database::open(dir.path()).unwrap();
    std::fs::write(dir.path().join(SNAPSHOT_FILE), b"not a snapshot").unwrap();

    // Loading would fail on this file, so Locked proves the lock came first. A writer that
    // loaded and only then locked could snapshot over what the last holder wrote in between.
    assert!(matches!(Database::open(dir.path()), Err(DbError::Locked { .. })));
}

#[test]
fn reopening_works_once_the_writer_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    drop(Database::open(dir.path()).unwrap());

    let _second = Database::open(dir.path()).unwrap();
    assert!(
        matches!(Database::open(dir.path()), Err(DbError::Locked { .. })),
        "the reopened writer must hold the store in turn"
    );
}

/// A lock can outlive its holder for a moment: while any thread is spawning a process,
/// the child holds a copy of every open file until it execs.
#[test]
fn a_lock_that_lingers_for_a_moment_is_waited_out() {
    let dir = tempfile::tempdir().unwrap();
    let first = Database::open(dir.path()).unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(20));
        drop(first);
    });

    assert!(Database::open(dir.path()).is_ok());
    release.join().unwrap();
}

#[test]
fn read_only_opens_beside_a_writer_and_stays_out_of_its_way() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = Database::open(dir.path()).unwrap();
    writer.add(entry("a", vec![1.0, 0.0])).unwrap();

    let reader = Database::open_read_only(dir.path()).unwrap();
    assert_eq!(reader.len(), 1);
    writer.add(entry("b", vec![0.0, 1.0])).unwrap();
    drop(writer);

    // The reader holds nothing, so the next writer can start while it's still open.
    assert_eq!(Database::open(dir.path()).unwrap().len(), 2);
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_store_from_before_locks_stays_untouched_until_a_writer_opens_it() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut db = Database::open(dir.path()).unwrap();
        db.add(entry("a", vec![1.0, 0.0])).unwrap();
    }
    std::fs::remove_file(dir.path().join(LOCK_FILE)).unwrap();
    let before = files_in(dir.path());

    let reader = Database::open_read_only(dir.path()).unwrap();
    assert_eq!(reader.len(), 1);
    assert_eq!(files_in(dir.path()), before, "read-only must create nothing");

    assert_eq!(Database::open(dir.path()).unwrap().len(), 1);
    assert!(dir.path().join(LOCK_FILE).exists());
}

/// Not a test of its own: the process that `a_killed_writer_leaves_no_stale_lock`
/// starts, then kills.
#[test]
#[ignore = "started by a_killed_writer_leaves_no_stale_lock"]
fn hold_the_store_until_killed() {
    let Ok(dir) = std::env::var("ATHENADB_TEST_STORE") else { return };
    let mut db = Database::open(dir).unwrap();
    db.add(entry("a", vec![1.0, 0.0])).unwrap();
    println!("holding");
    std::thread::sleep(std::time::Duration::from_secs(120));
}

#[test]
fn a_killed_writer_leaves_no_stale_lock() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "hold_the_store_until_killed", "--ignored", "--nocapture"])
        .env("ATHENADB_TEST_STORE", dir.path())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let output = BufReader::new(writer.stdout.take().unwrap());
    assert!(output.lines().map_while(Result::ok).any(|line| line.contains("holding")));

    let refused = matches!(Database::open(dir.path()), Err(DbError::Locked { .. }));
    writer.kill().unwrap(); // SIGKILL, so nothing gets to clean up.
    writer.wait().unwrap();

    assert!(refused, "another process's writer must be refused");
    assert_eq!(Database::open(dir.path()).unwrap().len(), 1);
}
