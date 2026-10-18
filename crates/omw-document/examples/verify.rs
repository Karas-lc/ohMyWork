//! 人工核验工具（非正式产物）：解析任意 Markdown 文件或 stdin 文本，
//! 打印完整树与字节范围，并验证：
//! 1. 无操作序列化与输入 bytes 逐字节一致（含 BOM/CRLF）；
//! 2. 全部范围有效、不重叠、不越界（validate）。
//!
//! 用法：
//!   cargo run -p omw-document --example verify -- 路径/到/文件.md
//!   printf '文本' | cargo run -p omw-document --example verify
//!
//! 退出码：0 = 通过；1 = 解析失败或校验失败。

use omw_document::parse::parse;
use omw_document::{DocumentSession, NodeFlags, NodeId};
use std::io::Read;
use std::process::ExitCode;

fn main() -> ExitCode {
    // --- 读取输入（文件优先，否则 stdin）---
    let arg = std::env::args().nth(1);
    let (input, label) = match arg {
        Some(path) => match std::fs::read(&path) {
            Ok(bytes) => (bytes, path),
            Err(e) => {
                eprintln!("无法读取 {}: {e}", path);
                return ExitCode::from(1);
            }
        },
        None => {
            let mut buf = Vec::new();
            if std::io::stdin().read_to_end(&mut buf).is_err() {
                eprintln!("stdin 读取失败");
                return ExitCode::from(1);
            }
            (buf, "<stdin>".to_string())
        }
    };

    // --- 解析 ---
    let s = match parse(&input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("解析失败: {e}");
            return ExitCode::from(1);
        }
    };

    println!(
        "== 输入: {label}  编码: {:?}  字节数: {}",
        s.encoding(),
        input.len()
    );
    println!(
        "== 行数: {}  根节点: {:?}",
        s.source_map().line_count(),
        s.tree().root()
    );

    // --- 打印树 ---
    println!("== 树（kind [start,end) flags）==");
    dump(&s, s.tree().root(), 0);

    // --- 校验 1: 无操作序列化逐字节一致 ---
    let roundtrips = s.serialize().as_bytes() == input.as_slice();
    println!(
        "== 无操作往返: {}",
        if roundtrips {
            "V ✓ 逐字节一致"
        } else {
            "X 不一致"
        }
    );

    // --- 校验 2: 范围有效性 ---
    match s.validate() {
        Ok(()) => println!("== 范围校验: V ✓ 全部有效、不重叠、不越界"),
        Err(issues) => {
            println!("== 范围校验: X");
            for iss in &issues {
                println!("   - {iss}");
            }
        }
    }

    if roundtrips && s.validate().is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn dump(s: &DocumentSession, id: NodeId, depth: usize) {
    let n = s.tree().node(id);
    let range = match s.source_range(id) {
        Some(r) => r.to_debug(),
        None => "<none>".to_string(),
    };
    let mut note = String::new();
    if n.flags.contains(NodeFlags::UNATTRIBUTED) {
        note.push_str(" [UNATTRIBUTED]");
    }
    if n.flags.contains(NodeFlags::SYNTHETIC) {
        note.push_str(" [SYNTHETIC]");
    }
    println!(
        "{:indent$}{:?} {}{}",
        "",
        n.kind,
        range,
        note,
        indent = depth * 2
    );
    for c in &n.children {
        dump(s, *c, depth + 1);
    }
}
