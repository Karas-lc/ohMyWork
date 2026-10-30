#!/usr/bin/env python3
"""
Reproducible performance corpus generator for ohMyWork V1 plan 26.

- Fixed seed for determinism.
- Targets: ~10k notes, ~200 MB total, ~20k tasks.
- Mixed zh/en content.
- Structure: Atlas/, Efforts/, Archive/ (with light nesting to simulate real workspaces).
- Includes CommonMark + GFM tasks for index/search correctness verification.
- --smoke for small CI-sized subset (still exercises same generator paths).
- Writes checksum manifest for verification.
- python tools/generate-corpus.py --help works.
"""

import argparse
import hashlib
import json
import os
import random
import shutil
import sys
from pathlib import Path
from typing import List, Tuple

# Fixed seed ensures reproducibility across runs/machines for same seed.
DEFAULT_SEED = 42
DEFAULT_OUTPUT = "perf-corpus"
FULL_NOTES = 10000
SMOKE_NOTES = 200  # CI sized, still produces valid mixed structure + tasks
FULL_TARGET_MB = 200
AVG_TASKS_PER_NOTE = 2.0

# Topics for mixed content (reproducible via seed)
EN_TOPICS = [
    "project planning", "rust performance tuning", "markdown indexing",
    "task management", "search relevance", "workspace organization",
    "chinese english bilingual notes", "atlas knowledge base", "effort tracking",
    "archive cleanup", "gpui rendering", "sqlite fts queries", "cold start latency",
    "input debounce timing", "private bytes measurement", "index ingest throughput",
]
ZH_TOPICS = [
    "项目规划", "Rust性能调优", "Markdown索引", "任务管理", "搜索相关性",
    "工作区组织", "中英双语笔记", "知识库Atlas", "Efforts追踪",
    "Archive清理", "GPUI渲染", "SQLite FTS查询", "冷启动延迟",
    "输入防抖计时", "私有内存测量", "索引摄取吞吐",
]

EN_PARAGRAPH_TEMPLATES = [
    "This note discusses {topic} in detail. We explore tradeoffs and best practices for large scale personal knowledge bases on Windows.",
    "Performance considerations for {topic} include bounded caches, background generation with cancellation, and avoiding UI thread work.",
    "In practice, {topic} benefits from fixed query sets for reproducible benchmarking and checksummed corpora.",
    "Mixed language content helps verify tokenizer and n-gram behavior in the index for both ASCII and CJK.",
]
ZH_PARAGRAPH_TEMPLATES = [
    "本笔记详细讨论了{topic}。我们探讨了大型个人知识库在Windows上的权衡与最佳实践。",
    "{topic}的性能考虑包括有界缓存、带取消的后台生成以及避免UI线程阻塞。",
    "实际中，{topic}受益于固定查询集以实现可复现的基准测试和带校验和的语料库。",
    "混合语言内容有助于验证索引中针对ASCII和CJK的分词与n-gram行为。",
]

TASK_TEMPLATES_EN = [
    "- [ ] Implement {topic} measurement",
    "- [x] Baseline {topic} on reference hardware",
    "- [ ] Profile hot path for {topic} with evidence only",
    "- [ ] Add 20ms debounce accounting to first batch search",
]
TASK_TEMPLATES_ZH = [
    "- [ ] 实现{topic}测量",
    "- [x] 在参考硬件上对{topic}做基线",
    "- [ ] 仅在有profile证据时优化{topic}热点",
    "- [ ] 将20ms debounce计入首批搜索时间",
]

def generate_paragraphs(topic: str, lang_mix: float, target_chars: int, rng: random.Random) -> str:
    """Generate mixed zh/en paragraphs to hit target size per note."""
    paras: List[str] = []
    current = 0
    zh_t = ZH_PARAGRAPH_TEMPLATES
    en_t = EN_PARAGRAPH_TEMPLATES
    while current < target_chars:
        if rng.random() < lang_mix:
            tmpl = rng.choice(zh_t)
            p = tmpl.format(topic=rng.choice(ZH_TOPICS))
        else:
            tmpl = rng.choice(en_t)
            p = tmpl.format(topic=rng.choice(EN_TOPICS))
        extra = " " + " ".join(rng.choice(["detail", "analysis", "note", "观察", "数据", "baseline"]) for _ in range(rng.randint(8, 40)))
        p = p + extra
        paras.append(p)
        current += len(p)
    return "\n\n".join(paras)

