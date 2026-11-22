// Reads /input/cases.json and writes /output/answers.json.
// Fast path: transduce the raw bytes in WebAssembly. Fallback: parse
// and answer case by case with solve.js's run().
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { run, transduceBuffer } from './solve.js';

const inputPath = '/input/cases.json';
const outputPath = '/output/answers.json';

const raw = readFileSync(inputPath);
let out: Buffer;
try {
  out = transduceBuffer(raw);
} catch {
  const cases = JSON.parse(raw.toString('utf8'));
  const answers = cases.map((c: any) => run(c.input));
  out = Buffer.from(JSON.stringify(answers), 'utf8');
}
mkdirSync('/output', { recursive: true });
writeFileSync(outputPath, out);
