//! A durable store in a few lines. Run with: cargo run --release --example quickstart

use athenadb::core::Entry;
use athenadb::db::Database;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join("athenadb-quickstart");
    let _ = std::fs::remove_dir_all(&dir);

    let mut db = Database::open(&dir)?;
    for (id, vector) in [("cat", [0.9, 0.1, 0.0]), ("kitten", [0.85, 0.2, 0.0]), ("car", [0.0, 0.1, 0.95])] {
        db.add(Entry {
            id: id.into(),
            vector: vector.to_vec(),
            metadata: serde_json::json!({ "source": "quickstart" }),
            namespace: "pets".into(),
            embed_model_id: "bge-small".into(),
            inv_norm: 0.0, // computed on insert
        })?;
    }

    for hit in db.search(&[1.0, 0.0, 0.0], 2, Some("pets"), Some("bge-small"))? {
        println!("{} {:.3}", hit.id, hit.score);
    }

    drop(db); // every add was fsynced, so reopening recovers everything
    println!("{} vectors after reopen", Database::open(&dir)?.len());
    Ok(())
}
