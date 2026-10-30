//! 表格编辑边界：CellSlot 基于 Comrak 解析树而非 split('|')。
//! 物化 synthetic cell 只在第一次确定输入时。
//! 遵循 Comrak 0.55 scanners.re table_cell 规则：奇数反斜杠转义的 | 不分列。
//! 单元格内多行粘贴拒绝。

use std::ops::Range;

use crate::model::{NodeFlags, NodeId, NodeKind};

/// 表格单元槽位。
#[derive(Clone, Debug)]
pub(crate) struct CellSlot {
    pub row: usize,
    pub column: usize,
    pub source: Range<usize>,
    pub insertion: Option<CellInsertion>,
}

#[derive(Clone, Debug)]
pub(crate) struct CellInsertion {
    pub prefix: String, // e.g. " | " or before existing |
    pub suffix: String,
}

pub(crate) fn find_cell_slot(
    table_node: NodeId,
    row: usize,
    col: usize,
    tree_nodes: &[crate::model::TreeNode],
    source: &str,
) -> Option<CellSlot> {
    // 真实：遍历 table->row->cell ，用 sourcepos + flags
    // stub: 简单按 | 数（仅示例，实际用 Comrak 逻辑）
    // 跳过 SYNTHETIC
    if let Some(node) = tree_nodes.get(table_node.0 as usize) {
        if matches!(node.kind, NodeKind::Table { .. }) {
            // TODO: 扫描 source 行，使用 scanners.re 规则解析 cell 边界
            // 奇数 \ 保护 |
            // 这里返回占位
            return Some(CellSlot {
                row,
                column: col,
                source: 0..0,
                insertion: None,
            });
        }
    }
    None
}

/// 第一次输入时物化缺失 cell。
/// 有末尾 | 则在末尾 | 前插入 " | " ；无则追加。
pub(crate) fn materialize_cell_if_needed(
    slot: &CellSlot,
    source: &mut String,
    target_col: usize,
) -> Range<usize> {
    if slot.source.end > 0 && slot.insertion.is_none() {
        return slot.source.clone();
    }
    // stub 物化
    let insert = " | ".to_string();
    let pos = slot.source.end;
    source.insert_str(pos, &insert);
    pos..pos + insert.len()
}

/// 表格内输入裸 | 转义为 \| 。
pub(crate) fn escape_pipe_in_cell(text: &str) -> String {
    text.replace("|", "\\|")
}

/// 拒绝多行粘贴到 cell。
pub(crate) fn validate_cell_paste(text: &str) -> Result<(), &'static str> {
    if text.contains('\n') {
        Err("表格单元格不支持换行；请分次粘贴")
    } else {
        Ok(())
    }
}

/// Tab/Shift+Tab/Enter 导航逻辑 stub。
/// 按行优先；最后移出表格。
pub(crate) fn next_cell(
    current: (usize, usize),
    col_count: usize,
    row_count: usize,
    shift: bool,
    enter: bool,
) -> Option<(usize, usize)> {
    if enter {
        let (r, c) = current;
        if r + 1 < row_count {
            Some((r + 1, c))
        } else {
            None // 移出
        }
    } else if shift {
        let (r, c) = current;
        if c > 0 {
            Some((r, c - 1))
        } else if r > 0 {
            Some((r - 1, col_count - 1))
        } else {
            None
        }
    } else {
        let (r, c) = current;
        if c + 1 < col_count {
            Some((r, c + 1))
        } else if r + 1 < row_count {
            Some((r + 1, 0))
        } else {
            None
        }
    }
}
