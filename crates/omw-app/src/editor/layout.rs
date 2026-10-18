use omw_document::NodeId;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LayoutKey {
    pub node: NodeId,
    pub content_version: u64,
    pub width_bits: u32,
    pub font_version: u64,
    pub theme_version: u64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockLayout {
    pub top: f32,
    pub height: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub top: f32,
    pub height: f32,
    pub overscan: f32,
}
impl Viewport {
    pub fn visible_range(self, blocks: &[BlockLayout]) -> std::ops::Range<usize> {
        let low = (self.top - self.overscan).max(0.0);
        let high = self.top + self.height + self.overscan;
        let first = blocks.partition_point(|b| b.top + b.height < low);
        let end = blocks.partition_point(|b| b.top <= high);
        first..end
    }
}
#[derive(Default)]
pub struct LayoutCache {
    entries: HashMap<LayoutKey, BlockLayout>,
    prefix: Vec<f32>,
}
impl LayoutCache {
    pub fn insert(&mut self, key: LayoutKey, layout: BlockLayout) {
        self.entries.insert(key, layout);
    }
    pub fn get(&self, key: &LayoutKey) -> Option<BlockLayout> {
        self.entries.get(key).copied()
    }
    pub fn rebuild_prefix(&mut self, heights: impl IntoIterator<Item = f32>) {
        self.prefix.clear();
        self.prefix.push(0.0);
        for height in heights {
            self.prefix
                .push(self.prefix.last().copied().unwrap_or(0.0) + height.max(0.0));
        }
    }
    pub fn total_height(&self) -> f32 {
        self.prefix.last().copied().unwrap_or(0.0)
    }
    pub fn block_at_y(&self, y: f32) -> Option<usize> {
        if self.prefix.len() < 2 || y < 0.0 || y >= self.total_height() {
            return None;
        }
        Some(
            self.prefix
                .partition_point(|&top| top <= y)
                .saturating_sub(1),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prefix_lookup_and_viewport_skip_offscreen_blocks() {
        let blocks: Vec<_> = (0..100_000)
            .map(|i| BlockLayout {
                top: i as f32 * 10.0,
                height: 10.0,
            })
            .collect();
        assert_eq!(
            Viewport {
                top: 400.0,
                height: 20.0,
                overscan: 10.0
            }
            .visible_range(&blocks),
            38..44
        );
        let mut cache = LayoutCache::default();
        cache.rebuild_prefix([10.0, 20.0]);
        assert_eq!(cache.block_at_y(15.0), Some(1));
        assert_eq!(cache.total_height(), 30.0);
    }
}
