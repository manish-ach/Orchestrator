// Tiny line-based YAML tokenizer for the pipeline-file viewer. Not a real
// parser — just enough classes (key/string/number/bool/comment/dash) to
// color a CI file convincingly, including `script: |` block scalars, whose
// body lines are marked y-script so shell commands read as one unit.

export interface YamlTok {
  text: string;
  cls: '' | 'y-key' | 'y-val' | 'y-str' | 'y-num' | 'y-bool' | 'y-com' | 'y-dash' | 'y-script';
}

export function highlightYaml(src: string): YamlTok[][] {
  const out: YamlTok[][] = [];
  // indentation the current block scalar's body must exceed, or null
  let blockKeyIndent: number | null = null;

  for (const line of src.split('\n')) {
    const indent = /^ */.exec(line)![0].length;

    if (blockKeyIndent !== null) {
      if (line.trim() === '' || indent > blockKeyIndent) {
        out.push([{ text: line, cls: 'y-script' }]);
        continue;
      }
      blockKeyIndent = null;
    }

    if (/^\s*#/.test(line)) {
      out.push([{ text: line, cls: 'y-com' }]);
      continue;
    }

    const toks: YamlTok[] = [];
    const lead = /^(\s*)(- )?/.exec(line)!;
    if (lead[1]) toks.push({ text: lead[1], cls: '' });
    if (lead[2]) toks.push({ text: lead[2], cls: 'y-dash' });
    let rest = line.slice(lead[0].length);

    const key = /^([^\s:#][^:#]*):(?=\s|$)/.exec(rest);
    if (key) {
      toks.push({ text: key[0], cls: 'y-key' });
      rest = rest.slice(key[0].length);
    }

    let comment = '';
    const ci = rest.indexOf(' #');
    if (ci >= 0) {
      comment = rest.slice(ci);
      rest = rest.slice(0, ci);
    }

    if (rest.trim()) {
      const v = rest.trim();
      let cls: YamlTok['cls'] = 'y-val';
      if (/^(["']).*\1$/.test(v) || v.startsWith('[') || v.startsWith('{')) cls = 'y-str';
      else if (/^-?\d+(\.\d+)?$/.test(v)) cls = 'y-num';
      else if (/^(true|false|null|yes|no|~)$/i.test(v)) cls = 'y-bool';
      else if (/^[|>][+-]?\d*$/.test(v)) {
        cls = 'y-dash';
        blockKeyIndent = indent;
      }
      toks.push({ text: rest, cls });
    } else if (rest) {
      toks.push({ text: rest, cls: '' });
    }
    if (comment) toks.push({ text: comment, cls: 'y-com' });
    out.push(toks);
  }
  return out;
}
