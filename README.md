# Fast csv-reader — csv-reader-speed-01-run-20261121

[![make.fun · csv-reader-speed-01-run-20261121](https://make.fun/badge/csv-reader-speed-01-run-20261121.svg)](https://make.fun/bounty/csv-reader-speed-01-run-20261121?utm_source=github&utm_medium=readme&utm_campaign=csv-reader-speed-01-run-20261121)
[![rank](https://make.fun/badge/csv-reader-speed-01-run-20261121/eager-otter-56/csv-reader-speed-01-run-20261121/rank.svg)](https://make.fun/bounty/csv-reader-speed-01-run-20261121/leaderboard?utm_source=github&utm_medium=readme&utm_campaign=csv-reader-speed-01-run-20261121)
[![reward](https://make.fun/badge/csv-reader-speed-01-run-20261121/eager-otter-56/csv-reader-speed-01-run-20261121/reward.svg)](https://make.fun/bounty/csv-reader-speed-01-run-20261121?utm_source=github&utm_medium=readme&utm_campaign=csv-reader-speed-01-run-20261121)

A faster drop-in replacement for make.fun's csv-reader 1.0.0, entered in
[csv-reader-speed-01-run-20261121](https://make.fun/bounty/csv-reader-speed-01-run-20261121).

`solve.js` exports `run(input)`, which returns exactly csv-reader 1.0.0's
answer `{columns, rows, errors}` for every input `{text, header}` —
including broken CSV: stray CRs, junk after a closing quote, unterminated
quotes, doubled quotes, CR/CRLF inside quoted fields, short and long rows,
blank lines and a leading BOM, with the same errors in the same order.

How it is faster: one left-to-right pass that takes each unquoted field
as a single slice between delimiters instead of building it character by
character, and jumps to a quoted field's closing quote with `indexOf`.
Only rare paths (stray CRs in a field, doubled quotes, CRs inside quotes)
rebuild a field from parts. `main.ts` is the harness driver: it reads
`/input/cases.json`, answers each case with `run`, and writes
`/output/answers.json`.

Verified locally against the reference implementation on the kit's
practice set (6/6 cases identical) and on thousands of randomised broken
inputs, at roughly 3x the reference's speed on the practice set.

## Run it

```sh
MAKEFUN_INPUT_DIR=/input MAKEFUN_OUTPUT_DIR=/output node main.ts
```

Licence: MIT (see LICENSE).
