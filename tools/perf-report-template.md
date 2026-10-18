# Performance evidence

## Environment

- Date, Rust version, build profile, Windows build:
- CPU, RAM, GPU/driver, disk:
- Display/DPI, power plan, antivirus/background activity:
- Corpus generator seed, manifest counts/checksum, Archive inclusion:

## Backend API measurements

Run `pwsh -File tools/bench.ps1 -CorpusDir <path> -Smoke` for a small corpus or `-Full` for the full generated corpus. Existing corpora are reused. The driver builds the locked release runner and uses Cargo's reported executable path; failures stop the run.

Record the runner's complete output from `tools/perf-results-latest.txt`:

- Indexed file/task counts; investigate disagreement with the manifest, including Archive/ignore exclusions.
- First indexing duration, measured separately.
- Search API P50/P95 and every raw sample, 50-result page, repeated fixed query set.

Search API duration excludes UI debounce, scheduling, rendering and display. No synthetic debounce wait is added. These samples cannot establish the product search first-batch budget.

## GUI acceptance, separate measurements required

| Requirement | Required measurement | Result |
| --- | --- | --- |
| Startup P95 ≤ 2 s | Process launch to usable workbench, cold/hot separately | Unmeasured |
| Search first batch P95 ≤ 100 ms | User query through debounce to visible results | Unmeasured |
| Input-to-display P95 ≤ 50 ms | Real editor input through rendered/displayed frame | Unmeasured |
| Main idle PrivateBytes ≤ 500 MiB | Real workbench with specified ten-tab corpus/state | Unmeasured |

Report worker/Pi/GPU resources separately. `IndexStore::stats()`, string formatting, and the backend runner's own memory are not proxies for these GUI measurements. Do not compare them to GUI budgets or mark acceptance complete.

## Evidence and limitations

- Exact command, corpus path, build profile and measurement boundaries:
- Raw samples, outliers and correctness failures:
- Unmeasured scenarios and missing instrumentation:
- Profiling evidence before any optimization:

The runner supplies backend evidence only; this report does not assert release acceptance.
