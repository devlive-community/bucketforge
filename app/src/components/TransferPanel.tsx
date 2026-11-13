import { FontAwesomeIcon } from "@fortawesome/react-fontawesome";
import {
  faCircleExclamation,
  faRotateRight,
  faXmark,
} from "@fortawesome/free-solid-svg-icons";
import type { TransferItem } from "../types";
import { DELETE_KINDS, formatBytes, formatDuration } from "../util";
import { useI18n } from "../i18n";

/** 进度以字节计的传输种类(可显示速度 / ETA);文件夹类以文件数计,不显示。 */
const BYTE_KINDS = new Set(["上传", "下载", "迁移"]);

/** 失败(可再跑)的状态。 */
const FAILED = new Set(["error", "cancelled", "interrupted"]);

type Translate = (zh: string, params?: Record<string, string | number>) => string;

/**
 * 是否可从面板重跑。迁移需带源端信息(重启后丢失 → 不可重试)。
 * 删除任务不提供面板重试:失败项保留在列表 / 搜索结果里,重新勾选即可。
 */
function isRetryable(i: TransferItem): boolean {
  return (
    i.kind === "上传" ||
    i.kind === "下载" ||
    i.kind === "下载文件夹" ||
    ((i.kind === "迁移" || i.kind === "迁移文件夹") && !!i.srcAccount)
  );
}

/** 删除任务的阶段化文案。返回 null 表示交给通用状态文案。 */
function deleteText(i: TransferItem, t: Translate): string | null {
  if (!DELETE_KINDS.has(i.kind)) return null;

  const failed = i.failed ?? 0;
  const deleted = Math.max(0, i.done - failed);
  const failSuffix = failed > 0 ? ` · ${t("失败 {n}", { n: failed })}` : "";

  if (i.status === "active") {
    if (i.deletePhase === "waiting") {
      return t("等待确认 · 已扫描 {n} 项", { n: i.discovered ?? 0 });
    }
    if (i.cancelling) {
      if (i.deletePhase === "preparing" || !i.deletePhase) {
        return t("正在取消… · 已扫描 {n} 项", { n: i.discovered ?? 0 });
      }
      return t("正在取消… · 已删除 {n}", { n: deleted }) + failSuffix;
    }
    switch (i.deletePhase) {
      case "preparing":
        return t("准备中 · 已扫描 {n} 项", { n: i.discovered ?? 0 });
      case "deleting_files":
        if (i.total > 0) {
          return (
            t("删除文件 {done}/{total}", {
              done: i.phaseDone ?? 0,
              total: i.phaseTotal ?? i.total,
            }) + failSuffix
          );
        }
        return (
          t("已发现 {found} 项 · 已删除 {deleted}", {
            found: i.discovered ?? 0,
            deleted,
          }) + failSuffix
        );
      case "cleaning_directories":
        return (
          t("清理目录占位 {done}/{total}", {
            done: i.phaseDone ?? 0,
            total: i.phaseTotal ?? 0,
          }) + failSuffix
        );
      default:
        return null;
    }
  }

  if (i.status === "done") {
    return t("完成 · 已删除 {n}", { n: deleted });
  }
  if (i.status === "error") {
    if (failed === 0) return t("失败");
    if (deleted === 0) return t("失败 · {n} 项未删除", { n: failed });
    return t("失败 · 已删除 {n}", { n: deleted }) + failSuffix;
  }
  if (i.status === "cancelled") {
    if (deleted === 0 && failed === 0) return t("已取消 · 未删除任何对象");
    return t("已取消 · 已删除 {n}", { n: deleted }) + failSuffix;
  }
  return null;
}

/** 删除任务的活动进度:数字或不定进度;null 表示沿用通用百分比。 */
function deletePercent(i: TransferItem): number | "indeterminate" | null {
  if (!DELETE_KINDS.has(i.kind) || i.status !== "active") return null;
  switch (i.deletePhase) {
    case "preparing":
      return "indeterminate";
    case "waiting":
      return 0;
    case "deleting_files":
      return i.total > 0
        ? Math.min(100, Math.round((i.done / i.total) * 100))
        : "indeterminate";
    case "cleaning_directories":
      if (i.total > 0) {
        return Math.min(100, Math.round((i.done / i.total) * 100));
      }
      return (i.phaseTotal ?? 0) > 0
        ? Math.min(
            100,
            Math.round(((i.phaseDone ?? 0) / (i.phaseTotal ?? 1)) * 100),
          )
        : "indeterminate";
    default:
      return null;
  }
}

interface Props {
  items: TransferItem[];
  onClear: () => void;
  onRetry: (id: string) => void;
  onCancel: (id: string) => void;
  onShowErrors: (item: TransferItem) => void;
}

