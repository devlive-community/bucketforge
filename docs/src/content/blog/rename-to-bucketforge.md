---
title: Nebula 更名 BucketForge:改一个名字,要搬走哪些数据
date: 2026-10-13
author: BucketForge Team
description: 项目从 Nebula 更名为 BucketForge。名字好改,难的是 bundle identifier 一变,应用数据目录、SQLite 库、钥匙串条目全都换了地址——这篇讲老用户升级后账号为什么还在。
tags: ['公告', '开发', '迁移']
---

从 1.14.0 起,Nebula 正式更名为 **BucketForge**。

- 仓库:<https://github.com/devlive-community/bucketforge>(旧地址 GitHub 会自动重定向)
- 官网:<https://bucketforge.devlive.org/>

## 为什么改名

"Nebula" 这个名字太常见了——搜索引擎里有图数据库、有 VPN、有游戏,唯独很难搜到我们。BucketForge 直白地说明了它是什么:一个打理各家云上 **Bucket** 的工具,也和 devlive 社区的 Forge 系列产品保持一致。

## 名字好改,identifier 难改

界面上的名字、README、官网、crate 名(`bucketforge-provider` / `bucketforge-image` / …)都是机械替换。真正需要设计的是 **bundle identifier**:从 `org.devlive.nebula` 改成了 `org.devlive.bucketforge`。

Tauri 按 identifier 决定应用数据放在哪,系统钥匙串里的条目也按服务名区分。identifier 一变,三样东西同时"搬家":

| 数据 | 旧位置 | 新位置 |
|------|--------|--------|
| 数据目录 | `<平台数据目录>/org.devlive.nebula/` | `<平台数据目录>/org.devlive.bucketforge/` |
| SQLite 库 | `nebula.db` | `bucketforge.db` |
| 钥匙串 | 服务 `org.devlive.nebula` / 条目 `__nebula_secrets__` | 服务 `org.devlive.bucketforge` / 条目 `__bucketforge_secrets__` |

如果什么都不做,老用户升级后看到的是一个全新安装:账号、书签、同步任务、AI 抠图插件全没了。

## 迁移:只复制,不删除

启动时在打开数据库**之前**做一次迁移:

```rust
// 更名迁移:旧版(Nebula)数据目录与新目录同级,首次启动复制旧库与插件过来。
if let Some(parent) = dir.parent() {
    let legacy = parent.join(app_core::LEGACY_IDENTIFIER);
    if let Err(e) = app_core::migrate_legacy_data_dir(&legacy, &dir) {
        eprintln!("migrate legacy data failed: {e}");
    }
}
```

Tauri 2 在三个平台上的 `app_data_dir` 都是"平台数据目录 / identifier",所以旧目录就是新目录的兄弟目录,不需要按平台分别拼路径。

几个细节:

- **触发条件**:新库不存在、旧库存在才迁移。迁移过一次之后新库就在了,不会重复执行,也不会覆盖用户在新版本里产生的数据。
- **SQLite 伴生文件**:还没合并进主库的写入可能躺在 `-wal` 文件里,上次异常退出留下的回滚日志在 `-journal` 里。所以 `nebula.db-wal` / `-shm` / `-journal` 要一起复制,并且改成和新主库匹配的名字 `bucketforge.db-wal`,否则最近的改动会丢。
- **复制顺序**:先复制伴生文件和插件目录,**最后**才复制主库。中途失败时新目录里没有主库,下次启动会重新完整迁移一遍,不会留下半套数据。
- **只复制不删除**:旧目录原样保留。万一新版本有问题,用户装回旧版本数据还在;迁移失败也只打一行日志,不阻止新版本启动。

## 钥匙串:读新条目,没有就回退旧条目

密钥不在 SQLite 里,而是在系统钥匙串的一条合并条目里。`KeyringSecrets` 增加了一个可选的"旧条目":

```rust
let secrets = KeyringSecrets::new(KEYRING_SERVICE)
    .with_legacy(legacy::LEGACY_IDENTIFIER, legacy::LEGACY_SECRETS_KEY);
```

首次加载时新条目不存在(`NoEntry`),就去读旧服务名下的旧条目;读到非空内容立即写进新条目,之后就只读新条目了。同样,旧条目不删。

在 macOS 上,读旧条目时系统可能会弹一次钥匙串授权——旧条目是旧 App 创建的,新 App 第一次访问需要用户点"允许"。这只会发生一次。

## 自动更新怎么接上

老版本内置的更新地址是旧仓库的 `releases/latest/download/latest.json`。GitHub 仓库改名后,旧地址会自动重定向到新仓库,所以老用户照常能收到 1.14.0 的更新提示,装上之后再由上面的迁移逻辑接管数据。官网下载页对 1.14.0 之前的版本仍然拼旧的安装包名(`Nebula_x.y.z_*`),历史版本下载不受影响。
