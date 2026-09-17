import test from "node:test";
import assert from "node:assert/strict";
import {
    cargoVersions,
    readInventory,
    validateInventory,
    probeVersion,
    inspectEnvironment,
} from "./stack.mjs";

const inventory = await readInventory();

test("recognizes Node's v-prefixed version output", async () => {
    assert.deepEqual(
        await probeVersion("node", ["--version"], async () => ({
            stdout: "v22.22.3\n",
        })),
        { status: "available", version: "22.22.3" },
    );
});

test("committed stack agrees with versions, locks and migration heads", () => {
    assert.deepEqual(validateInventory(inventory), []);
});

test("reports independent drift instead of silently repairing files", () => {
    const data = structuredClone(inventory);
    data.pkg.version = "99.0.0";
    data.mise = data.mise.replace('node = "22.22.3"', 'node = "latest"');
    data.profile.push("0020_future.sql");
    const issues = validateInventory(data);
    assert(issues.includes("root app version differs from manifest"));
    assert(issues.includes("mise node differs from manifest"));
    assert(issues.includes("Profile migration head differs from manifest"));
});

test("missing lock entries and unsupported manifest fail validation", () => {
    const data = structuredClone(inventory);
    data.manifest.schemaVersion = 2;
    data.cargoLock = "";
    data.pnpmLock = "";
    const issues = validateInventory(data);
    assert(issues.includes("Unsupported stack manifest schemaVersion"));
    assert(
        issues.some((issue) => issue.startsWith("Cargo.lock missing tauri@")),
    );
    assert(
        issues.some((issue) => issue.startsWith("pnpm-lock missing react@")),
    );
});

test("Cargo reader respects package boundaries and multiple versions", () => {
    assert.deepEqual(
        cargoVersions(
            '[[package]]\nname = "x"\nversion = "1.0.0"\n[[package]]\nname = "y"\nversion = "3.0.0"\n[[package]]\nname = "x"\nversion = "2.0.0"',
            "x",
        ),
        ["1.0.0", "2.0.0"],
    );
});

test("runtime probes are shell-free, bounded and do not expose error secrets", async () => {
    const result = await probeVersion(
        "rustc",
        ["--version"],
        async (command, args, options) => {
            assert.equal(command, "rustc");
            assert.deepEqual(args, ["--version"]);
            assert.equal(options.timeout, 5000);
            assert.equal(options.shell, undefined);
            return { stdout: "rustc 1.95.0 (hash date)\n" };
        },
    );
    assert.deepEqual(result, { status: "available", version: "1.95.0" });
    const failed = await probeVersion("tool", [], async () => {
        throw new Error("password=secret");
    });
    assert.deepEqual(failed, { status: "failed" });
    const missing = await probeVersion("tool", [], async () => {
        throw Object.assign(new Error(), { code: "ENOENT" });
    });
    assert.deepEqual(missing, { status: "missing" });
    assert.deepEqual(
        await probeVersion("tool", [], async () => ({ stdout: "unknown" })),
        { status: "unrecognized" },
    );
});

test("doctor distinguishes installed version drift from availability", async () => {
    const tools = await inspectEnvironment(inventory.manifest, async () => ({
        status: "available",
        version: "99.0.0",
    }));
    assert.equal(Object.keys(tools).length, 4);
    assert(Object.values(tools).every((tool) => !tool.matches));
});
