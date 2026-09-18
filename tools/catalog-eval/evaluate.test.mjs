import assert from "node:assert/strict";
import { readFile, mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { once } from "node:events";
import test from "node:test";
import {
  evaluate,
  LocalReasoner,
  requestBody,
  validateCorpus,
  validOutput,
  verifyArtifact,
  PROMPT,
  PROMPTS,
  SCHEMA,
} from "./evaluate.mjs";

test("Rust reasoner uses the evaluated v2 prompt and schema", async () => {
  const protocol = JSON.parse(
    await readFile(new URL("../../crates/catalog-reasoner/protocol.json", import.meta.url)),
  );
  assert.equal(protocol.promptVersion, "v2");
  assert.equal(protocol.prompt, PROMPTS.v2);
  assert.deepEqual(protocol.schema, SCHEMA);
});

test("versioned diagnostic prompts preserve the original comparison", () => {
  assert.equal(requestBody({}).messages[0].content, PROMPT);
  assert.notEqual(requestBody({}, "v2").messages[0].content, PROMPT);
  assert.equal(requestBody({}, "v2").messages[0].content, PROMPTS.v2);
  assert.throws(() => requestBody({}, "unknown"));
  assert.throws(() => new LocalReasoner(18491, 1000, "unknown"));
});

test("artifact verification rejects missing or tampered model files", async () => {
  const directory = await mkdtemp(join(tmpdir(), "nova-catalog-eval-"));
  const path = join(directory, "fixture.txt");
  try {
    const content = "synthetic artifact";
    const artifact = { path, sha256: createHash("sha256").update(content).digest("hex") };
    await assert.rejects(verifyArtifact(artifact));
    await writeFile(path, content);
    await verifyArtifact(artifact);
    await writeFile(path, "modified artifact");
    await assert.rejects(verifyArtifact(artifact));
  } finally {
    await rm(directory, { recursive: true });
  }
});

const corpus = JSON.parse(
  await readFile(new URL("../../fixtures/catalog-intelligence/v1/smoke.json", import.meta.url)),
);
const known = corpus.cases[0];

test("frozen transfer cases use different title families from development smoke", async () => {
  const transfer = JSON.parse(
    await readFile(
      new URL("../../fixtures/catalog-intelligence/v1/transfer.json", import.meta.url),
    ),
  );
  validateCorpus(transfer);
  const developmentFamilies = new Set(corpus.cases.map((item) => item.family));
  assert.ok(transfer.cases.every((item) => !developmentFamilies.has(item.family)));
  assert.equal(transfer.cases.length, 12);
  assert.equal(transfer.cases.filter((item) => item.expected.candidate_id === null).length, 6);
});

test("synthetic corpus covers ambiguity, languages and negative identity cases", () => {
  validateCorpus(corpus);
  assert.ok(corpus.cases.some((c) => c.input.title.includes("العربية")));
  assert.ok(corpus.cases.filter((c) => c.expected.candidate_id === null).length >= 5);
  assert.throws(() => validateCorpus({ ...corpus, cases: [known, known] }));
});

test("prompts contain input but never the evaluation answer", () => {
  const body = requestBody(known.input);
  assert.deepEqual(JSON.parse(body.messages[1].content), known.input);
  assert.ok(!body.messages[1].content.includes('"expected"'));
  assert.equal(body.response_format.json_schema.schema.additionalProperties, false);
});

test("invented identities, unsupported taxonomy and extra fields fail shape validation", () => {
  for (const output of [
    { ...known.expected, candidate_id: "fabricated" },
    { ...known.expected, canonical_name: "Other" },
    { ...known.expected, entity_type: "made_up" },
    { ...known.expected, authority: "official" },
  ])
    assert.equal(validOutput(output, known.input.candidates), false);
});

test("metrics distinguish false links, abstention and runtime failure", async () => {
  let count = 0;
  const result = await evaluate(
    { ...corpus, cases: corpus.cases.slice(0, 3) },
    {
      resolve: async () => {
        count++;
        if (count === 1) return { ...known.expected, candidate_id: "invented" };
        if (count === 2)
          return {
            canonical_name: null,
            entity_type: "unknown",
            candidate_id: null,
            quality_hint: "unknown",
          };
        throw new Error("not included in report");
      },
    },
  );
  assert.equal(result.summary.falseLinks, 1);
  assert.equal(result.summary.missedIdentities, 1);
  assert.equal(result.summary.schemaFailures, 2);
  assert.ok(!JSON.stringify(result).includes("not included in report"));
});

async function withServer(handler, run) {
  const server = createServer(handler);
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  try {
    await run(server.address().port);
  } finally {
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
  }
}

test("local adapter parses a complete response", async () => {
  await withServer(
    (_req, res) =>
      res.end(
        JSON.stringify({
          choices: [
            {
              finish_reason: "stop",
              message: {
                content: JSON.stringify(known.expected),
              },
            },
          ],
        }),
      ),
    async (port) => {
      assert.deepEqual(await new LocalReasoner(port).resolve(known.input), known.expected);
    },
  );
});

test("local adapter rejects redirects, truncated output, oversized bodies and timeout", async () => {
  for (const handler of [
    (_req, res) => {
      res.writeHead(302, { location: "http://example.com" });
      res.end();
    },
    (_req, res) =>
      res.end(
        JSON.stringify({
          choices: [{ finish_reason: "length", message: { content: "{}" } }],
        }),
      ),
    (_req, res) => res.end(" ".repeat(65537)),
    () => {},
  ])
    await withServer(handler, async (port) => {
      await assert.rejects(new LocalReasoner(port, 200).resolve(known.input));
    });
  assert.throws(() => new LocalReasoner(80));
  assert.throws(() => new LocalReasoner(18491, Infinity));
});
