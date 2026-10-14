//! Root-directory adaptation for credentials scoped to one bucket.

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;

use bucketforge_provider::{
    path, ByteStream, CorsRule, Entry, Grant, IncompleteUpload, LifecycleRule, Page, ProgressFn,
    StorageProvider, WebsiteConfig,
};

/// Presents a single known bucket at the provider root without calling ListBuckets.
///
/// Only discovery at the root is redirected. Object, prefix, and bucket-level calls keep
/// the vendor's existing behavior, including its own permission errors.
pub struct PinnedBucketProvider {
    inner: Arc<dyn StorageProvider>,
    bucket: String,
}

impl PinnedBucketProvider {
    pub fn new(inner: Arc<dyn StorageProvider>, bucket: impl Into<String>) -> Self {
        Self {
            inner,
            bucket: bucket.into(),
        }
    }

    fn root_entry(&self) -> Entry {
        Entry::directory(self.bucket.clone())
    }
}

#[async_trait]
impl StorageProvider for PinnedBucketProvider {
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn capabilities(&self) -> bucketforge_provider::Capabilities {
        self.inner.capabilities()
    }

    async fn list(&self, path: &str) -> bucketforge_provider::Result<Vec<Entry>> {
        if path::split(path).0.is_none() {
            return Ok(vec![self.root_entry()]);
        }
        self.inner.list(path).await
    }

    async fn list_page(
        &self,
        path: &str,
        cursor: Option<String>,
    ) -> bucketforge_provider::Result<Page> {
        if path::split(path).0.is_none() {
            return Ok(Page {
                entries: vec![self.root_entry()],
                cursor: None,
            });
        }
        self.inner.list_page(path, cursor).await
    }

    async fn read_stream(
        &self,
        path: &str,
    ) -> bucketforge_provider::Result<(Option<u64>, ByteStream)> {
        self.inner.read_stream(path).await
    }

    async fn read_range(
        &self,
        path: &str,
        offset: u64,
    ) -> bucketforge_provider::Result<(Option<u64>, ByteStream)> {
        self.inner.read_range(path, offset).await
    }

    async fn stat(&self, path: &str) -> bucketforge_provider::Result<Entry> {
        self.inner.stat(path).await
    }

    async fn read(&self, path: &str) -> bucketforge_provider::Result<Bytes> {
        self.inner.read(path).await
    }

    async fn write(
        &self,
        path: &str,
        data: Bytes,
        content_type: Option<&str>,
    ) -> bucketforge_provider::Result<()> {
        self.inner.write(path, data, content_type).await
    }

    async fn write_with_progress(
        &self,
        path: &str,
        data: Bytes,
        content_type: Option<&str>,
        progress: ProgressFn<'_>,
    ) -> bucketforge_provider::Result<()> {
        self.inner
            .write_with_progress(path, data, content_type, progress)
            .await
    }

    async fn write_stream(
        &self,
        path: &str,
        len: Option<u64>,
        stream: ByteStream,
        content_type: Option<&str>,
        progress: ProgressFn<'_>,
    ) -> bucketforge_provider::Result<()> {
        self.inner
            .write_stream(path, len, stream, content_type, progress)
            .await
    }

    async fn begin_multipart(
        &self,
        path: &str,
        content_type: Option<&str>,
    ) -> bucketforge_provider::Result<String> {
        self.inner.begin_multipart(path, content_type).await
    }

    async fn upload_part(
        &self,
        path: &str,
        upload_id: &str,
        part_number: u32,
        data: Bytes,
    ) -> bucketforge_provider::Result<String> {
        self.inner
            .upload_part(path, upload_id, part_number, data)
            .await
    }

    async fn complete_multipart(
        &self,
        path: &str,
        upload_id: &str,
        parts: &[(u32, String)],
    ) -> bucketforge_provider::Result<()> {
        self.inner.complete_multipart(path, upload_id, parts).await
    }

    async fn abort_multipart(
        &self,
        path: &str,
        upload_id: &str,
    ) -> bucketforge_provider::Result<()> {
        self.inner.abort_multipart(path, upload_id).await
    }

    async fn list_incomplete_uploads(
        &self,
        bucket: &str,
    ) -> bucketforge_provider::Result<Vec<IncompleteUpload>> {
        self.inner.list_incomplete_uploads(bucket).await
    }

    async fn set_storage_class(&self, path: &str, class: &str) -> bucketforge_provider::Result<()> {
        self.inner.set_storage_class(path, class).await
    }

    async fn restore(&self, path: &str, days: u32) -> bucketforge_provider::Result<()> {
        self.inner.restore(path, days).await
    }

    async fn set_object_acl(&self, path: &str, public: bool) -> bucketforge_provider::Result<()> {
        self.inner.set_object_acl(path, public).await
    }

    async fn object_grants(&self, path: &str) -> bucketforge_provider::Result<Vec<Grant>> {
        self.inner.object_grants(path).await
    }

    async fn set_object_grants(
        &self,
        path: &str,
        grants: &[Grant],
    ) -> bucketforge_provider::Result<()> {
        self.inner.set_object_grants(path, grants).await
    }

    fn public_url(&self, path: &str) -> Option<String> {
        self.inner.public_url(path)
    }

    async fn set_content_type(
        &self,
        path: &str,
        content_type: &str,
    ) -> bucketforge_provider::Result<()> {
        self.inner.set_content_type(path, content_type).await
    }

