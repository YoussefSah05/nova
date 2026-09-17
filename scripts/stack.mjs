import { readFile, readdir } from "node:fs/promises";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { fileURLToPath, pathToFileURL } from "node:url";
import path from "node:path";

const run = promisify(execFile);
const root = fileURLToPath(new URL("../", import.meta.url));
const exactVersion = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;

// These readers deliberately handle only the generated/simple fields we own.
// Unsupported syntax fails closed; this is not a general TOML/YAML parser.
function quotedField(text, key) {
    return text.match(new RegExp(`^${key}\\s*=\\s*"([^"\\n]+)"`, "m"))?.[1];
}

export function cargoVersions(lock, name) {
    return lock
        .split(/^\[\[package\]\]\s*$/m)
        .filter((block) => quotedField(block, "name") === name)
        .map((block) => quotedField(block, "version"));
}

function pnpmHasVersion(lock, name, version) {
    const packages = lock
        .split(/^packages:\s*$/m)[1]
        ?.split(/^snapshots:\s*$/m)[0];
    return (
        packages?.split("\n").some((line) => {
            const key = line.match(/^ {2}(.+):\s*$/)?.[1];
            return key?.replace(/^['"]|['"]$/g, "") === `${name}@${version}`;
        }) ?? false
    );
}

export async function readInventory(directory = root) {
    const files = [
        "nova-stack.json",
        "package.json",
        "apps/desktop/package.json",
        "apps/desktop/src-tauri/tauri.conf.json",
        "apps/desktop/src-tauri/Cargo.toml",
        "Cargo.lock",
        "pnpm-lock.yaml",
        "rust-toolchain.toml",
        "mise.toml",
    ];
    const texts = await Promise.all(
        files.map((file) => readFile(path.join(directory, file), "utf8")),
    );
    const [manifest, pkg, desktop, tauri] = texts
        .slice(0, 4)
        .map((s) => JSON.parse(s));
    const migrations = await readdir(
        path.join(directory, "crates/storage/migrations"),
    );
    const profile = await readdir(
        path.join(directory, "crates/storage/migrations/profile"),
    );
    return {
        manifest,
        pkg,
        desktop,
        tauri,
        cargoToml: texts[4],
        cargoLock: texts[5],
        pnpmLock: texts[6],
        rustToolchain: texts[7],
        mise: texts[8],
        migrations,
        profile,
    };
}

export function validateInventory(data) {
    const m = data.manifest;
    const issues = [];
    const check = (condition, message) => {
        if (!condition) issues.push(message);
    };
    check(m.schemaVersion === 1, "Unsupported stack manifest schemaVersion");
    check(
        typeof m.appVersion === "string" && exactVersion.test(m.appVersion),
        "Invalid appVersion",
    );
    for (const [label, actual] of Object.entries({
        root: data.pkg.version,
        frontend: data.desktop.version,
        tauri: data.tauri.version,
        rustDesktop: quotedField(data.cargoToml, "version"),
    })) {
        check(
            actual === m.appVersion,
            `${label} app version differs from manifest`,
        );
    }
    check(
        cargoVersions(data.cargoLock, "nova-desktop").join() === m.appVersion,
        "Cargo.lock desktop version differs from manifest",
    );
    for (const name of ["rust", "node", "pnpm", "just"]) {
        const version = m.toolchain?.[name];
        check(
            typeof version === "string" && exactVersion.test(version),
            `${name} needs an exact toolchain version`,
        );
        check(
            quotedField(data.mise, name) === version && version !== undefined,
            `mise ${name} differs from manifest`,
        );
    }
    check(
        quotedField(data.rustToolchain, "channel") === m.toolchain?.rust,
        "rust-toolchain differs from manifest",
    );
    check(
        data.pkg.packageManager === `pnpm@${m.toolchain?.pnpm}`,
        "packageManager differs from manifest",
    );
    for (const [field, required] of Object.entries({
        rustPackages: ["tauri", "sqlx", "libsqlite3-sys"],
        jsPackages: ["react", "@tauri-apps/api", "@tauri-apps/cli"],
    })) {
        for (const name of required)
            check(
                typeof m[field]?.[name] === "string",
                `${field} missing ${name}`,
            );
    }
    for (const [name, version] of Object.entries(m.rustPackages ?? {})) {
        check(
            typeof version === "string" && exactVersion.test(version),
            `Invalid Rust pin: ${name}`,
        );
        check(
            cargoVersions(data.cargoLock, name).includes(version),
            `Cargo.lock missing ${name}@${version}`,
        );
    }
    for (const [name, version] of Object.entries(m.jsPackages ?? {})) {
        check(
            typeof version === "string" && exactVersion.test(version),
            `Invalid JS pin: ${name}`,
        );
        check(
            pnpmHasVersion(data.pnpmLock, name, version),
            `pnpm-lock missing ${name}@${version}`,
        );
    }
    check(
        data.tauri.bundle?.macOS?.minimumSystemVersion ===
            m.macos?.minimumSystemVersion &&
            typeof m.macos?.minimumSystemVersion === "string",
        "macOS deployment target differs from manifest",
    );
    const latest = (names) =>
        names
            .filter((name) => /^\d+_.*\.sql$/.test(name))
            .sort()
            .at(-1);
    check(
        latest(data.migrations) === m.database?.registryMigration &&
            !!m.database?.registryMigration,
        "Registry migration head differs from manifest",
    );
    check(
        latest(data.profile) === m.database?.profileMigration &&
            !!m.database?.profileMigration,
        "Profile migration head differs from manifest",
    );
    check(
        Array.isArray(m.unverified) &&
            m.unverified.every((item) => typeof item === "string"),
        "unverified must be a list of remaining validation gaps",
    );
    return issues;
}

export async function probeVersion(
    command,
    args = ["--version"],
    execute = run,
) {
    try {
        const { stdout } = await execute(command, args, {
            timeout: 5000,
            maxBuffer: 16384,
            encoding: "utf8",
            windowsHide: true,
        });
        const version = stdout.match(
            /(?:^|\s)v?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)(?=\s|$)/,
        )?.[1];
        return version
            ? { status: "available", version }
            : { status: "unrecognized" };
    } catch (error) {
        // Never return arbitrary stderr, command environment or filesystem paths.
        return { status: error.code === "ENOENT" ? "missing" : "failed" };
    }
}

export async function inspectEnvironment(manifest, probe = probeVersion) {
    const entries = await Promise.all(
        Object.entries({
            rust: "rustc",
            node: "node",
            pnpm: "pnpm",
            just: "just",
        }).map(async ([name, command]) => {
            const result = await probe(command);
            return [
                name,
                {
                    ...result,
                    expected: manifest.toolchain?.[name],
                    matches:
                        result.status === "available" &&
                        result.version === manifest.toolchain?.[name],
                },
            ];
        }),
    );
    return Object.fromEntries(entries);
}

export async function main(args) {
    const [command = "stack", ...flags] = args;
    if (
        !["stack", "doctor", "verify-environment"].includes(command) ||
        flags.some((flag) => !["--json", "--manifest-only"].includes(flag)) ||
        (flags.includes("--manifest-only") && command !== "verify-environment")
    ) {
        console.error(
            "Usage: node scripts/stack.mjs stack|doctor|verify-environment [--json] [--manifest-only]",
        );
        return 2;
    }
    try {
        const data = await readInventory();
        const issues = validateInventory(data);
        const tools =
            command === "stack" || flags.includes("--manifest-only")
                ? undefined
                : await inspectEnvironment(data.manifest);
        const report = {
            manifest: data.manifest,
            issues,
            ...(tools ? { tools } : {}),
            scope: tools
                ? "Manifest and development toolchain; not native runtime/release certification"
                : "Repository manifest only; no runtime capability claim",
        };
        if (flags.includes("--json") || command === "stack")
            console.log(JSON.stringify(report, null, 2));
        else {
            console.log(
                `Nova ${data.manifest.appVersion}: ${issues.length === 0 ? "manifest consistent" : "manifest drift"}`,
            );
            for (const issue of issues) console.log(`FAIL: ${issue}`);
            for (const [name, result] of Object.entries(tools ?? {}))
                console.log(
                    `${result.matches ? "OK" : "FAIL"}: ${name} ${result.version ?? result.status}; expected ${result.expected}`,
                );
            console.log(report.scope);
            for (const gap of data.manifest.unverified ?? [])
                console.log(`UNVERIFIED: ${gap}`);
        }
        return issues.length ||
            Object.values(tools ?? {}).some((tool) => !tool.matches)
            ? 1
            : 0;
    } catch {
        console.error(
            "Cannot read stack inputs. Run from a complete checkout with valid manifest and lockfiles; no files were changed.",
        );
        return 2;
    }
}

if (
    process.argv[1] &&
    import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href
)
    process.exitCode = await main(process.argv.slice(2));
