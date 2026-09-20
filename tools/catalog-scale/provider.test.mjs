import assert from "node:assert/strict";
import { test } from "node:test";
import { once } from "node:events";
import { entry, startProvider } from "./provider.mjs";

test("100k refresh preserves exactly 98% and changes 2%", () => {
  let same = 0,
    changed = 0,
    replaced = 0;
  for (let i = 0; i < 100000; i++) {
    const a = entry(i, 100000),
      b = entry(i, 100000, 1);
    if (a === b) same++;
    else if (a.split("\n")[1] === b.split("\n")[1]) changed++;
    else replaced++;
  }
  assert.deepEqual({ same, changed, replaced }, { same: 98000, changed: 1000, replaced: 1000 });
});
test("provider streams the requested count and rejects unbounded workloads", async () => {
  const server = startProvider();
  await once(server, "listening");
  try {
    const root = `http://127.0.0.1:${server.address().port}`;
    const result = await fetch(`${root}/playlist.m3u?count=1000`);
    const text = await result.text();
    assert.equal(text.match(/#EXTINF/g).length, 1000);
    assert.ok(text.includes("رحلة"));
    assert.ok(text.includes("/movie/"));
    assert.ok(text.includes("/series/"));
    assert.ok(text.includes("/live/"));
    assert.equal((await fetch(`${root}/playlist.m3u?count=999999999`)).status, 400);
  } finally {
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
  }
});
