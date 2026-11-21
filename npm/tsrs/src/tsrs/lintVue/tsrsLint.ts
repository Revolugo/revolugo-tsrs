/**
 * Runs `tsrs-lint`, the type-aware linter that Oxlint drives for the .ts
 * files, on a payload of our own. Oxlint leaves the .vue files out of its
 * payload, and this `tsrs-lint` lints them through the content mapper of
 * their tsconfig: a diagnostic comes back at its place in the .vue file.
 *
 * The protocol is tsgolint's: a JSON payload on stdin, and on stdout frames
 * of a 4-byte little-endian length, a 1-byte type and JSON.
 */
import getExePath from "#getExePath";
import { spawnSync } from "node:child_process";
import { join } from "node:path";
import { isRecord } from "./host/nodes.ts";
import { ConfigurationError } from "./project.ts";

export interface TypeAwareRule {
    name: string;
    options?: unknown;
}

/** Files linted with the same rules. */
export interface TypeAwareGroup {
    files: string[];
    rules: TypeAwareRule[];
}

export interface TypeAwareDiagnostic {
    /** UTF-8 byte offsets in the file; none for a diagnostic about the whole file. */
    end?: number;
    file: string;
    message: string;
    pos?: number;
    /** The rule, or the code of a compiler or tsconfig error. */
    rule: string;
}

const FRAME_ERROR = 0;
const FRAME_DIAGNOSTIC = 1;
const HEADER_LENGTH = 5;

/**
 * The `tsrs` binary, as the launchers of the package find it (`TSRS_BINARY`, or else the platform package).
 * `tsrs --lint` is `tsrs-lint`.
 */
function binary(): string {
    try {
        return getExePath();
    }
    catch (error) {
        throw new ConfigurationError(error instanceof Error ? error.message : String(error), { cause: error });
    }
}

/**
 * The names of the rules that `tsrs-lint` implements (`tsrs-lint rules`): those of a configuration that the Vue lint
 * runs with it, without the `typescript/` of their name.
 */
export function implementedRules(root: string): Set<string> {
    const run = spawnSync(binary(), ["--lint", "rules"], { cwd: root, encoding: "utf8" });
    if (run.error !== undefined) {
        throw run.error;
    }
    if (run.status !== 0) {
        // A `tsrs` of another build, through TSRS_BINARY: upstream's has no `rules` command.
        throw new ConfigurationError(
            `\`tsrs-lint rules\` failed (exit ${run.status}): ${run.stderr.trim()}\ntsrs-lint-vue needs the tsrs-lint of its own package, which lists the rules it implements.`,
        );
    }

    return new Set(run.stdout.split("\n").filter(name => name !== ""));
}

const text = (value: unknown): string => typeof value === "string" ? value : "";

function toDiagnostic(frame: unknown, root: string): TypeAwareDiagnostic {
    if (!isRecord(frame)) {
        throw new TypeError("tsrs-lint sent a diagnostic that is not an object");
    }
    const message = isRecord(frame["message"]) ? frame["message"] : {};
    const range = isRecord(frame["range"]) ? frame["range"] : {};
    const { end, pos } = range;
    const help = text(message["help"]);

    return {
        file: text(frame["file_path"]) || root,
        message: [text(message["description"]), help]
            .filter(part => part !== "")
            .join(" "),
        rule: text(frame["rule"]) || text(message["id"]),
        ...(typeof pos === "number" && typeof end === "number" ? { end, pos } : {}),
    };
}

/**
 * Lints `groups`, whose files are relative to `root`. With `project`, a file
 * is linted in the projects of this tsconfig (and of those it references),
 * with their content mappers, before the tsconfig nearest to it.
 */
export function runTsrsLint(
    root: string,
    project: string | undefined,
    groups: readonly TypeAwareGroup[],
): TypeAwareDiagnostic[] {
    const run = spawnSync(
        binary(),
        ["--lint", "headless", "--runExternalCode", ...(project === undefined ? [] : ["--project", project])],
        {
            cwd: root,
            input: JSON.stringify({
                configs: groups.map(group => ({
                    file_paths: group.files.map(file => join(root, file)),
                    rules: group.rules,
                })),
                version: 2,
            }),
            maxBuffer: Number.MAX_SAFE_INTEGER,
        },
    );
    if (run.error !== undefined) {
        throw run.error;
    }
    const diagnostics: TypeAwareDiagnostic[] = [];
    const errors: string[] = [];
    const { stdout } = run;
    let offset = 0;
    while (offset + HEADER_LENGTH <= stdout.length) {
        const length = stdout.readUInt32LE(offset);
        const type = stdout.readUInt8(offset + HEADER_LENGTH - 1);
        const start = offset + HEADER_LENGTH;
        const frame: unknown = JSON.parse(stdout.toString("utf8", start, start + length));
        if (type === FRAME_DIAGNOSTIC) {
            diagnostics.push(toDiagnostic(frame, root));
        }
        else if (type === FRAME_ERROR) {
            errors.push(isRecord(frame) ? text(frame["error"]) : "");
        }
        offset = start + length;
    }
    if (run.status !== 0 || errors.length > 0) {
        throw new Error(
            `tsrs-lint failed (exit ${run.status}): ${
                [...errors, run.stderr.toString("utf8").trim()].filter(line => line !== "").join("\n")
            }`,
        );
    }

    return diagnostics;
}