/** 底部传输任务面板:每个上传 / 下载 / 删除独立显示进度与状态。 */
export function TransferPanel({
  items,
  onClear,
  onRetry,
  onCancel,
  onShowErrors,
}: Props) {
  const { t } = useI18n();
  // 等待用户确认的删除不算「进行中」,但它仍是 active,不能被「清除已完成」清掉。
  const active = items.filter(
    (i) => i.status === "active" && i.deletePhase !== "waiting",
  ).length;
  const hasFinished = items.some((i) => i.status !== "active");
  // 可一键重跑的失败任务(用于「重试全部」)。
  const retryableFailed = items.filter(
    (i) => FAILED.has(i.status) && isRetryable(i),
  );

  return (
    <div className="transfers">
      <div className="transfers__head">
        <span className="transfers__title">
          {t("传输")}
          {active > 0 ? t(" · 进行中 {n}", { n: active }) : ""}
        </span>
        <div className="transfers__actions">
          {retryableFailed.length > 0 && (
            <button
              className="transfers__clear"
              onClick={() => retryableFailed.forEach((i) => onRetry(i.id))}
            >
              {t("重试全部失败 ({n})", { n: retryableFailed.length })}
            </button>
          )}
          <button
            className="transfers__clear"
            onClick={onClear}
            disabled={!hasFinished}
          >
            {t("清除已完成")}
          </button>
        </div>
      </div>
      <div className="transfers__list">
        {items.map((i) => {
          const pct =
            i.status === "done"
              ? 100
              : i.total > 0
                ? Math.round((i.done / i.total) * 100)
                : 0;
          // 进行中的字节类传输:百分比后追加速度与 ETA(如 45% · 3.2 MB/s · ~12s)。
          let generic = `${pct}%`;
          const speed = i.speed ?? 0;
          if (BYTE_KINDS.has(i.kind) && speed > 0) {
            generic += ` · ${formatBytes(speed)}/s`;
            if (i.total > i.done) {
              const eta = formatDuration((i.total - i.done) / speed);
              if (eta) generic += ` · ~${eta}`;
            }
          }

          const deletePct = deletePercent(i);
          const indeterminate = deletePct === "indeterminate";
          const deleteLabel = deleteText(i, t);
          const retryable = isRetryable(i);
          const label =
            deleteLabel ??
            (i.status === "error"
              ? t("失败")
              : i.status === "cancelled"
                ? t("已取消")
                : i.status === "interrupted"
                  ? t("已中断")
                  : i.status === "done"
                    ? i.skipped
                      ? t("已跳过")
                      : t("完成")
                    : i.cancelling
                      ? t("正在取消…")
                      : generic);
          const width =
            i.status === "error"
              ? 100
              : indeterminate
                ? 40
                : typeof deletePct === "number"
                  ? deletePct
                  : pct;
          const canCancel =
            i.status === "active" &&
            !i.cancelling &&
            i.deletePhase !== "waiting";
          const showErrors =
            DELETE_KINDS.has(i.kind) && (i.deleteErrors?.length ?? 0) > 0;

          return (
            <div className="transfers__item" key={i.id}>
              <div className="transfers__row">
                <span className="transfers__kind">{t(i.kind)}</span>
                <span className="transfers__name">{i.name}</span>
                <span
                  className={`transfers__status ${
                    i.status === "error" ? "transfers__status--error" : ""
                  }`}
                >
                  {label}
                </span>
                {canCancel && (
                  <button
                    className="transfers__retry"
                    title={t("取消")}
                    onClick={() => onCancel(i.id)}
                  >
                    <FontAwesomeIcon icon={faXmark} />
                  </button>
                )}
                {showErrors && (
                  <button
                    className="transfers__retry"
                    title={t("查看失败项")}
                    aria-label={t("查看失败项")}
                    onClick={() => onShowErrors(i)}
                  >
                    <FontAwesomeIcon icon={faCircleExclamation} />
                  </button>
                )}
                {(i.status === "error" ||
                  i.status === "cancelled" ||
                  i.status === "interrupted") &&
                  retryable && (
                    <button
                      className="transfers__retry"
                      title={i.status === "error" ? t("重试") : t("继续")}
                      onClick={() => onRetry(i.id)}
                    >
                      <FontAwesomeIcon icon={faRotateRight} />
                    </button>
                  )}
              </div>
              <div className="transfers__track">
                <div
                  className={`transfers__fill ${
                    i.status === "error" ? "transfers__fill--error" : ""
                  } ${indeterminate ? "transfers__fill--indeterminate" : ""}`}
                  style={{ width: `${width}%` }}
                />
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
