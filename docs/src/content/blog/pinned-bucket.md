---
title: 指定 bucket:单桶授权的 AK 也能连,靠的是包一层 provider
date: 2026-10-13
author: BucketForge Team
description: 很多团队给工具发的 AK 只授权了一个桶,没有 ListBuckets 权限,一连就报错。这次给账号加了"指定 Bucket":点开账号直达该桶,根目录只显示它——实现上不改任何厂商代码,而是给 provider 包了一层。
tags: ['开发', '对象存储', '权限', '账号']
---

这个需求来自 GitHub issue:「一个账号点开能不能直达某个 bucket,而不是列出全部 bucket?」评论区还补了一个更扎心的场景——「我的 AK 只被授权了其中一个 bucket,但连接无法配置路径 / bucket,无法连接。」功能由社区贡献者在 PR #4 中实现,这篇讲讲它的设计。

## 为什么单桶 AK 会连不上

最小权限是正确的做法:给某个工具发 AK 时,只授权它要用的那一个桶,比如阿里云 RAM 策略里只写 `acs:oss:*:*:my-bucket` 和 `acs:oss:*:*:my-bucket/*`。这样的 AK 能读写桶里的对象,但**列不了账号下有哪些桶**——ListBuckets 是账号级操作,不在授权范围内,直接 403。

而 BucketForge 打开账号的第一件事就是列账号根目录,也就是 ListBuckets。于是这个 AK 在界面上表现为"连不上",哪怕它对目标桶的权限一点问题都没有。

## 指定 Bucket

账号表单里多了一个可选字段「指定 Bucket」。填了之后:

- **点开账号直达该桶**:侧边栏点账号、启动时默认打开、保存账号后、删除当前账号切到下一个,落点都是这个桶的根。
- **桶根就是账号的根**:「上一层」按钮、快捷键、命令面板的"上一层"都在桶根停住;面包屑点到账号根、或书签 / 历史里残留的空路径,也会被拉回桶根。
- **全程不调 ListBuckets**:单桶 AK 从头到尾不会碰到它没权限的接口。

留空就是普通账号,行为和以前完全一样。

## 实现:不改厂商代码,包一层 provider

BucketForge 里每家云都实现同一个 `StorageProvider` trait,App 层只认这个 trait。指定 bucket 的实现没有去动任何一家厂商的代码,而是写了一个通用的包装器:

```rust
/// Presents a single known bucket at the provider root without calling ListBuckets.
pub struct PinnedBucketProvider {
    inner: Arc<dyn StorageProvider>,
    bucket: String,
}
```

它把 `list` / `list_page` 在**根路径**上的调用拦下来,直接返回一条代表该桶的目录条目;其余几十个方法——读写、分片上传、ACL、标签、生命周期、版本……全部原样转发给内层 provider:

```rust
async fn list(&self, path: &str) -> Result<Vec<Entry>> {
    if path::split(path).0.is_none() {
        return Ok(vec![self.root_entry()]);
    }
    self.inner.list(path).await
}
```

注册账号时,有指定 bucket 就把厂商 provider 包一层再放进注册表:

```rust
fn register_pinned(&self, rec: &AccountRecord, provider: Arc<dyn StorageProvider>) {
    let provider: Arc<dyn StorageProvider> = if rec.pinned_bucket.is_empty() {
        provider
    } else {
        Arc::new(PinnedBucketProvider::new(provider, rec.pinned_bucket.clone()))
    };
    self.registry.register(provider);
}
```

这样做的好处是**所有**会列账号根目录的地方——文件浏览、跨账号迁移和复制 / 移动的目标选择——都自动生效,不用在每个调用点加判断;新接入一家云也天然支持。

只有"发现"这一步被改写。其它调用照常发给云厂商,如果 AK 对某个操作确实没权限(比如对这个桶改 CORS),报出来的仍然是厂商原始的权限错误,不会被包装器吞掉或伪装成成功。

## 存储与编辑

- 指定 bucket 存在 `accounts` 表的新列 `pinned_bucket` 上,老库启动时 `ALTER TABLE ... ADD COLUMN` 补列,向后兼容。
- 它不是凭证,不进钥匙串;修改时单独调用 `set_account_pinned_bucket`,并**就地重建**该账号的 provider(包上或拆掉包装器),不用重启。
- 填的值不能包含 `/`——这里只能填桶名,不是路径。
- 编辑账号时,只要 AK 和 Endpoint 没变,密钥就可以留空:改别名、公共域名、指定 bucket 都不用再把 Secret 输一遍。
