//! 批量删除执行器。
//!
//! 删除是破坏性操作,这里把三个容易出错的边界集中处理:
//!
//! - **默认 fail-closed**:递归删除先扫描到 20,000 条;超过就不删任何对象,由上层让用户
//!   显式选择无上限模式。
//! - **部分失败不谎报成功**:单个对象失败会进入 [`DeleteSummary`],且任一个文件失败时
//!   不删除目录占位对象,避免目录从 UI 消失、文件却残留。
//! - **可取消**:每个批次启动前检查取消标志;已发出的请求允许完成,之后不再调度新批次。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bucketforge_provider::{DeleteItemResult, Entry, ProgressFn, StorageProvider};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};

use crate::{Result, SEARCH_SCAN_LIMIT};

/// 摘要里最多保留多少条错误样例;失败总数仍然完整计数。
const MAX_RECORDED_ERRORS: usize = 5;
/// 每处理多少个对象发一次进度,避免 20k 次 Tauri 事件把 UI 淹掉。
const PROGRESS_STEP: u64 = 50;

/// 一个对象的删除失败。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteFailure {
    pub path: String,
    pub error: String,
}

/// 一次批量删除的最终摘要。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteSummary {
    pub deleted: u64,
    pub failed: u64,
    /// 最多保留 [`MAX_RECORDED_ERRORS`] 条,供 UI 展示。
    pub errors: Vec<DeleteFailure>,
    pub cancelled: bool,
}

impl DeleteSummary {
    fn record(&mut self, result: DeleteItemResult) {
        match result.error {
            Some(error) => self.record_error(result.path, error),
            None => self.deleted += 1,
        }
    }

    fn record_error(&mut self, path: impl Into<String>, error: impl Into<String>) {
        self.failed += 1;
        if self.errors.len() < MAX_RECORDED_ERRORS {
            self.errors.push(DeleteFailure {
                path: path.into(),
                error: error.into(),
            });
        }
    }

    fn merge(&mut self, other: DeleteSummary) {
        self.deleted += other.deleted;
        self.failed += other.failed;
        self.cancelled |= other.cancelled;
        for failure in other.errors {
            if self.errors.len() >= MAX_RECORDED_ERRORS {
                break;
            }
            self.errors.push(failure);
        }
    }

    fn processed(&self) -> u64 {
        self.deleted + self.failed
    }
}

/// 递归删除的结果。超限不是错误,而是需要用户二次确认的状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DeleteFolderOutcome {
    Deleted { summary: DeleteSummary },
    LimitExceeded { limit: usize, at_least: usize },
}

/// 把路径切成 `provider.max_delete_batch()` 大小的批次,按 `concurrency` 个批次并发执行。
///
/// `total` 是全局总数(跨多个阶段复用时保持不变);`progress_offset` 是此前阶段已处理的
/// 数量。`total = None` 表示流式无上限删除,进度分母未知。
pub(crate) async fn run_delete_batches(
    provider: Arc<dyn StorageProvider>,
    paths: Vec<String>,
    total: Option<u64>,
    progress_offset: u64,
    concurrency: usize,
    cancel: Arc<AtomicBool>,
    progress: ProgressFn<'_>,
) -> DeleteSummary {
    let mut summary = DeleteSummary::default();
    if paths.is_empty() {
        progress(progress_offset, total.unwrap_or(0));
        return summary;
    }

    let batch_size = provider.max_delete_batch().max(1);
    let concurrency = concurrency.clamp(1, 10);
    let chunks: Vec<Vec<String>> = paths
        .chunks(batch_size)
        .map(|chunk| chunk.to_vec())
        .collect();

    let mut batches = stream::iter(chunks)
        .map(|chunk| {
            let provider = provider.clone();
            let cancel = cancel.clone();
            async move {
                if cancel.load(Ordering::Relaxed) {
                    None
                } else {
                    Some(provider.delete_batch(&chunk).await)
                }
            }
        })
        .buffer_unordered(concurrency);

    let mut last_report = 0u64;
    while let Some(batch) = batches.next().await {
        let Some(results) = batch else {
            summary.cancelled = true;
            break;
        };
        for result in results {
            summary.record(result);
        }

        let processed = summary.processed();
        let reached_end = total.is_some_and(|total| processed >= total);
        if processed - last_report >= PROGRESS_STEP || reached_end {
            progress(progress_offset + processed, total.unwrap_or(0));
            last_report = processed;
        }
        if cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            break;
        }
    }

    if last_report < summary.processed() {
        progress(progress_offset + summary.processed(), total.unwrap_or(0));
    }
    summary
}

/// 默认递归删除先做一次有界扫描;超过上限时不返回任何可删除对象。
struct WalkOutcome {
    files: Vec<Entry>,
    dirs: Vec<String>,
    truncated: bool,
    cancelled: bool,
}

