/**
 * Runs Oxlint, the linter of the project, for the rules of the configuration
 * that the lint does not run itself: Oxlint's own rules, and the rules of the
 * JS plugins that are neither ported (`meta.native`) nor hosted
 * (`meta.sfc`). The lint gives one report of both (./cli.ts).
 *
 * Oxlint comes from the project, as the parsers do: this package depends on
 * nothing.
 */
import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import {
    dirname,
    join,
    relative,
    resolve,
} from "node:path";
import { isRecord } from "./host/nodes.ts";
import { ConfigurationError } from "./project.ts";

/** A problem that Oxlint reports, at a 1-based line and column. */
export interface OxlintProblem {
    column: number;
    /** Relative to the root of the project. */
    file: string;
    line: number;
    message: string;
    /** As Oxlint names it: `eslint(no-var)`, `js(naming-convention)`. */
    rule: string;
    severity: number;
}

export interface OxlintRun {
    /** The configuration file. */
    config: string;
    fix: boolean;
    /** What to lint, relative to `root`. */
    paths: readonly string[];
    root: string;
    /** Stops the run: Oxlint is killed, and the result is a rejection. */
    signal?: AbortSignal;
}

const WARNING = 1;
const ERROR = 2;

/** The `oxlint` command of the project at `root`. */
function oxlintBin(root: string): string {
    const require = createRequire(join(root, "package.json"));
    let manifest: string;
    try {
        manifest = require.resolve("oxlint/package.json");
    }
    catch (error) {
        throw new ConfigurationError(
            `Cannot find the package oxlint from ${root}. tsrs-lint runs the rules of Oxlint with it, and takes it from the project it lints: add oxlint to the project's dependencies, or pass --no-oxlint.`,
            { cause: error },
        );
    }
    const { bin } = JSON.parse(readFileSync(manifest, "utf8")) as { bin?: unknown; };
    const path = typeof bin === "string" ? bin : isRecord(bin) ? bin["oxlint"] : undefined;
    if (typeof path !== "string") {
        throw new ConfigurationError(`The oxlint package of ${root} has no oxlint command`);
    }

    return resolve(dirname(manifest), path);
}

const number = (value: unknown): number | undefined => typeof value === "number" ? value : undefined;

function toProblem(root: string, diagnostic: unknown): OxlintProblem {
    const record = isRecord(diagnostic) ? diagnostic : {};
    const [label]: unknown[] = Array.isArray(record["labels"]) ? record["labels"] : [];
    const span = isRecord(label) && isRecord(label["span"]) ? label["span"] : {};

    return {
        column: number(span["column"]) ?? 1,
        file: relative(root, resolve(root, String(record["filename"]))),
        line: number(span["line"]) ?? 1,
        message: String(record["message"]),
        rule: typeof record["code"] === "string" ? record["code"] : "oxlint",
        severity: record["severity"] === "error" ? ERROR : WARNING,
    };
}

/**
 * The report of `oxlint -f json`. Oxlint exits with 1 for a problem and for a configuration it cannot read: a
 * report says which.
 */
function parseReport(root: string, stdout: string, stderr: string, status: number | null): OxlintProblem[] {
    // Oxlint can print a line of text before the report.
    const start = stdout.indexOf("{");
    let report: unknown;
    try {
        report = start === -1 ? undefined : JSON.parse(stdout.slice(start));
    }
    catch {
        report = undefined;
    }
    if ((status !== 0 && status !== 1) || !isRecord(report) || !Array.isArray(report["diagnostics"])) {
        // A configuration that Oxlint cannot read, mostly: what Oxlint says is the reason.
        throw new ConfigurationError(
            `oxlint failed (exit ${status}): ${[stdout.trim(), stderr.trim()].filter(text => text !== "").join("\n")}`,
        );
    }

    return report["diagnostics"].map(diagnostic => toProblem(root, diagnostic));
}

/** Runs Oxlint and returns what it reports. Oxlint starts before the function returns. */
export function runOxlint({ config, fix, paths, root, signal }: OxlintRun): Promise<OxlintProblem[]> {
    const bin = oxlintBin(root);

    return new Promise((done, fail) => {
        const child = spawn(
            process.execPath,
            [
                bin,
                "--config",
                config,
                "--format",
                "json",
                // The lint decides which paths have files, and a path without one is no error there.
                "--no-error-on-unmatched-pattern",
                ...(fix ? ["--fix"] : []),
                "--",
                ...paths,
            ],
            { cwd: root, signal, stdio: ["ignore", "pipe", "pipe"] },
        );
        const stdout: Buffer[] = [];
        const stderr: Buffer[] = [];
        child.stdout.on("data", (chunk: Buffer) => stdout.push(chunk));
        child.stderr.on("data", (chunk: Buffer) => stderr.push(chunk));
        child.once("error", fail);
        child.once("close", status => {
            try {
                done(parseReport(root, Buffer.concat(stdout).toString("utf8"), Buffer.concat(stderr).toString("utf8"), status));
            }
            catch (error) {
                fail(error);
            }
        });
    });
}
