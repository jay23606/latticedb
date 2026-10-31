#!/usr/bin/env python3
"""pg8000 smoke script for latticed, text mode (issue 710).

Runs against the server named by LATTICE_DSN (a postgresql:// URL) and prints one JSON object per step.
tests/drivers/smoke.sh compares the lines with expected.jsonl and known.jsonl. The steps follow the
node-postgres and pgx scripts. The connection runs in autocommit mode, so every statement is its own
transaction, and the explicit BEGIN, COMMIT and ROLLBACK steps are written out as node/smoke.mjs does.
pg8000 sends every bind parameter in text format (its bind message carries no format codes) and maps
int8 to a Python int.

The last DDL step uses the DBAPI default instead (autocommit off). pg8000 then sends `begin transaction`
before its first statement, so that step shows whether latticed accepts DDL inside that block (#382).
"""

import json
import os
import sys
from urllib.parse import parse_qs, urlsplit

import pg8000.dbapi

DSN = os.environ.get("LATTICE_DSN")
if not DSN:
    print("LATTICE_DSN is not set", file=sys.stderr)
    sys.exit(2)

INSERT = "INSERT INTO drv_smoke VALUES (%s, %s, %s, %s)"


def connect(dsn):
    parts = urlsplit(dsn)
    if parts.scheme != "postgresql" or not parts.username or not parts.hostname or not parts.port:
        raise SystemExit("LATTICE_DSN must look like postgresql://USER@HOST:PORT/DATABASE")
    if parse_qs(parts.query).get("sslmode") != ["disable"]:
        raise SystemExit("LATTICE_DSN must set sslmode=disable: this script runs over loopback without TLS")
    return pg8000.dbapi.connect(
        user=parts.username,
        host=parts.hostname,
        port=parts.port,
        database=parts.path.lstrip("/"),
    )


def emit(record):
    print(json.dumps(record), flush=True)


def step(name, tags, fn):
    try:
        record = fn()
    except pg8000.dbapi.DatabaseError as err:
        fields = err.args[0] if err.args and isinstance(err.args[0], dict) else {}
        record = {"sqlstate": fields.get("C", "")}
    except Exception as err:
        # The first line only, as the node script records it.
        record = {"client_error": str(err).split("\n")[0]}
    emit({"step": name, "tags": tags, **record})


def rows():
    return [list(row) for row in cur.fetchall()]


def select_literals():
    cur.execute("SELECT 1 AS one, 'x' AS s, true AS b")
    return {"rows": rows()}


def create_table():
    cur.execute("CREATE TABLE drv_smoke (id INT PRIMARY KEY, name TEXT NOT NULL, qty BIGINT, ok BOOLEAN)")
    return {"ok": True}


def insert_bolt():
    cur.execute(INSERT, (1, "bolt", 9000000000, True))
    return {"rowcount": cur.rowcount}


def insert_nut_null():
    cur.execute(INSERT, (2, "nut", None, False))
    return {"rowcount": cur.rowcount}


def select_with_params():
    cur.execute("SELECT id, name, qty, ok FROM drv_smoke WHERE id = %s OR name = %s ORDER BY id", (2, "bolt"))
    return {"rows": rows()}


def update_with_params():
    cur.execute("UPDATE drv_smoke SET qty = %s WHERE id = %s", (7, 1))
    return {"rowcount": cur.rowcount}


def delete_with_params():
    cur.execute("DELETE FROM drv_smoke WHERE id = %s", (2,))
    return {"rowcount": cur.rowcount}


def transaction_commit():
    cur.execute("BEGIN")
    try:
        cur.execute(INSERT, (3, "pin", 4, True))
        rowcount = cur.rowcount
    except Exception:
        cur.execute("ROLLBACK")
        raise
    cur.execute("COMMIT")
    return {"rowcount": rowcount}


def select_after_commit():
    cur.execute("SELECT id FROM drv_smoke ORDER BY id")
    return {"rows": rows()}


def transaction_rollback():
    cur.execute("BEGIN")
    try:
        cur.execute(INSERT, (4, "gone", 0, True))
        rowcount = cur.rowcount
    finally:
        cur.execute("ROLLBACK")
    return {"rowcount": rowcount}


def select_after_rollback():
    cur.execute("SELECT id FROM drv_smoke WHERE id = %s", (4,))
    return {"rows": rows()}


def error_duplicate_key():
    cur.execute(INSERT, (1, "dup", None, None))
    return {}


def error_syntax():
    cur.execute("SELECT FROM FROM drv_smoke")
    return {}


def select_after_errors():
    cur.execute("SELECT id FROM drv_smoke ORDER BY id")
    return {"rows": rows()}


def drop_table():
    cur.execute("DROP TABLE drv_smoke")
    return {"ok": True}


def ddl_in_implicit_transaction():
    # DBAPI default: autocommit off, so pg8000 sends `begin transaction` before its first statement.
    conn.autocommit = False
    try:
        cur.execute("CREATE TABLE drv_probe (id INT)")
        conn.commit()
        cur.execute("DROP TABLE drv_probe")
        conn.commit()
    except Exception:
        conn.rollback()
        raise
    finally:
        conn.autocommit = True
    return {"ok": True}


conn = connect(DSN)
conn.autocommit = True
cur = conn.cursor()

step("select_literals", ["select", "int", "text", "bool"], select_literals)
step("create_table", ["ddl"], create_table)
step("insert_bolt", ["dml", "parameter", "int", "text", "bool", "bigint"], insert_bolt)
step("insert_nut_null", ["dml", "parameter", "int", "text", "bool", "null"], insert_nut_null)
step(
    "select_with_params",
    ["select", "parameter", "int", "text", "bool", "bigint", "null"],
    select_with_params,
)
step("update_with_params", ["dml", "parameter", "bigint"], update_with_params)
step("delete_with_params", ["dml", "parameter"], delete_with_params)
step("transaction_commit", ["transaction", "dml", "parameter"], transaction_commit)
step("select_after_commit", ["select"], select_after_commit)
step("transaction_rollback", ["transaction", "dml", "parameter"], transaction_rollback)
step("select_after_rollback", ["select", "parameter"], select_after_rollback)
step("error_duplicate_key", ["error", "dml", "parameter", "null"], error_duplicate_key)
step("error_syntax", ["error"], error_syntax)
step("select_after_errors", ["select"], select_after_errors)
step("drop_table", ["ddl"], drop_table)
step("ddl_in_implicit_transaction", ["ddl", "transaction"], ddl_in_implicit_transaction)

conn.close()
emit({"step": "close", "tags": ["close"], "ok": True})