def generate_tasks(topic: str, count: int, rng: random.Random) -> str:
    tasks: List[str] = []
    for i in range(count):
        if rng.random() < 0.5:
            tmpl = rng.choice(TASK_TEMPLATES_ZH)
            t = tmpl.format(topic=rng.choice(ZH_TOPICS))
        else:
            tmpl = rng.choice(TASK_TEMPLATES_EN)
            t = tmpl.format(topic=rng.choice(EN_TOPICS))
        if rng.random() < 0.3:
            t += f" [due:: 2026-{(i%12)+1:02d}-15] [priority:: {rng.randint(1,9)}]"
        if rng.random() < 0.2:
            t = t.replace("[ ]", "[x]")
        tasks.append(t)
    return "\n".join(tasks)

def make_note_path(base: Path, area: str, idx: int, base_seed: int, rng: random.Random | None = None) -> Path:
    """Create realistic nested path under area for scan realism. Uses sub-Random so outer seed varies paths/content."""
    if rng is None:
        rng = random.Random(base_seed + idx)
    sub_rng = random.Random(base_seed + idx)
    depth = sub_rng.choice([0, 0, 1, 1, 2])
    parts = [area]
    if depth >= 1:
        sub = sub_rng.choice(["Notes", "Topics", "Projects", "Research", "Archive-Sub"])
        parts.append(sub)
    if depth >= 2:
        sub2 = sub_rng.choice(["2025", "2026", "Personal", "Work", "Misc"])
        parts.append(sub2)
    fname = f"note-{idx:05d}-{sub_rng.choice(['zh','en','mix'])}.md"
    return base.joinpath(*parts) / fname

def generate_note(area: str, idx: int, target_size: int, rng: random.Random) -> Tuple[str, int]:
    """Return (content, task_count) for one note."""
    topic = rng.choice(EN_TOPICS + ZH_TOPICS)
    lang_mix = 0.4 if "zh" in topic or rng.random() < 0.45 else 0.15
    body = generate_paragraphs(topic, lang_mix, target_size, rng)
    num_tasks = max(1, int(rng.gauss(AVG_TASKS_PER_NOTE, 0.8)))
    num_tasks = min(6, max(0, num_tasks))
    task_block = generate_tasks(topic, num_tasks, rng)
    header = f"# {topic.title()} Note {idx}\n\n"
    content = header + body + "\n\n## Tasks\n\n" + task_block + "\n"
    return content, num_tasks

def count_tasks_in_text(text: str) -> int:
    return text.count("- [ ]") + text.count("- [x]")

def compute_tree_checksum(root: Path) -> str:
    """Reproducible checksum over relative paths + content (sorted). Skip MANIFEST as it is written after checksum snapshot."""
    h = hashlib.sha256()
    files: List[Tuple[str, bytes]] = []
    for p in sorted(root.rglob("*")):
        if p.is_file() and p.name != "MANIFEST.json":
            rel = p.relative_to(root).as_posix()
            data = p.read_bytes()
            files.append((rel, data))
    for rel, data in files:
        h.update(rel.encode("utf-8"))
        h.update(b"\0")
        h.update(data)
        h.update(b"\0")
    return h.hexdigest()

