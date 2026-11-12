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

use bucketforge_provider::{DeleteItemResult, Entry, StorageProvider};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};

use crate::{Result, SEARCH_SCAN_LIMIT};

/// 摘要里最多保留多少条错误样例;失败总数仍然完整计数。
const MAX_RECORDED_ERRORS: usize = 5;

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

    pub(crate) fn processed(&self) -> u64 {
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

/// 删除过程的阶段化进度事件。
///
/// 删除不是单一计数:有界递归删除先扫描,再删文件,最后清理目录占位;无上限删除则边发现
/// 边删。阶段用 tag 区分,避免用 `total = 0` 同时表示“未知”和“不适用”。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum DeleteProgressEvent {
    Preparing {
        /// 已扫描的条目数(文件、目录项与目录占位对象)。
        scanned: u64,
    },
    DeletingFiles {
        /// 累计已处理(成功 + 失败),用于总进度条。
        done: u64,
        /// 总项数(文件 + 目录占位 + 根占位);`None` 表示无上限流式删除。
        total: Option<u64>,
        /// 当前阶段已处理。
        phase_done: u64,
        /// 当前阶段总数;`None` 表示无上限流式删除。
        phase_total: Option<u64>,
        /// 流式删除中已发现的条目数。
        discovered: u64,
        /// 累计失败数。
        failed: u64,
    },
    CleaningDirectories {
        /// 累计已处理(成功 + 失败),用于总进度条。
        done: u64,
        /// 总项数(文件 + 目录占位 + 根占位);`None` 表示无上限流式删除。
        total: Option<u64>,
        /// 当前阶段已处理(含根目录占位)。
        phase_done: u64,
        /// 当前阶段总数(含根目录占位)。
        phase_total: u64,
        /// 累计失败数。
        failed: u64,
    },
}

impl DeleteProgressEvent {
    /// 阶段标识。节流器用它判断是否必须立即送达(阶段切换不能丢)。
    pub fn phase(&self) -> &'static str {
        match self {
            Self::Preparing { .. } => "preparing",
            Self::DeletingFiles { .. } => "deleting_files",
            Self::CleaningDirectories { .. } => "cleaning_directories",
        }
    }
}

pub type DeleteProgressFn<'a> = &'a (dyn Fn(DeleteProgressEvent) + Send + Sync);

