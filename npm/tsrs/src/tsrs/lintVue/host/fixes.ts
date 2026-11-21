/**
 * The fixes of the hosted rules: the `fixer` a rule builds them with, and
 * how they are merged and applied, as ESLint does it.
 */
import type {
    Fix,
    Fixer,
    LintMessage,
    ReportDescriptor,
} from "./types.ts";

const BOM = "\uFEFF";

const replace = (range: Fix["range"], text: string): Fix => ({ range, text });

/** The `fixer` given to the `fix` function of a report. */
const FIXER: Fixer = {
    insertTextAfter: (target, text) => replace([target.range[1], target.range[1]], text),
    insertTextAfterRange: (range, text) => replace([range[1], range[1]], text),
    insertTextBefore: (target, text) => replace([target.range[0], target.range[0]], text),
    insertTextBeforeRange: (range, text) => replace([range[0], range[0]], text),
    remove: target => replace(target.range, ""),
    removeRange: range => replace(range, ""),
    replaceText: (target, text) => replace(target.range, text),
    replaceTextRange: replace,
};

const byRange = (a: Fix, b: Fix): number => a.range[0] - b.range[0] || a.range[1] - b.range[1];

function isIterable(value: Fix | Iterable<Fix>): value is Iterable<Fix> {
    return Symbol.iterator in value;
}

/** One fix that does what the fixes of a report do together: the text between them is kept. */
function merge(fixes: readonly Fix[], text: string): Fix | undefined {
    const sorted = [...fixes].sort(byRange);
    const [first] = sorted;
    const last = sorted.at(-1);
    if (first === undefined || last === undefined) {
        return undefined;
    }
    if (sorted.length === 1) {
        return replace([first.range[0], first.range[1]], first.text);
    }
    const [start] = first.range;
    const [, end] = last.range;
    let merged = "";
    let lastPosition = Number.MIN_SAFE_INTEGER;
    for (const fix of sorted) {
        if (fix.range[0] < lastPosition) {
            throw new Error("Fix objects must not be overlapped in a report.");
        }
        if (fix.range[0] >= 0) {
            merged += text.slice(Math.max(0, start, lastPosition), fix.range[0]);
        }
        merged += fix.text;
        [, lastPosition] = fix.range;
    }
    merged += text.slice(Math.max(0, start, lastPosition), end);

    return replace([start, end], merged);
}

/** The fix of a report on `text`, if it has one. */
export function fixOf(descriptor: ReportDescriptor, text: string): Fix | undefined {
    if (typeof descriptor.fix !== "function") {
        return undefined;
    }
    const fix = descriptor.fix(FIXER);
    if (fix === null || fix === undefined) {
        return undefined;
    }

    return merge(isIterable(fix) ? [...fix] : [fix], text);
}

export interface Fixed {
    fixed: boolean;
    /** The messages whose fix was not applied, or that have none. */
    messages: LintMessage[];
    output: string;
}

/**
 * Applies the fixes of `messages` to `source`, from the start of the text: a
 * fix that overlaps one already applied is left for the next pass.
 */
export function applyFixes(source: string, messages: readonly LintMessage[]): Fixed {
    const fixable = messages
        .flatMap(message => (message.fix === undefined ? [] : [{ fix: message.fix, message }]))
        .sort((a, b) => byRange(a.fix, b.fix));
    if (fixable.length === 0) {
        return { fixed: false, messages: [...messages], output: source };
    }
    const bom = source.startsWith(BOM) ? BOM : "";
    const text = source.slice(bom.length);
    const remaining = messages.filter(message => message.fix === undefined);
    let output = bom;
    let lastPosition = Number.NEGATIVE_INFINITY;
    for (const { fix, message } of fixable) {
        const [start, end] = fix.range;
        if (lastPosition >= start || start > end) {
            remaining.push(message);
        }
        else {
            if ((start < 0 && end >= 0) || (start === 0 && fix.text.startsWith(BOM))) {
                output = "";
            }
            output += text.slice(Math.max(0, lastPosition), Math.max(0, start));
            output += fix.text;
            lastPosition = end;
        }
    }
    output += text.slice(Math.max(0, lastPosition));

    return {
        fixed: true,
        messages: remaining.sort((a, b) => a.line - b.line || a.column - b.column),
        output,
    };
}