    async fn object_tags(&self, path: &str) -> bucketforge_provider::Result<Vec<(String, String)>> {
        self.inner.object_tags(path).await
    }

    async fn set_object_tags(
        &self,
        path: &str,
        tags: &[(String, String)],
    ) -> bucketforge_provider::Result<()> {
        self.inner.set_object_tags(path, tags).await
    }

    async fn create_bucket(&self, bucket: &str) -> bucketforge_provider::Result<()> {
        self.inner.create_bucket(bucket).await
    }

    async fn delete_bucket(&self, bucket: &str) -> bucketforge_provider::Result<()> {
        self.inner.delete_bucket(bucket).await
    }

    async fn bucket_lifecycle(
        &self,
        bucket: &str,
    ) -> bucketforge_provider::Result<Vec<LifecycleRule>> {
        self.inner.bucket_lifecycle(bucket).await
    }

    async fn set_bucket_lifecycle(
        &self,
        bucket: &str,
        rules: &[LifecycleRule],
    ) -> bucketforge_provider::Result<()> {
        self.inner.set_bucket_lifecycle(bucket, rules).await
    }

    async fn bucket_cors(&self, bucket: &str) -> bucketforge_provider::Result<Vec<CorsRule>> {
        self.inner.bucket_cors(bucket).await
    }

    async fn set_bucket_cors(
        &self,
        bucket: &str,
        rules: &[CorsRule],
    ) -> bucketforge_provider::Result<()> {
        self.inner.set_bucket_cors(bucket, rules).await
    }

    async fn bucket_website(
        &self,
        bucket: &str,
    ) -> bucketforge_provider::Result<Option<WebsiteConfig>> {
        self.inner.bucket_website(bucket).await
    }

    async fn set_bucket_website(
        &self,
        bucket: &str,
        config: Option<&WebsiteConfig>,
    ) -> bucketforge_provider::Result<()> {
        self.inner.set_bucket_website(bucket, config).await
    }

    async fn bucket_versioning(&self, bucket: &str) -> bucketforge_provider::Result<bool> {
        self.inner.bucket_versioning(bucket).await
    }

    async fn set_bucket_versioning(
        &self,
        bucket: &str,
        enabled: bool,
    ) -> bucketforge_provider::Result<()> {
        self.inner.set_bucket_versioning(bucket, enabled).await
    }

    async fn list_object_versions(
        &self,
        path: &str,
    ) -> bucketforge_provider::Result<Vec<bucketforge_provider::ObjectVersion>> {
        self.inner.list_object_versions(path).await
    }

    async fn restore_object_version(
        &self,
        path: &str,
        version_id: &str,
    ) -> bucketforge_provider::Result<()> {
        self.inner.restore_object_version(path, version_id).await
    }

    async fn delete_object_version(
        &self,
        path: &str,
        version_id: &str,
    ) -> bucketforge_provider::Result<()> {
        self.inner.delete_object_version(path, version_id).await
    }

    async fn delete(&self, path: &str) -> bucketforge_provider::Result<()> {
        self.inner.delete(path).await
    }

    async fn copy(&self, from: &str, to: &str) -> bucketforge_provider::Result<()> {
        self.inner.copy(from, to).await
    }

    async fn rename(&self, from: &str, to: &str) -> bucketforge_provider::Result<()> {
        self.inner.rename(from, to).await
    }

    async fn presign(&self, path: &str, expires_secs: u64) -> bucketforge_provider::Result<String> {
        self.inner.presign(path, expires_secs).await
    }

    async fn presign_put(
        &self,
        path: &str,
        expires_secs: u64,
    ) -> bucketforge_provider::Result<String> {
        self.inner.presign_put(path, expires_secs).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bucketforge_provider::Capabilities;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordedProvider {
        listed_paths: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl StorageProvider for RecordedProvider {
        fn id(&self) -> &str {
            "recorded"
        }

        fn capabilities(&self) -> Capabilities {
            Capabilities::default()
        }

        async fn list(&self, path: &str) -> bucketforge_provider::Result<Vec<Entry>> {
            self.listed_paths.lock().unwrap().push(path.to_string());
            Ok(vec![Entry::file("real-bucket/object", 1)])
        }

        async fn stat(&self, path: &str) -> bucketforge_provider::Result<Entry> {
            Ok(Entry::file(path, 1))
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
            Ok(())
        }
    }

    fn provider() -> (PinnedBucketProvider, Arc<RecordedProvider>) {
        let inner = Arc::new(RecordedProvider::default());
        let recorded = inner.clone();
        (PinnedBucketProvider::new(inner, "pinned"), recorded)
    }

    #[tokio::test]
    async fn root_listing_is_synthetic_without_list_buckets() {
        let (provider, recorded) = provider();

        let entries = provider.list("").await.unwrap();
        assert_eq!(entries, vec![Entry::directory("pinned")]);

        let page = provider.list_page("/", None).await.unwrap();
        assert_eq!(page.entries, vec![Entry::directory("pinned")]);
        assert_eq!(page.cursor, None);
        assert!(recorded.listed_paths.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn bucket_paths_delegate_without_rewriting() {
        let (provider, recorded) = provider();
        let entries = provider.list("other/objects/").await.unwrap();

        assert_eq!(entries, vec![Entry::file("real-bucket/object", 1)]);
        assert_eq!(
            *recorded.listed_paths.lock().unwrap(),
            vec!["other/objects/".to_string()]
        );
    }
}
