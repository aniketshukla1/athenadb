"""Phase 1.3: PyO3 binding — add/query/delete/snapshot and on-disk persistence.

Skipped automatically if the native module hasn't been built yet
(`maturin develop --release --features python -m athenadb/Cargo.toml`).
"""

from __future__ import annotations

import json

import pytest

athenadb = pytest.importorskip("athenadb")


def test_add_and_query_orders_by_cosine(tmp_path):
    db = athenadb.AthenaDB(str(tmp_path / "db"))
    db.add(
        ids=["x", "y", "z"],
        vectors=[[1.0, 0.0, 0.0], [0.9, 0.1, 0.0], [0.0, 1.0, 0.0]],
        metadatas=[json.dumps({"label": "x"}), json.dumps({"label": "y"}), json.dumps({"label": "z"})],
        namespace="default",
        embed_model_id="bge-small",
    )
    assert len(db) == 3

    hits = db.query([1.0, 0.0, 0.0], 2)
    assert len(hits) == 2
    ids = [h[0] for h in hits]
    assert ids == ["x", "y"]
    # tuple shape: (id, score, namespace, embed_model_id, metadata_json)
    top = hits[0]
    assert top[1] > 0.99
    assert top[2] == "default"
    assert top[3] == "bge-small"
    assert json.loads(top[4]) == {"label": "x"}


def test_namespace_filter(tmp_path):
    db = athenadb.AthenaDB(str(tmp_path / "db"))
    db.add(ids=["a"], vectors=[[1.0, 0.0]], namespace="team-a", embed_model_id="m")
    db.add(ids=["b"], vectors=[[1.0, 0.0]], namespace="team-b", embed_model_id="m")
    hits = db.query([1.0, 0.0], 5, "team-a")
    assert [h[0] for h in hits] == ["a"]


def test_delete(tmp_path):
    db = athenadb.AthenaDB(str(tmp_path / "db"))
    db.add(ids=["a", "b"], vectors=[[1.0, 0.0], [0.0, 1.0]])
    removed = db.delete(["a", "missing"])
    assert removed == 1
    assert len(db) == 1


def test_persistence_across_reopen(tmp_path):
    path = str(tmp_path / "db")
    db = athenadb.AthenaDB(path)
    db.add(ids=["a", "b"], vectors=[[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], embed_model_id="m")
    del db  # drop the handle; data is durable in the WAL

    reopened = athenadb.AthenaDB(path)
    assert len(reopened) == 2
    assert reopened.query([1.0, 0.0, 0.0], 1)[0][0] == "a"


def test_snapshot_then_reopen(tmp_path):
    path = str(tmp_path / "db")
    db = athenadb.AthenaDB(path)
    db.add(ids=["a"], vectors=[[1.0, 0.0]])
    db.snapshot()
    db.add(ids=["b"], vectors=[[0.0, 1.0]])
    del db

    reopened = athenadb.AthenaDB(path)
    assert len(reopened) == 2


def test_dimension_mismatch_raises(tmp_path):
    db = athenadb.AthenaDB(str(tmp_path / "db"))
    db.add(ids=["a"], vectors=[[1.0, 0.0]])
    with pytest.raises(Exception):
        db.add(ids=["b"], vectors=[[1.0, 0.0, 0.0]])


def test_add_documents_shim(tmp_path):
    db = athenadb.AthenaDB(str(tmp_path / "db"))
    db.add_documents(ids=["a"], embeddings=[[1.0, 0.0]], embed_model_id="m")
    assert len(db) == 1


def test_scan_filters_and_paginates(tmp_path):
    db = athenadb.AthenaDB(str(tmp_path / "db"))
    for i in range(5):
        db.add(ids=[f"a{i}"], vectors=[[1.0, 0.0]], namespace="ns-a", embed_model_id="m")
    for i in range(3):
        db.add(ids=[f"b{i}"], vectors=[[0.0, 1.0]], namespace="ns-b", embed_model_id="m")

    # namespace filter; tuple shape is (id, namespace, embed_model_id, metadata_json)
    rows = db.scan(namespace="ns-a")
    assert len(rows) == 5
    assert all(r[1] == "ns-a" and r[2] == "m" for r in rows)
    assert len(db.scan(namespace="ns-b")) == 3
    assert len(db.scan()) == 8  # no filter -> all

    # embedding-model filter
    assert len(db.scan(embed_model_id="m")) == 8
    assert db.scan(embed_model_id="other") == []

    # pagination
    assert len(db.scan(namespace="ns-a", limit=2)) == 2
    assert len(db.scan(namespace="ns-a", limit=2, offset=4)) == 1
    assert db.scan(namespace="ns-a", offset=100) == []


def test_scan_metadata_round_trips(tmp_path):
    db = athenadb.AthenaDB(str(tmp_path / "db"))
    db.add(
        ids=["m1"],
        vectors=[[1.0, 0.0]],
        metadatas=[json.dumps({"created_at": 123, "kind": "episode"})],
        namespace="episodic",
        embed_model_id="m",
    )
    row = db.scan(namespace="episodic")[0]
    assert row[0] == "m1"
    assert json.loads(row[3]) == {"created_at": 123, "kind": "episode"}


def test_scan_survives_reopen(tmp_path):
    path = str(tmp_path / "db")
    db = athenadb.AthenaDB(path)
    db.add(ids=["a", "b"], vectors=[[1.0, 0.0], [0.0, 1.0]], embed_model_id="m")
    del db
    reopened = athenadb.AthenaDB(path)
    assert len(reopened.scan()) == 2
