//! Phase 1.2 exit criteria: durability across restarts, snapshots, and torn
//! WAL writes (the `kill -9` mid-write signature).

use std::fs::OpenOptions;
use std::io::Write;

use athenadb::core::Entry;
use athenadb::db::Database;

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
