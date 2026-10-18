//! 属性测试：固定种子 × 大量随机操作，验证不变量。
//!
//! 每个种子跑 N 步操作，每步后校验：
//! - 树本身合法（validate）
//! - 工作树语义 == 重解析语义（tree == reparse）
//! - 全部撤销后字节 == 初始字节；全部重做后字节 == 最终字节
//! - 随机 op 的合法/拒绝路径都不破坏不变量
//!
//! 生成器只构造“参数必定落在可编辑位”的操作；对拒绝型错误
//! （类型化 EditError）跳过重试，不视为失败——拒绝边界是显式行为。

mod common;

use common::{byte_of, check_invariants, current_bytes, session};
use omw_document::{ByteOffset, EditSession, MarkKind, VisualPos, VisualRange};

/// 最小确定性 PRNG（xorshift64*），不依赖 rand 依赖。
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const SEEDS: [u64; 5] = [0x5EED, 0xC0FFEE, 0x1234, 0xBEEF, 0xABCD];
const OPS_PER_SEED: usize = 200;

/// 求一个叶内的可编辑位（含叶末尾）。
fn leaf_editable(es: &EditSession, leaf_idx: usize) -> Vec<VisualPos> {
    let mut out = Vec::new();
    let text_s = String::from_utf8(es.bytes()).expect("utf-8");
    let leaves = es.text_leaves();
    let (_, r) = &leaves[leaf_idx];
    let (s, e) = (r.start.0 as usize, r.end.0 as usize);
    let mut i = s;
    loop {
        if let Some(v) = es.byte_to_visual(ByteOffset(i as u32)) {
            out.push(v);
        }
        if i >= e {
            break;
        }
        let ch_len = text_s[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        i += ch_len;
    }
    out
}

/// 在单个叶内随机取一个子范围（起点..终点，两处均为可编辑位）。
fn rand_leaf_range(es: &EditSession, rng: &mut Rng) -> Option<(ByteOffset, ByteOffset)> {
    let len = es.text_leaves().len();
    if len == 0 {
        return None;
    }
    let leaf = rng.below(len);
    let poss = leaf_editable(es, leaf);
    if poss.len() < 2 {
        return None;
    }
    let a = rng.below(poss.len());
    let b = rng.below(poss.len());
    let (s, e) = if a <= b { (a, b) } else { (b, a) };
    Some((byte_of(es, poss[s]), byte_of(es, poss[e])))
}

/// 候选输入片段（转义后必须是字面文本，故只用普通 ASCII 词与 CJK）。
const INSERTS: [&str; 8] = ["z", "ab", "word", "x", "hej", "中文", "测试", "ok"];

fn run_seed(seed: u64) {
    let fixture = "alpha beta\n\n- item one\n- item two\n\nplain **bold** tail\n\n代码 `x` end\n\n> quote line\n\n<div>\nraw\n</div>\n\nfinal para\n";
    let (_, mut es) = session(fixture);
    let original = current_bytes(&es);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed);

    // 允许 op 集合：插入 / 单叶删除 / 单叶替换 / 词级 toggle。
    // 拒绝类（如删除整段、跨 Raw）由类型化错误表示，此处验证它们不
    // 破坏不变量即可，不计数为成功 op。
    let mut accepted: usize = 0;
    let mut rejected: usize = 0;
    while accepted < OPS_PER_SEED && rejected < OPS_PER_SEED * 20 {
        let kind = rng.below(4);
        let op_res = match kind {
            0 => {
                // insert
                let len = es.text_leaves().len();
                if len == 0 {
                    None
                } else {
                    let poss = leaf_editable(&es, rng.below(len));
                    if poss.is_empty() {
                        None
                    } else {
                        let v = poss[rng.below(poss.len())];
                        let s = INSERTS[rng.below(INSERTS.len())];
                        Some(es.insert_text(v, s))
                    }
                }
            }
            1 => {
                // delete within one leaf
                rand_leaf_range(&es, &mut rng)
                    .map(|(b, e)| es.delete_text(VisualRange::new(v_of(&es, b), v_of(&es, e))))
            }
            2 => {
                // replace within one leaf
                rand_leaf_range(&es, &mut rng).map(|(b, e)| {
                    es.replace_selection(
                        VisualRange::new(v_of(&es, b), v_of(&es, e)),
                        INSERTS[rng.below(INSERTS.len())],
                    )
                })
            }
            _ => {
                // toggle mark on a small run in one leaf (选短词际边界)
                rand_leaf_range(&es, &mut rng).and_then(|(b, e)| {
                    if e.0 - b.0 > 24 {
                        return None;
                    }
                    let mark = if rng.below(2) == 0 {
                        MarkKind::Bold
                    } else {
                        MarkKind::Italic
                    };
                    Some(es.toggle_mark(mark, VisualRange::new(v_of(&es, b), v_of(&es, e))))
                })
            }
        };

        match op_res {
            None => continue,
            Some(Ok(_)) => {
                accepted += 1;
                check_invariants(&es, &format!("seed={seed:#x} op#{accepted}"));
            }
            Some(Err(e)) => {
                // 类型化拒绝：状态必须原封不动，不破坏不变量。
                let _ = e;
                rejected += 1;
                check_invariants(&es, &format!("seed={seed:#x} reject"));
            }
        }
    }

    assert!(
        accepted >= OPS_PER_SEED / 2,
        "seed {seed:#x}: too few accepted ops ({accepted})"
    );
    let final_bytes = current_bytes(&es);
    let final_tree = es.semantic_projection();
    check_invariants(&es, "final");

    // undo 全部 → 原字节
    let mut undo_n = 0;
    while es.history().undo_depth() > 0 {
        if let Err(e) = es.undo() {
            panic!("seed {seed:#x}: undo failed at step {undo_n}: {e}");
        }
        undo_n += 1;
        check_invariants(&es, &format!("seed={seed:#x} undo#{undo_n}"));
    }
    assert_eq!(
        current_bytes(&es),
        original,
        "seed {seed:#x}: undo-all != original"
    );

    // redo 全部 → 终态
    let mut redo_n = 0;
    while es.history().redo_depth() > 0 {
        if let Err(e) = es.redo() {
            panic!("seed {seed:#x}: redo failed at step {redo_n}: {e}");
        }
        redo_n += 1;
        check_invariants(&es, &format!("seed={seed:#x} redo#{redo_n}"));
    }
    assert_eq!(
        current_bytes(&es),
        final_bytes,
        "seed {seed:#x}: redo-all != final"
    );
    assert_eq!(
        es.semantic_projection(),
        final_tree,
        "seed {seed:#x}: redo-all tree mismatch"
    );
}

fn v_of(es: &EditSession, b: ByteOffset) -> VisualPos {
    es.byte_to_visual(b).expect("byte must be editable pos")
}

#[test]
fn property_fixed_seeds_random_edits() {
    for &seed in &SEEDS {
        run_seed(seed);
    }
}
