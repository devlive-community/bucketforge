//! 批量删除的结果模型。
//!
//! 批量删除是"尽力而为"的:单批请求成功不代表每个 key 都成功,厂商可能在响应里逐 key
//! 返回错误。适配层把两种失败都归一化成 [`DeleteItemResult::error`],由上层汇总成删除摘要。

use serde::{Deserialize, Serialize};

/// 一个对象的删除结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteItemResult {
    pub path: String,
    /// `None` 表示删除成功;`Some` 是面向用户的错误文本。
    pub error: Option<String>,
}

impl DeleteItemResult {
    pub fn deleted(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            error: None,
        }
    }

    pub fn failed(path: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            error: Some(error.into()),
        }
    }

    pub fn is_ok(&self) -> bool {
        self.error.is_none()
    }
}

/// 把一组 `bucket/key` 路径按桶分组,调用厂商客户端的 `delete_objects`,再映射回
/// [`DeleteItemResult`]。
///
/// 客户端需提供 `delete_objects(bucket, keys) -> Result<Vec<{key, code, message}>>`。
/// 批量删除是**按桶**的,一次请求不能跨桶;某个桶的整批请求失败时,该桶下的所有路径
/// 都记为同一条错误。共享 `s3-core` 的适配层与阿里 / 华为 / 腾讯的专有客户端都复用这个宏。
#[macro_export]
macro_rules! delete_batch_multi {
    ($client:expr, $paths:expr) => {{
        use std::collections::HashMap;

        let mut by_bucket: HashMap<String, Vec<(String, String)>> = HashMap::new();
        let mut errors: HashMap<String, String> = HashMap::new();
        for path in $paths {
            match $crate::path::split(path) {
                (Some(bucket), key) if !key.is_empty() => {
                    by_bucket
                        .entry(bucket.to_string())
                        .or_default()
                        .push((path.clone(), key.to_string()));
                }
                _ => {
                    errors.insert(path.clone(), "无效的对象路径".to_string());
                }
            }
        }

        for (bucket, items) in by_bucket {
            let keys: Vec<String> = items.iter().map(|(_, key)| key.clone()).collect();
            match $client.delete_objects(&bucket, &keys).await {
                Ok(failures) => {
                    let mut failures_by_key: HashMap<String, String> = failures
                        .into_iter()
                        .map(|f| (f.key, format!("{}: {}", f.code, f.message)))
                        .collect();
                    for (path, key) in items {
                        if let Some(error) = failures_by_key.remove(&key) {
                            errors.insert(path, error);
                        }
                    }
                }
                Err(e) => {
                    let message = e.to_string();
                    for (path, _) in items {
                        errors.insert(path, message.clone());
                    }
                }
            }
        }

        $paths
            .iter()
            .map(|path| match errors.remove(path) {
                Some(error) => $crate::DeleteItemResult::failed(path.clone(), error),
                None => $crate::DeleteItemResult::deleted(path.clone()),
            })
            .collect::<Vec<_>>()
    }};
}