async fn walk_for_delete(
    provider: &Arc<dyn StorageProvider>,
    root: &str,
    limit: Option<usize>,
    cancel: &AtomicBool,
) -> Result<WalkOutcome> {
    let cap = limit.map(|limit| limit.saturating_add(1));
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut queue = VecDeque::from([root.to_string()]);
    let mut count = 0usize;

    'walk: while let Some(dir) = queue.pop_front() {
        if cancel.load(Ordering::Relaxed) {
            return Ok(WalkOutcome {
                files: Vec::new(),
                dirs: Vec::new(),
                truncated: false,
                cancelled: true,
            });
        }
        let mut cursor = None;
        loop {
            let page = provider.list_page(&dir, cursor).await?;
            for entry in page.entries {
                if cancel.load(Ordering::Relaxed) {
                    return Ok(WalkOutcome {
                        files: Vec::new(),
                        dirs: Vec::new(),
                        truncated: false,
                        cancelled: true,
                    });
                }
                if cap.is_some_and(|cap| count >= cap) {
                    break 'walk;
                }
                count += 1;
                let is_dir = entry.is_dir();
                if is_dir || entry.path.ends_with('/') {
                    dirs.push(entry.path.clone());
                    // 只有真正的目录前缀需要继续下钻;以 `/` 结尾的占位对象只进删除列表,
                    // 否则会在父子列表里重复遍历。
                    if is_dir {
                        queue.push_back(entry.path);
                    }
                } else {
                    files.push(entry);
                }
            }
            match page.cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
    }

    let root_key = root.trim_end_matches('/');
    files.retain(|entry| entry.path.trim_end_matches('/') != root_key);
    dirs.retain(|path| path.trim_end_matches('/') != root_key);
    dirs.sort();
    dirs.dedup();

    Ok(WalkOutcome {
        files,
        dirs,
        truncated: limit.is_some_and(|limit| count > limit),
        cancelled: false,
    })
}

pub(crate) async fn delete_folder(
    provider: Arc<dyn StorageProvider>,
    root: &str,
    allow_unbounded: bool,
    concurrency: usize,
    cancel: Arc<AtomicBool>,
    progress: ProgressFn<'_>,
) -> Result<DeleteFolderOutcome> {
    if allow_unbounded {
        let summary = delete_prefix_streaming(provider, root, concurrency, cancel, progress).await;
        return Ok(DeleteFolderOutcome::Deleted { summary });
    }

    let walk = walk_for_delete(&provider, root, Some(SEARCH_SCAN_LIMIT), &cancel).await?;
    if walk.cancelled {
        return Ok(DeleteFolderOutcome::Deleted {
            summary: DeleteSummary {
                cancelled: true,
                ..DeleteSummary::default()
            },
        });
    }
    if walk.truncated {
        return Ok(DeleteFolderOutcome::LimitExceeded {
            limit: SEARCH_SCAN_LIMIT,
            at_least: SEARCH_SCAN_LIMIT + 1,
        });
    }

    let file_total = walk.files.len() as u64;
    let dir_total = walk.dirs.len() as u64;
    let grand_total = file_total + dir_total + 1;
    let file_paths: Vec<String> = walk.files.into_iter().map(|entry| entry.path).collect();
    let mut summary = run_delete_batches(
        provider.clone(),
        file_paths,
        Some(grand_total),
        0,
        concurrency,
        cancel.clone(),
        progress,
    )
    .await;

    // 文件阶段有失败或取消:保留目录占位,避免目录消失但文件残留。
    if summary.cancelled || summary.failed > 0 {
        return Ok(DeleteFolderOutcome::Deleted { summary });
    }

    let mut dirs = walk.dirs;
    dirs.sort_by_key(|path| std::cmp::Reverse(path.len()));
    let dir_summary = run_delete_batches(
        provider.clone(),
        dirs,
        Some(grand_total),
        summary.processed(),
        concurrency,
        cancel.clone(),
        progress,
    )
    .await;
    summary.merge(dir_summary);

    // 目录占位本身失败时也不删根前缀,保持结构可发现。
    if summary.cancelled || summary.failed > 0 {
        return Ok(DeleteFolderOutcome::Deleted { summary });
    }

    let root_marker = crate::ensure_trailing_slash(root);
    let root_summary = run_delete_batches(
        provider,
        vec![root_marker],
        Some(grand_total),
        summary.processed(),
        1,
        cancel,
        progress,
    )
    .await;
    summary.merge(root_summary);
    Ok(DeleteFolderOutcome::Deleted { summary })
}

