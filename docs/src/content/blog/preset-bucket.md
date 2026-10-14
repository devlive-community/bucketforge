---
title: 账号预设 bucket:点开直达,单桶授权的 AK 也能连
date: 2026-10-13
author: BucketForge Team
description: 很多团队给工具发的 AK 只授权了一个桶,没有 ListBuckets 权限,一连就报错。这次给账号加了"预设 bucket":点开账号直达该桶,根目录也只列它,不再去调 ListBuckets。
tags: ['开发', '对象存储', '权限', '账号']
---

这次的需求来自 GitHub issue:「一个账号点开能不能直达某个 bucket,而不是列出全部 bucket?」评论区还补了一个更扎心的场景——「我的 AK 只被授权了其中一个 bucket,但连接无法配置路径 / bucket,无法连接。」

两个问题其实是一回事:BucketForge 打开账号时,默认停在账号根目录,而根目录的内容来自 **ListBuckets**。

## 为什么单桶 AK 会连不上

最小权限是正确的做法:给某个工具发 AK 时,只授权它要用的那一个桶,比如阿里云 RAM 策略里只写 `acs:oss:*:*:my-bucket` 和 `acs:oss:*:*:my-bucket/*`。这样的 AK 能读写桶里的对象,但**列不了账号下有哪些桶**——ListBuckets 是账号级(Service 级)操作,资源不在授权范围内,直接 403。

而旧版打开账号的第一件事就是列桶,于是这个 AK 在界面上表现为"连不上",哪怕它对目标桶的权限一点问题都没有。

## 预设 bucket

账号表单里多了一个可选字段「预设 Bucket」。填了之后:

- **点开账号直达该桶**:侧边栏点账号、命令面板跳账号、启动时默认打开第一个账号,落点都是这个桶,而不是根目录。
- **根目录只列这一个桶,不发请求**:在桶里点「上一层」回到账号根目录时,不去调 ListBuckets,直接返回一条代表该桶的目录条目。单桶 AK 从此全程不会碰到它没权限的接口。

第二点是关键。只做"直达"不够——用户迟早会点「上一层」,或者在迁移 / 复制移动对话框里选目标位置,这些地方都会去列账号根目录。所以短路放在了最底下的业务层 `app-core`,而不是前端:

```rust
/// 配了预设 bucket 的账号在根目录只列这一个桶,不调 ListBuckets——只授权了单个桶的
/// AK 没有 ListBuckets 权限,直接列会失败。
fn preset_root(&self, account: &str, path: &str) -> Option<Vec<Entry>> {
    if !path.trim_matches('/').is_empty() {
        return None;
    }
    Some(vec![Entry::directory(self.default_bucket(account)?)])
}
```

`browse` 和分页的 `browse_page` 都先过这一关。所有"列账号根目录"的入口——文件浏览、跨账号迁移的目标选择、复制 / 移动的目标选择——都走这两个方法,一处改动全部生效,前端一行判断都不用加。

## 存储:一列,和公共域名同一套路

预设 bucket 存在 `accounts` 表的新列 `default_bucket` 上,老库启动时 `ALTER TABLE ... ADD COLUMN` 补列(已存在就忽略),和之前的「自定义公共域名」完全同一套路:

- 它不是凭证,所以**不进钥匙串**,放 SQLite;
- 重新保存凭证(`upsert`)时**不覆盖**它,单独用 `set_default_bucket` 更新——否则改一次密钥,预设 bucket 就被清空了;
- 用户可能填成 `/my-bucket/` 或带空格,入库前统一去掉首尾的 `/` 和空白。

测试里用一个**不可达的 endpoint**(`127.0.0.1:1`)注册账号,再去列根目录:如果代码真的发了 ListBuckets 请求就会报错,而预设 bucket 应该原样返回那一个桶——这样就能离线证明"根目录不会发请求"。

## 顺手修的:改个设置不必再输一遍密钥

以前编辑账号时,只有"纯改别名"可以不填密钥,改公共域名也得把 Secret 重新输一遍。现在规则改成:**AK 和 Endpoint 都没变,密钥就可以留空**——别名、公共域名、预设 bucket 随便改,密钥只在真的要换凭证时才需要。
