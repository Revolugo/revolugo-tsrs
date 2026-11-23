/**
 * What the lint takes from the project it lints. This package depends on nothing: the parsers of a `.vue` file
 * are those of the project, so that a rule reads the tree that the project's other tools read.
 */
import { createRequire } from "node:module";
import { join } from "node:path";

/** A mistake in the command line, in the configuration or in what the project provides: the lint did not run. */
export class ConfigurationError extends Error {}

/**
 * Loads the package `name` as `root` resolves it, with `require`. `purpose` says what the lint does with the package,
 * for the error of a project that does not have it.
 */
export function requireFromProject(root: string, name: string, purpose: string): unknown {
    const require = createRequire(join(root, "package.json"));
    let resolved: string;
    try {
        resolved = require.resolve(name);
    }
    catch (error) {
        throw new ConfigurationError(
            `Cannot find the package ${name} from ${root}. tsrs-lint ${purpose} with it, and takes it from the project it lints: add ${name} to the project's dependencies.`,
            { cause: error },
        );
    }

    return require(resolved);
}
