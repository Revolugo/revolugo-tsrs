/**
 * Runs `tsrs-lint headless`, the native linter, on a payload of our own: the
 * rules of typescript-eslint that need the types, and the rules of JS plugins
 * that it has a port of. It lints a .vue file through the content mapper of
 * its tsconfig: a diagnostic comes back at its place in the .vue file.
 *
 * The protocol is tsgolint's: a JSON payload on stdin, and on stdout frames
 * of a 4-byte little-endian length, a 1-byte type and JSON.
 */
import getExePath from "#getExePath";
import {
    spawn,
    spawnSync,
} from "node:child_process";
import {
    closeSync,
    mkdtempSync,
    openSync,
    readFileSync,
    rmSync,
    writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { isRecord } from "./host/nodes.ts";
import { ConfigurationError } from "./project.ts";

export interface NativeRule {
    /** As `tsrs-lint rules` lists it: `no-floating-promises`, `js/camelcase`. */
    name: string;
    /** The options object of a rule of typescript-eslint, the options array of a ported rule. */
    options?: unknown;
}

/** Files linted with the same rules. */
export interface NativeGroup {
    files: string[];
    rules: NativeRule[];
}

/** A replacement of the bytes from `pos` to `end` of a file. */
export interface NativeFix {
    end: number;
    pos: number;
    text: string;
}

export interface NativeDiagnostic {
    /** UTF-8 byte offsets in the file; none for a diagnostic about the whole file. */
    end?: number;
    file: string;
    /** What fixes the problem, when the rule knows and the run asked for fixes. */
    fixes: NativeFix[];
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
 * The names of the rules that `tsrs-lint` implements (`tsrs-lint rules`). A rule of typescript-eslint has its name
 * without `typescript/`, and a ported rule has the name of its plugin: `js/camelcase`.
 */
export function implementedRules(root: string): Set<string> {
    const run = spawnSync(binary(), ["--lint", "rules"], { cwd: root, encoding: "utf8" });
    if (run.error !== undefined) {
        throw run.error;
    }
    if (run.status !== 0) {
        // A `tsrs` of another build, through TSRS_BINARY: upstream's has no `rules` command.
        throw new ConfigurationError(
            `\`tsrs-lint rules\` failed (exit ${run.status}): ${run.stderr.trim()}\nThe lint needs the tsrs-lint of its own package, which lists the rules it implements.`,
        );
    }

    return new Set(run.stdout.split("\n").filter(name => name !== ""));
}

const text = (value: unknown): string => typeof value === "string" ? value : "";

function toFix(fix: unknown): NativeFix[] {
    const range = isRecord(fix) && isRecord(fix["range"]) ? fix["range"] : {};
    const { end, pos } = range;

    return isRecord(fix) && typeof pos === "number" && typeof end === "number"
        ? [{ end, pos, text: text(fix["text"]) }]
        : [];
}

function toDiagnostic(frame: unknown, root: string): NativeDiagnostic {
    if (!isRecord(frame)) {
        throw new TypeError("tsrs-lint sent a diagnostic that is not an object");
    }
    const message = isRecord(frame["message"]) ? frame["message"] : {};
    const range = isRecord(frame["range"]) ? frame["range"] : {};
    const { end, pos } = range;
    const help = text(message["help"]);

    return {
        file: text(frame["file_path"]) || root,
        fixes: Array.isArray(frame["fixes"]) ? frame["fixes"].flatMap(toFix) : [],
        message: [text(message["description"]), help]
            .filter(part => part !== "")
            .join(" "),
        rule: text(frame["rule"]) || text(message["id"]),
        ...(typeof pos === "number" && typeof end === "number" ? { end, pos } : {}),
    };
}

export interface NativeRun {
    /** Whether the rules say how to fix what they report. */
    fix: boolean;
    /**
     * The tsconfig whose projects (and those it references) a file is linted in, with their content mappers, before
     * the tsconfig nearest to the file.
     */
    project: string | undefined;
    /** The directory that the files of the groups are relative to. */
    root: string;
    /** Stops the run: `tsrs-lint` is killed, and the result is a rejection. */
    signal?: AbortSignal;
}

function parseFrames(stdout: Buffer, root: string): { diagnostics: NativeDiagnostic[]; errors: string[]; } {
    const diagnostics: NativeDiagnostic[] = [];
    const errors: string[] = [];
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

    return { diagnostics, errors };
}

/**
 * Lints `groups` and returns what `tsrs-lint` reports, in its order.
 *
 * `tsrs-lint` starts before the function returns, and reads and writes files, not pipes: it runs to its end while
 * the caller keeps the thread, to run the rule host for instance.
 */
export function runTsrsLint({ fix, project, root, signal }: NativeRun, groups: readonly NativeGroup[]): Promise<NativeDiagnostic[]> {
    const directory = mkdtempSync(join(tmpdir(), "tsrs-lint-"));
    const file = (name: string): string => join(directory, name);
    writeFileSync(
        file("payload.json"),
        JSON.stringify({
            configs: groups.map(group => ({
                file_paths: group.files.map(path => join(root, path)),
                rules: group.rules,
            })),
            version: 2,
        }),
    );
    const stdio = [openSync(file("payload.json"), "r"), openSync(file("stdout"), "w"), openSync(file("stderr"), "w")];
    const exited = new Promise<number | null>((resolve, reject) => {
        const child = spawn(
            binary(),
            [
                "--lint",
                "headless",
                "--runExternalCode",
                ...(fix ? ["--fix"] : []),
                ...(project === undefined ? [] : ["--project", project]),
            ],
            { cwd: root, signal, stdio },
        );
        child.once("error", reject);
        child.once("exit", resolve);
    });

    return exited
        .then(status => {
            const { diagnostics, errors } = parseFrames(readFileSync(file("stdout")), root);
            if (status !== 0 || errors.length > 0) {
                throw new Error(
                    `tsrs-lint failed (exit ${status}): ${
                        [...errors, readFileSync(file("stderr"), "utf8").trim()].filter(line => line !== "").join("\n")
                    }`,
                );
            }

            return diagnostics;
        })
        .finally(() => {
            for (const descriptor of stdio) {
                closeSync(descriptor);
            }
            rmSync(directory, { force: true, recursive: true });
        });
}
