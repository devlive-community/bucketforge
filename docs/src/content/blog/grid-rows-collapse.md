---
title: 网格视图变成一排排细横条:overflow:hidden 和 CSS Grid 的自动最小高度
date: 2026-10-13
author: BucketForge Team
description: 用户反馈切到网格视图后卡片全变成了细横条。根因不在 JS,而在一行 overflow:hidden——它让网格项的自动最小高度变成 0,条目一多 auto 行就被均分压扁。
tags: ['开发', '前端', 'CSS', 'Bug']
---

GitHub 上有用户反馈:目录里有 1000 个文件,切到网格视图后看不到缩略图,只有一排排细横条;横条还能选中、双击预览、右键,就是看不清。

截图里每条横条之间的间距,刚好是网格的 `gap`(14px)加两条 1px 边框——**卡片还在,只是高度被压成了几乎为 0**。

## 复现条件

网格容器是这样的:

```css
.grid {
  flex: 1;               /* 在纵向 flex 布局里撑满,高度是确定的 */
  overflow-y: auto;
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
  gap: 14px;
  align-content: start;
}

.card {
  overflow: hidden;      /* 让缩略图圆角不溢出 */
  /* ... */
}
```

条目少的时候一切正常,条目一多(一屏放不下)就塌。

## 根因:滚动容器的自动最小尺寸是 0

CSS Grid 在给 `auto` 行定尺寸时,先用每个网格项的**最小贡献值**(minimum contribution)作为行的基础尺寸,再把剩余空间分给各行,直到达到它们的最大内容尺寸。

而规范里有一条:网格项的 `min-height: auto` 解析成"自动最小尺寸",**但如果网格项本身是滚动容器(`overflow` 不是 `visible`),自动最小尺寸就是 0**。`overflow: hidden` 也算滚动容器。

于是:

1. 每张卡片的最小贡献值都是 0,所有 `auto` 行的基础尺寸都是 0;
2. 容器高度是确定的(flex 撑满),剩余空间 = 容器高度 − 所有 gap;
3. 这点空间被平均分给几十行,每行只分到一两个像素。

条目少的时候剩余空间足够大,每行都能长到内容高度,所以看不出问题;条目一多,行数 × gap 吃掉了大部分高度,分到每行的就只剩一条线了。

## 修复:一行

```css
.grid {
  grid-auto-rows: max-content;
}
```

让隐式行直接按内容高度排,不再参与"从 0 开始分剩余空间"的过程。超出容器的部分由 `overflow-y: auto` 正常滚动。卡片上的 `overflow: hidden` 保留,圆角裁剪不受影响。

另一种修法是给卡片加 `min-height: min-content`,把自动最小尺寸覆盖掉;但 `grid-auto-rows` 写在容器上,一处生效,也不怕以后新加的网格项忘了写。

## 同一个 issue 里的另两处"显示不全"

- **详情抽屉底部按钮被截断**:抽屉宽 380px,四个按钮 `flex: 1` 但 `white-space: nowrap`,文字放不下时按钮不会缩,只会把最后一个「细粒度授权」挤出抽屉。改成两列网格(`repeat(2, minmax(0, 1fr))`),按钮再多也只会换行。
- **窄窗口下工具栏右侧按钮被裁掉**:工具栏是单行 flex,窗口窄时「上传」按钮跑出可视区。加上 `flex-wrap: wrap`,放不下就折到第二行。