/// 无上限模式:逐页列出、逐页删除,不把整个前缀的文件列表装进内存。
async fn delete_prefix_streaming(
    provider: Arc<dyn StorageProvider>,
    root: &str,
    concurrency: usize,
    cancel: Arc<AtomicBool>,
    progress: ProgressFn<'_>,
) -> DeleteSummary {
    let mut summary = DeleteSummary::default();
    let mut dirs = Vec::new();
    let mut queue = VecDeque::from([root.to_string()]);

    'walk: while let Some(dir) = queue.pop_front() {
        if cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            break;
        }
        let mut cursor = None;
        loop {
            if cancel.load(Ordering::Relaxed) {
                summary.cancelled = true;
                break 'walk;
            }
            let page = match provider.list_page(&dir, cursor).await {
                Ok(page) => page,
                Err(error) => {
                    summary.record_error(dir.clone(), error.to_string());
                    break 'walk;
                }
            };
            let mut page_files = Vec::new();
            for entry in page.entries {
                if entry.path.trim_end_matches('/') == root.trim_end_matches('/') {
                    continue;
                }
                let is_dir = entry.is_dir();
                if is_dir || entry.path.ends_with('/') {
                    dirs.push(entry.path.clone());
                    if is_dir {
                        queue.push_back(entry.path);
                    }
                } else {
                    page_files.push(entry.path);
                }
            }
            if !page_files.is_empty() {
                let page_summary = run_delete_batches(
                    provider.clone(),
                    page_files,
                    None,
                    summary.processed(),
                    concurrency,
                    cancel.clone(),
                    progress,
                )
                .await;
                summary.merge(page_summary);
                if summary.cancelled {
                    break 'walk;
                }
            }
            match page.cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
    }

    if summary.cancelled || summary.failed > 0 {
        return summary;
    }

    if !dirs.is_empty() {
        dirs.sort();
        dirs.dedup();
        dirs.sort_by_key(|path| std::cmp::Reverse(path.len()));
        let dir_summary = run_delete_batches(
            provider.clone(),
            dirs,
            None,
            summary.processed(),
            concurrency,
            cancel.clone(),
            progress,
        )
        .await;
        summary.merge(dir_summary);
        if summary.cancelled || summary.failed > 0 {
            return summary;
        }
    }

    let root_marker = crate::ensure_trailing_slash(root);
    let root_summary = run_delete_batches(
        provider,
        vec![root_marker],
        None,
        summary.processed(),
        1,
        cancel,
        progress,
    )
    .await;
    summary.merge(root_summary);
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use bucketforge_provider::{Capabilities, Entry, ProviderError};
    use bytes::Bytes;
    use std::sync::Mutex;

    #[derive(Default)]
    struct BatchProvider {
        calls: Mutex<Vec<usize>>,
    }

    #[async_trait]
    impl StorageProvider for BatchProvider {
        fn id(&self) -> &str {
            "batch"
        }

        fn capabilities(&self) -> Capabilities {
            Capabilities::default()
        }

        async fn list(&self, _path: &str) -> bucketforge_provider::Result<Vec<Entry>> {
            Ok(Vec::new())
        }

        async fn stat(&self, path: &str) -> bucketforge_provider::Result<Entry> {
            Ok(Entry::file(path.to_string(), 0))
        }

        async fn read(&self, _path: &str) -> bucketforge_provider::Result<Bytes> {
            Ok(Bytes::new())
        }

        async fn write(
            &self,
            _path: &str,
            _data: Bytes,
            _content_type: Option<&str>,
        ) -> bucketforge_provider::Result<()> {
            Ok(())
        }

        async fn delete(&self, _path: &str) -> bucketforge_provider::Result<()> {
            Err(ProviderError::Unsupported("single delete".into()))
        }

        fn max_delete_batch(&self) -> usize {
            2
        }

        async fn delete_batch(&self, paths: &[String]) -> Vec<DeleteItemResult> {
            self.calls.lock().unwrap().push(paths.len());
            paths
                .iter()
                .map(|path| DeleteItemResult::deleted(path.clone()))
                .collect()
        }
    }

    #[tokio::test]
    async fn batches_by_provider_limit() {
        let provider = Arc::new(BatchProvider::default());
        let paths: Vec<String> = (0..5).map(|index| format!("b/{index}.txt")).collect();
        let summary = run_delete_batches(
            provider.clone(),
            paths,
            Some(5),
            0,
            3,
            Arc::new(AtomicBool::new(false)),
            &|_, _| {},
        )
        .await;

        assert_eq!(summary.deleted, 5);
        let mut calls = provider.calls.lock().unwrap().clone();
        calls.sort();
        assert_eq!(calls, vec![1, 2, 2]);
    }

    #[tokio::test]
    async fn cancellation_stops_before_any_batch() {
        let provider = Arc::new(BatchProvider::default());
        let summary = run_delete_batches(
            provider.clone(),
            vec!["b/a.txt".to_string()],
            Some(1),
            0,
            3,
            Arc::new(AtomicBool::new(true)),
            &|_, _| {},
        )
        .await;

        assert!(summary.cancelled);
        assert!(provider.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn limit_exceeded_serializes_to_the_frontend_shape() {
        let value = serde_json::to_value(DeleteFolderOutcome::LimitExceeded {
            limit: SEARCH_SCAN_LIMIT,
            at_least: SEARCH_SCAN_LIMIT + 1,
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "status": "limit_exceeded",
                "limit": SEARCH_SCAN_LIMIT,
                "at_least": SEARCH_SCAN_LIMIT + 1,
            })
        );
    }
}
