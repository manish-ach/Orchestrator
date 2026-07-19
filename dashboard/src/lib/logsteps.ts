// Splits a job log into steps and classifies lines for the terminal view.
//
// The executor runs every script through `sh -x`, which emits a `+ <cmd>`
// trace line before each executed statement. Those markers become step
// headers (GitHub-Actions-style); everything between two markers is that
// step's output. Logs from before the tracing change simply produce one
// unnamed step, and the viewer falls back to a flat listing.

export interface LogStep {
  /** the traced command, or null for output before the first marker */
  cmd: string | null;
  /** original text lines belonging to this step */
  lines: string[];
  /** 1-based line number of the step's first output line in the raw log */
  start: number;
}

const MARKER = /^\++ (.*)$/;

export function parseSteps(lines: string[]): LogStep[] {
  const steps: LogStep[] = [];
  let cur: LogStep = { cmd: null, lines: [], start: 1 };
  lines.forEach((t, i) => {
    const m = MARKER.exec(t);
    if (m) {
      if (cur.cmd !== null || cur.lines.length) steps.push(cur);
      cur = { cmd: m[1], lines: [], start: i + 2 };
    } else {
      cur.lines.push(t);
    }
  });
  if (cur.cmd !== null || cur.lines.length) steps.push(cur);
  return steps;
}

export type LineKind = '' | 'err' | 'warn' | 'ok' | 'meta';

/** Keyword-based tint for a log line — docker/cargo/pytest friendly. */
export function classify(t: string): LineKind {
  if (/^\[executor\]/.test(t)) return 'meta';
  if (/\b(error|fatal|panic(ked)?|denied|traceback|exception|unreachable)\b/i.test(t)) return 'err';
  if (/\bfail(ed|ure)?\b/i.test(t) && !/\b0 fail(ed|ures)?\b/i.test(t)) return 'err';
  if (/\bwarn(ing)?s?\b/i.test(t)) return 'warn';
  if (/\b(passed|success(ful|fully)?|✓|finished|completed?|done|ok)\b/i.test(t) || /^\s*✓/.test(t))
    return 'ok';
  return '';
}
