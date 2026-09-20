import { createServer } from "node:http";
import { once } from "node:events";
import { pathToFileURL } from "node:url";

export const WORKLOADS = [1000, 10000, 100000, 250000, 500000];
export function entry(index, count, revision = 0) {
  const replacement = revision === 1 && index % 100 === 1;
  const id = replacement ? index + count : index;
  const family = Math.floor(id / 3);
  const kind = family % 10 < 2 ? "live" : family % 10 < 7 ? "movie" : "series";
  const language = ["FR", "EN", "AR"][id % 3];
  const quality = ["HD", "FHD", "UHD"][id % 3];
  const title = ["Voyage", "Journey", "رحلة"][family % 3];
  const changed = revision === 1 && index % 100 === 0 ? " Updated" : "";
  const label =
    id % 997 === 0
      ? `???${changed}`
      : `${title} ${family}${changed}${kind === "series" ? " S01E01" : kind === "movie" ? " (2024)" : ""}`;
  return `#EXTINF:-1 tvg-id="fixture-${family}" group-title="${language} | ${kind}",${label} ${quality}\nhttp://127.0.0.1:9/${kind}/fixture/fixture/${id}.mp4\n`;
}

export function startProvider(port = 0) {
  const server = createServer(async (req, res) => {
    const url = new URL(req.url, "http://127.0.0.1");
    const count = Number(url.searchParams.get("count") ?? 100000);
    const revision = Number(url.searchParams.get("revision") ?? 0);
    if (
      url.pathname !== "/playlist.m3u" ||
      !WORKLOADS.includes(count) ||
      ![0, 1].includes(revision)
    ) {
      res.writeHead(400).end("Use /playlist.m3u?count=100000&revision=0");
      return;
    }
    res.writeHead(200, {
      "Content-Type": "audio/x-mpegurl; charset=utf-8",
      "Cache-Control": "no-store",
    });
    res.write("#EXTM3U\n");
    try {
      for (let index = 0; index < count; index++) {
        if (res.destroyed) return;
        if (!res.write(entry(index, count, revision))) {
          // A disconnected consumer must not strand a producer waiting on drain.
          await new Promise((resolve) => {
            const ready = () => {
              res.off("drain", ready);
              res.off("close", ready);
              resolve();
            };
            res.once("drain", ready);
            res.once("close", ready);
          });
        }
      }
      res.end();
    } catch {
      res.destroy();
    }
  });
  server.listen(port, "127.0.0.1");
  return server;
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const server = startProvider(Number(process.argv[2] ?? 0));
  await once(server, "listening");
  console.log(
    `Synthetic scale provider: http://127.0.0.1:${server.address().port}/playlist.m3u?count=100000`,
  );
  console.log("Generated media URLs deliberately do not serve playable content.");
}
