//! Measures the real indexing and search APIs, not application startup or rendering.
use workspace::indexing::index_workspace;
use project::{IndexStore, ScanOptions, SearchRequest};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
fn percentile(samples: &[Duration], p: f64) -> Duration {
    let mut samples = samples.to_vec();
    samples.sort_unstable();
    samples
        .get(((samples.len().saturating_sub(1)) as f64 * p).round() as usize)
        .copied()
        .unwrap_or_default()
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "perf_runner --corpus <root> [--queries N] [--runs N] [--smoke]\nMeasures indexing and query API latency only. GUI startup, input-to-pixel and workbench memory are unmeasured."
        );
        return Ok(());
    }
    let value = |key: &str| {
        args.iter()
            .position(|a| a == key)
            .and_then(|i| args.get(i + 1))
    };
    let root = PathBuf::from(value("--corpus").ok_or("--corpus <root> required")?);
    let count = value("--queries")
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(100)
        .clamp(1, 100);
    let runs = value("--runs")
        .map(|s| s.parse::<usize>())
        .transpose()?
        .unwrap_or(3)
        .clamp(1, 30);
    let temporary = std::env::temp_dir().join(format!("omw-perf-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&temporary)?;
    let result = (|| -> Result<String, Box<dyn std::error::Error>> {
        let store = IndexStore::open(temporary.join("index.sqlite"))?;
        let start = Instant::now();
        index_workspace(&store, &root, ScanOptions::default())?;
        let first = start.elapsed();
        let stats = store.stats()?;
        let queries = [
            "中",
            "中文",
            "项目方案",
            "rust",
            "性能",
            "task",
            "任务",
            "search",
            "搜索",
            "markdown",
            "ＡＢＣ",
            "🙂",
            "\"",
            "%",
            "索引",
            "保存",
            "priority",
            "due",
            "workspace",
            "计划",
        ];
        let mut samples = vec![];
        for _ in 0..runs {
            for i in 0..count {
                let query = queries[i % queries.len()];
                let start = Instant::now();
                let page = store.search(SearchRequest::new(query, false, 0, 50))?;
                if !page.index_ready {
                    return Err("index unexpectedly not ready".into());
                }
                samples.push(start.elapsed());
            }
        }
        let report = format!(
            "# Performance evidence\n\nBuild: {}\nCorpus: {}\nIndexed files: {}; tasks: {}\nFirst index: {:?}\nSearch API only (50-result page, excluding UI scheduling/debounce/rendering): P50 {:?}; P95 {:?}\nRaw query milliseconds: {:?}\n\nUNMEASURED: process startup-to-interactive, editor layout, input-to-pixel, ten-tab workbench PrivateBytes, GPU/worker/Pi resources. No GUI budget pass claimed. Previous stats()/format! proxy numbers are invalid for those metrics.\n",
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            root.display(),
            stats.files,
            stats.tasks,
            first,
            percentile(&samples, 0.5),
            percentile(&samples, 0.95),
            samples
                .iter()
                .map(|s| s.as_secs_f64() * 1000.)
                .collect::<Vec<_>>()
        );
        Ok(report)
    })();
    let _ = std::fs::remove_dir_all(&temporary);
    let report = result?;
    println!("{report}");
    std::fs::write("tools/perf-results-latest.txt", report)?;
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("Performance run failed: {error}");
        std::process::exit(1);
    }
}
