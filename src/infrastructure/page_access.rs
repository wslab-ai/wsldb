use std::sync::Arc;

use super::page::Node;
use crate::Result;
use crate::domain::PageId;

pub(crate) trait PageReader {
    fn read_node(&self, page_id: PageId) -> Result<Arc<Node>>;

    fn inspect_node<T>(
        &self,
        page_id: PageId,
        inspect: impl FnOnce(&Node) -> Result<T>,
    ) -> Result<T> {
        let node = self.read_node(page_id)?;
        inspect(&node)
    }
}

pub(crate) trait PageWriter: PageReader {
    fn write_node(&mut self, node: &Node) -> Result<PageId>;

    fn write_hot_node(&mut self, node: &Node) -> Result<PageId> {
        self.write_node(node)
    }

    fn replace_node(&mut self, _page_id: PageId, node: &Node) -> Result<PageId> {
        self.write_node(node)
    }
}
