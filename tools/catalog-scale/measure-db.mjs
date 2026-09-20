// Read-only SQLite microbenchmark on a generated review profile, not IPC latency.
import { DatabaseSync } from "node:sqlite";
import { performance } from "node:perf_hooks";
import { stat } from "node:fs/promises";
const path = process.argv[2];
if (!path) throw new Error("Usage: node tools/catalog-scale/measure-db.mjs /absolute/profile.db");
const db = new DatabaseSync(path, { readOnly: true });
try {
  const counts = {};
  for (const table of ["channels", "vod_items", "series_episodes"]) {
    counts[table] = db.prepare(`SELECT count(*) AS count FROM ${table}`).get().count;
  }
  const source = db.prepare("SELECT id FROM sources ORDER BY id LIMIT 1").get()?.id;
  const query = db.prepare("SELECT id,name,clean_title FROM vod_items WHERE source_id=? AND name LIKE ? ORDER BY id LIMIT 200 OFFSET ?");
  const timings = [];
  for (let index = 0; index < 60; index++) {
    const start = performance.now();
    const rows = query.all(source, index % 2 === 0 ? "%Voyage%" : "%رحلة%", (index % 4) * 200);
    if (rows.length > 200) throw new Error("unbounded page");
    timings.push(performance.now() - start);
  }
  timings.sort((a,b) => a-b);
  console.log(JSON.stringify({ version: 1, scope: "warm SQLite projection only; not native IPC or semantic search", counts,
    databaseBytes: (await stat(path)).size, samples: timings.length,
    p50Ms: timings[29], p95Ms: timings[56], pageLimit: 200 }, null, 2));
} finally { db.close(); }
