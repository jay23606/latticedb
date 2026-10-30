// node-postgres (pg) smoke script for latticed, text mode (issue 576).
//
// Runs against the server named by LATTICE_DSN (a postgresql:// URL) and prints one JSON object per
// step. tests/drivers/smoke.sh compares the lines with expected.jsonl. pg sends a query with values
// through the extended protocol and always asks for text results, so no step needs a known-failure
// pin. int8 (bigint) values come back as strings, as pg documents, and expected.jsonl says so.
import pg from "pg";

const DSN = process.env.LATTICE_DSN;
if (!DSN) {
  console.error("LATTICE_DSN is not set");
  process.exit(2);
}

const INSERT = "INSERT INTO drv_smoke VALUES ($1, $2, $3, $4)";

function emit(record) {
  console.log(JSON.stringify(record));
}

async function step(name, tags, fn) {
  let record;
  try {
    record = await fn();
  } catch (err) {
    record = err.code
      ? { sqlstate: err.code }
      : { client_error: String(err.message).split("\n")[0] };
  }
  emit({ step: name, tags, ...record });
}

async function run(client, text, values) {
  return client.query({ text, values, rowMode: "array" });
}

// Runs fn inside BEGIN and COMMIT, or ROLLBACK when fn throws; the error is rethrown.
async function inTransaction(client, fn) {
  await client.query("BEGIN");
  try {
    const result = await fn();
    await client.query("COMMIT");
    return result;
  } catch (err) {
    await client.query("ROLLBACK");
    throw err;
  }
}

const client = new pg.Client({ connectionString: DSN, ssl: false });
await client.connect();

await step("select_literals", ["select", "int", "text", "bool"], async () => {
  const res = await run(client, "SELECT 1 AS one, 'x' AS s, true AS b");
  return { rows: res.rows };
});

await step("create_table", ["ddl"], async () => {
  await client.query(
    "CREATE TABLE drv_smoke (id INT PRIMARY KEY, name TEXT NOT NULL, qty BIGINT, ok BOOLEAN)",
  );
  return { ok: true };
});

await step("insert_bolt", ["dml", "parameter", "int", "text", "bool", "bigint"], async () => {
  const res = await run(client, INSERT, [1, "bolt", 9000000000, true]);
  return { rowcount: res.rowCount };
});

await step("insert_nut_null", ["dml", "parameter", "int", "text", "bool", "null"], async () => {
  const res = await run(client, INSERT, [2, "nut", null, false]);
  return { rowcount: res.rowCount };
});

await step(
  "select_with_params",
  ["select", "parameter", "int", "text", "bool", "bigint", "null"],
  async () => {
    const res = await run(
      client,
      "SELECT id, name, qty, ok FROM drv_smoke WHERE id = $1 OR name = $2 ORDER BY id",
      [2, "bolt"],
    );
    return { rows: res.rows };
  },
);

await step("update_with_params", ["dml", "parameter", "bigint"], async () => {
  const res = await run(client, "UPDATE drv_smoke SET qty = $1 WHERE id = $2", [7, 1]);
  return { rowcount: res.rowCount };
});

await step("delete_with_params", ["dml", "parameter"], async () => {
  const res = await run(client, "DELETE FROM drv_smoke WHERE id = $1", [2]);
  return { rowcount: res.rowCount };
});

await step("transaction_commit", ["transaction", "dml", "parameter"], async () => {
  const res = await inTransaction(client, () => run(client, INSERT, [3, "pin", 4, true]));
  return { rowcount: res.rowCount };
});

await step("select_after_commit", ["select"], async () => {
  const res = await run(client, "SELECT id FROM drv_smoke ORDER BY id");
  return { rows: res.rows };
});

await step("transaction_rollback", ["transaction", "dml", "parameter"], async () => {
  let rowcount;
  await client.query("BEGIN");
  try {
    rowcount = (await run(client, INSERT, [4, "gone", 0, true])).rowCount;
  } finally {
    await client.query("ROLLBACK");
  }
  return { rowcount };
});

await step("select_after_rollback", ["select", "parameter"], async () => {
  const res = await run(client, "SELECT id FROM drv_smoke WHERE id = $1", [4]);
  return { rows: res.rows };
});

await step("error_duplicate_key", ["error", "dml", "parameter", "null"], async () => {
  await run(client, INSERT, [1, "dup", null, null]);
  return {};
});

await step("error_syntax", ["error"], async () => {
  await client.query("SELECT FROM FROM drv_smoke");
  return {};
});

await step("select_after_errors", ["select"], async () => {
  const res = await run(client, "SELECT id FROM drv_smoke ORDER BY id");
  return { rows: res.rows };
});

await step("drop_table", ["ddl"], async () => {
  await client.query("DROP TABLE drv_smoke");
  return { ok: true };
});

await client.end();
emit({ step: "close", tags: ["close"], ok: true });