/// 把路径切成 `provider.max_delete_batch()` 大小的批次,按 `concurrency` 个批次并发执行。
///
/// 每完成一个批次回调一次 `report`,由调用方把本阶段的累计结果映射成
/// [`DeleteProgressEvent`]。节流在 Tauri 适配层做,核心层只在自然批次边界报告。
pub(crate) async fn run_delete_batches(
    provider: Arc<dyn StorageProvider>,
    paths: Vec<String>,
    concurrency: usize,
    cancel: Arc<AtomicBool>,
    mut report: impl FnMut(&DeleteSummary) + Send,
) -> DeleteSummary {
    let mut summary = DeleteSummary::default();
    report(&summary);
    if paths.is_empty() {
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

    while let Some(batch) = batches.next().await {
        let Some(results) = batch else {
            summary.cancelled = true;
            break;
        };
        for result in results {
            summary.record(result);
        }
        report(&summary);
        if cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            break;
        }
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
    progress: DeleteProgressFn<'_>,
) -> Result<WalkOutcome> {
    let cap = limit.map(|limit| limit.saturating_add(1));
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut queue = VecDeque::from([root.to_string()]);
    let mut count = 0u64;

    progress(DeleteProgressEvent::Preparing { scanned: 0 });

    'walk: while let Some(dir) = queue.pop_front() {
        if cancel.load(Ordering::Relaxed) {
            progress(DeleteProgressEvent::Preparing { scanned: count });
            return Ok(WalkOutcome {
                files: Vec::new(),
                dirs: Vec::new(),
                truncated: false,
                cancelled: true,
            });
        }
        let mut cursor = None;
        loop {
            let page = match provider.list_page(&dir, cursor).await {
                Ok(page) => page,
                Err(error) => {
                    progress(DeleteProgressEvent::Preparing { scanned: count });
                    return Err(error.into());
                }
            };
            for entry in page.entries {
                if cancel.load(Ordering::Relaxed) {
                    progress(DeleteProgressEvent::Preparing { scanned: count });
                    return Ok(WalkOutcome {
                        files: Vec::new(),
                        dirs: Vec::new(),
                        truncated: false,
                        cancelled: true,
                    });
                }
                if cap.is_some_and(|cap| count >= cap as u64) {
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
            progress(DeleteProgressEvent::Preparing { scanned: count });
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

    progress(DeleteProgressEvent::Preparing { scanned: count });

    Ok(WalkOutcome {
        files,
        dirs,
        truncated: limit.is_some_and(|limit| count > limit as u64),
        cancelled: false,
    })
}

pub(crate) async fn delete_folder(
    provider: Arc<dyn StorageProvider>,
    root: &str,
    allow_unbounded: bool,
    concurrency: usize,
    cancel: Arc<AtomicBool>,
    progress: DeleteProgressFn<'_>,
) -> Result<DeleteFolderOutcome> {
    if allow_unbounded {
        let summary = delete_prefix_streaming(provider, root, concurrency, cancel, progress).await;
        return Ok(DeleteFolderOutcome::Deleted { summary });
    }

    let walk = walk_for_delete(&provider, root, Some(SEARCH_SCAN_LIMIT), &cancel, progress).await?;
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
    let file_summary = run_delete_batches(
        provider.clone(),
        file_paths,
        concurrency,
        cancel.clone(),
        |run_summary| {
            progress(DeleteProgressEvent::DeletingFiles {
                done: run_summary.processed(),
                total: Some(grand_total),
                phase_done: run_summary.processed(),
                phase_total: Some(file_total),
                discovered: 0,
                failed: run_summary.failed,
            });
        },
    )
    .await;

    // 文件阶段有失败或取消:保留目录占位,避免目录消失但文件残留。
    if file_summary.cancelled || file_summary.failed > 0 {
        return Ok(DeleteFolderOutcome::Deleted {
            summary: file_summary,
        });
    }

    let file_done = file_summary.processed();
    let file_failed = file_summary.failed;
    let mut summary = file_summary;
    let mut dirs = walk.dirs;
    dirs.sort_by_key(|path| std::cmp::Reverse(path.len()));
    let dir_phase_total = dir_total + 1;
    let dir_summary = run_delete_batches(
        provider.clone(),
        dirs,
        concurrency,
        cancel.clone(),
        |run_summary| {
            progress(DeleteProgressEvent::CleaningDirectories {
                done: file_done + run_summary.processed(),
                total: Some(grand_total),
                phase_done: run_summary.processed(),
                phase_total: dir_phase_total,
                failed: file_failed + run_summary.failed,
            });
        },
    )
    .await;
    summary.merge(dir_summary);

    // 目录占位本身失败时也不删根前缀,保持结构可发现。
    if summary.cancelled || summary.failed > 0 {
        return Ok(DeleteFolderOutcome::Deleted { summary });
    }

    let root_marker = crate::ensure_trailing_slash(root);
    let root_done_base = summary.processed();
    let root_failed_base = summary.failed;
    let root_summary = run_delete_batches(provider, vec![root_marker], 1, cancel, |run_summary| {
        progress(DeleteProgressEvent::CleaningDirectories {
            done: root_done_base + run_summary.processed(),
            total: Some(grand_total),
            phase_done: dir_total + run_summary.processed(),
            phase_total: dir_phase_total,
            failed: root_failed_base + run_summary.failed,
        });
    })
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
    progress: DeleteProgressFn<'_>,
) -> DeleteSummary {
    let mut summary = DeleteSummary::default();
    let mut dirs = Vec::new();
    let mut queue = VecDeque::from([root.to_string()]);
    let mut discovered = 0u64;

    let report_streaming = |summary: &DeleteSummary, discovered: u64| {
        progress(DeleteProgressEvent::DeletingFiles {
            done: summary.processed(),
            total: None,
            phase_done: summary.processed(),
            phase_total: None,
            discovered,
            failed: summary.failed,
        });
    };

    report_streaming(&summary, discovered);

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
                discovered += 1;
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
                let base_done = summary.processed();
                let base_failed = summary.failed;
                let page_discovered = discovered;
                let page_summary = run_delete_batches(
                    provider.clone(),
                    page_files,
                    concurrency,
                    cancel.clone(),
                    |run_summary| {
                        progress(DeleteProgressEvent::DeletingFiles {
                            done: base_done + run_summary.processed(),
                            total: None,
                            phase_done: base_done + run_summary.processed(),
                            phase_total: None,
                            discovered: page_discovered,
                            failed: base_failed + run_summary.failed,
                        });
                    },
                )
                .await;
                summary.merge(page_summary);
                if summary.cancelled {
                    break 'walk;
                }
            } else {
                // 这一页只有目录:仍把发现数推进到 UI。
                report_streaming(&summary, discovered);
            }
            match page.cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
    }

    report_streaming(&summary, discovered);

    if summary.cancelled || summary.failed > 0 {
        return summary;
    }

    dirs.sort();
    dirs.dedup();
    dirs.sort_by_key(|path| std::cmp::Reverse(path.len()));
    let dir_total = dirs.len() as u64;
    let dir_phase_total = dir_total + 1;

    progress(DeleteProgressEvent::CleaningDirectories {
        done: summary.processed(),
        total: None,
        phase_done: 0,
        phase_total: dir_phase_total,
        failed: summary.failed,
    });

    let dir_done_base = summary.processed();
    let dir_failed_base = summary.failed;
    let dir_summary = run_delete_batches(
        provider.clone(),
        dirs,
        concurrency,
        cancel.clone(),
        |run_summary| {
            progress(DeleteProgressEvent::CleaningDirectories {
                done: dir_done_base + run_summary.processed(),
                total: None,
                phase_done: run_summary.processed(),
                phase_total: dir_phase_total,
                failed: dir_failed_base + run_summary.failed,
            });
        },
    )
    .await;
    summary.merge(dir_summary);
    if summary.cancelled || summary.failed > 0 {
        return summary;
    }

    let root_done_base = summary.processed();
    let root_failed_base = summary.failed;
    let root_marker = crate::ensure_trailing_slash(root);
    let root_summary = run_delete_batches(provider, vec![root_marker], 1, cancel, |run_summary| {
        progress(DeleteProgressEvent::CleaningDirectories {
            done: root_done_base + run_summary.processed(),
            total: None,
            phase_done: dir_total + run_summary.processed(),
            phase_total: dir_phase_total,
            failed: root_failed_base + run_summary.failed,
        });
    })
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
            3,
            Arc::new(AtomicBool::new(false)),
            |_| {},
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
            3,
            Arc::new(AtomicBool::new(true)),
            |_| {},
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

    #[test]
    fn progress_events_serialize_with_phase_tag() {
        assert_eq!(
            serde_json::to_value(DeleteProgressEvent::Preparing { scanned: 3 }).unwrap(),
            serde_json::json!({ "phase": "preparing", "scanned": 3 })
        );
        assert_eq!(
            serde_json::to_value(DeleteProgressEvent::DeletingFiles {
                done: 4,
                total: Some(10),
                phase_done: 4,
                phase_total: Some(10),
                discovered: 0,
                failed: 1,
            })
            .unwrap(),
            serde_json::json!({
                "phase": "deleting_files",
                "done": 4,
                "total": 10,
                "phase_done": 4,
                "phase_total": 10,
                "discovered": 0,
                "failed": 1,
            })
        );
        assert_eq!(
            serde_json::to_value(DeleteProgressEvent::CleaningDirectories {
                done: 10,
                total: None,
                phase_done: 2,
                phase_total: 3,
                failed: 0,
            })
            .unwrap(),
            serde_json::json!({
                "phase": "cleaning_directories",
                "done": 10,
                "total": null,
                "phase_done": 2,
                "phase_total": 3,
                "failed": 0,
            })
        );
    }

    #[tokio::test]
    async fn batch_progress_reports_initial_and_final_counts() {
        let provider = Arc::new(BatchProvider::default());
        let paths: Vec<String> = (0..5).map(|index| format!("b/{index}.txt")).collect();
        let reports = Arc::new(Mutex::new(Vec::new()));
        let sink = reports.clone();

        let summary = run_delete_batches(
            provider,
            paths,
            3,
            Arc::new(AtomicBool::new(false)),
            move |current| sink.lock().unwrap().push(current.processed()),
        )
        .await;

        assert_eq!(summary.deleted, 5);
        let reports = reports.lock().unwrap();
        assert_eq!(reports.first(), Some(&0));
        assert_eq!(reports.last(), Some(&5));
    }
}