def generate_corpus(output: Path, note_count: int, target_total_mb: int, seed: int) -> dict:
    if output.exists():
        shutil.rmtree(output)
    output.mkdir(parents=True)

    areas = ["Atlas", "Efforts", "Archive"]
    counts = [
        int(note_count * 0.65),
        int(note_count * 0.25),
        note_count - int(note_count * 0.65) - int(note_count * 0.25),
    ]
    counts[2] = max(0, counts[2])

    total_bytes = 0
    total_tasks = 0
    total_notes = 0
    note_idx = 0

    rng = random.Random(seed)
    per_note_target = max(800, int((target_total_mb * 1024 * 1024) / max(1, note_count)))

    for area, area_count in zip(areas, counts):
        for i in range(area_count):
            path = make_note_path(output, area, note_idx, seed, rng)
            path.parent.mkdir(parents=True, exist_ok=True)
            jitter = rng.randint(-2000, 8000)
            content, ntasks = generate_note(area, note_idx, per_note_target + jitter, rng)
            path.write_text(content, encoding="utf-8")
            total_bytes += len(content.encode("utf-8"))
            total_tasks += ntasks + count_tasks_in_text(content)
            total_notes += 1
            note_idx += 1

    actual_mb = total_bytes / (1024 * 1024)
    checksum = compute_tree_checksum(output)

    manifest = {
        "seed": seed,
        "note_count": total_notes,
        "task_count": total_tasks,
        "total_bytes": total_bytes,
        "total_mb": round(actual_mb, 2),
        "checksum": checksum,
        "areas": {a: c for a, c in zip(areas, counts)},
        "generator": "tools/generate-corpus.py",
        "version": "plan26-v1",
    }
    (output / "MANIFEST.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return manifest

def verify_corpus(output: Path) -> dict:
    if not output.exists():
        raise FileNotFoundError(f"Corpus not found: {output}")
    manifest_path = output / "MANIFEST.json"
    if not manifest_path.exists():
        raise FileNotFoundError("MANIFEST.json missing")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))

    # Recompute
    recomputed = compute_tree_checksum(output)
    if recomputed != manifest["checksum"]:
        raise ValueError(f"Checksum mismatch: expected {manifest['checksum'][:16]} got {recomputed[:16]}")

    # Count actual
    md_files = list(output.rglob("*.md"))
    actual_notes = len(md_files)
    actual_tasks = 0
    actual_bytes = 0
    for f in md_files:
        txt = f.read_text(encoding="utf-8", errors="ignore")
        actual_tasks += count_tasks_in_text(txt)
        actual_bytes += len(txt.encode("utf-8"))

    if actual_notes != manifest["note_count"]:
        print(f"WARNING: note count drift {actual_notes} vs manifest {manifest['note_count']}", file=sys.stderr)

    return {
        "verified": True,
        "notes": actual_notes,
        "tasks": actual_tasks,
        "bytes": actual_bytes,
        "checksum": manifest["checksum"],
    }

def main():
    parser = argparse.ArgumentParser(
        description="Generate fixed-seed perf corpus (~10k notes/200MB/20k tasks mixed zh/en). "
                    "Full generator always able to emit target size; --smoke for subset.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="Examples:\n"
               "  python tools/generate-corpus.py --output-dir perf-corpus --seed 42\n"
               "  python tools/generate-corpus.py --smoke --output-dir /tmp/smoke\n"
               "  python tools/generate-corpus.py --verify-only perf-corpus\n",
    )
    parser.add_argument("--output-dir", default=DEFAULT_OUTPUT, help="Directory to write corpus (will be created/overwritten)")
    parser.add_argument("--seed", type=int, default=DEFAULT_SEED, help="Random seed for full reproducibility")
    parser.add_argument("--smoke", action="store_true", help="Generate small CI-sized subset (~200 notes) exercising same code paths")
    parser.add_argument("--verify-only", metavar="DIR", help="Only verify existing corpus dir (no generation)")
    parser.add_argument("--target-mb", type=int, default=FULL_TARGET_MB, help="Approximate total size target (full mode)")
    parser.add_argument("--notes", type=int, default=None, help="Override note count (advanced)")

    args = parser.parse_args()


    if args.verify_only:
        try:
            res = verify_corpus(Path(args.verify_only))
            print(json.dumps(res, indent=2))
            print("Corpus verification PASSED.")
            sys.exit(0)
        except Exception as e:
            print(f"VERIFY FAILED: {e}", file=sys.stderr)
            sys.exit(2)

    note_count = args.notes or (SMOKE_NOTES if args.smoke else FULL_NOTES)
    target_mb = 5 if args.smoke else args.target_mb

    out = Path(args.output_dir)
    print(f"Generating corpus: notes={note_count} target~{target_mb}MB seed={args.seed} -> {out}", file=sys.stderr)
    manifest = generate_corpus(out, note_count, target_mb, args.seed)

    print("Generation complete.")
    print(json.dumps(manifest, indent=2))

    # Quick self-check (recomputes checksum from disk content generated under the provided seed)
    try:
        v = verify_corpus(out)
        print(f"Self-checksum OK: {v['checksum'][:16]}...")
    except Exception as e:
        print(f"Self-verify warning: {e}", file=sys.stderr)

    print("\nUse with: cargo run -p ohmywork --bin perf_runner -- --corpus " + str(out))
    print("Or pwsh tools/bench.ps1")

if __name__ == "__main__":
    main()
