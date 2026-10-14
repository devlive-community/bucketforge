import { FontAwesomeIcon } from "@fortawesome/react-fontawesome";
import {
  faDownload,
  faFile,
  faLink,
  faPen,
  faTags,
  faUserShield,
  faXmark,
} from "@fortawesome/free-solid-svg-icons";
import type { Entry } from "../types";
import { formatBytes, formatDate, guessType, storageLabel } from "../util";
import { useI18n } from "../i18n";

interface Props {
  entry: Entry;
  onClose: () => void;
  onDownload: (entry: Entry) => void;
  onShare: (entry: Entry) => void;
  onEditType: (entry: Entry) => void;
  onEditTags: (entry: Entry) => void;
  onEditGrants: (entry: Entry) => void;
}

/** 右侧对象详情抽屉。 */
export function FileDetails({
  entry,
  onClose,
  onDownload,
  onShare,
  onEditType,
  onEditTags,
  onEditGrants,
}: Props) {
  const { t } = useI18n();
  // 有真实 Content-Type 就显示它,否则按文件名推测。
  const contentType = entry.content_type ?? guessType(entry.name);
  const rows: [string, string][] = [
    [t("名称"), entry.name],
    [t("路径"), entry.path],
    [t("大小"), formatBytes(entry.size)],
    [t("存储类型"), t(storageLabel(entry.storage_class))],
    [t("修改时间"), formatDate(entry.last_modified)],
    ["ETag", entry.etag ?? "—"],
  ];

  return (
    <div className="details">
      <div className="details__header">
        <span className="details__title">{t("详情")}</span>
        <button className="details__close" onClick={onClose} title={t("关闭")}>
          <FontAwesomeIcon icon={faXmark} />
        </button>
      </div>

      <div className="details__hero">
        <FontAwesomeIcon icon={faFile} className="details__hero-icon" />
        <span className="details__hero-name">{entry.name}</span>
      </div>

      <div className="details__rows">
        {rows.slice(0, 2).map(([label, value]) => (
          <div className="details__row" key={label}>
            <span className="details__label">{label}</span>
            <span className="details__value">{value}</span>
          </div>
        ))}
        <div className="details__row">
          <span className="details__label">{t("类型")}</span>
          <span className="details__value">
            {contentType}
            <button
              className="details__edit"
              title={t("修改内容类型")}
              onClick={() => onEditType(entry)}
            >
              <FontAwesomeIcon icon={faPen} />
            </button>
          </span>
        </div>
        {rows.slice(2).map(([label, value]) => (
          <div className="details__row" key={label}>
            <span className="details__label">{label}</span>
            <span className="details__value">{value}</span>
          </div>
        ))}
      </div>

      <div className="details__actions">
        <button className="btn btn--primary" onClick={() => onDownload(entry)}>
          <FontAwesomeIcon icon={faDownload} /> {t("下载")}
        </button>
        <button className="btn" onClick={() => onShare(entry)}>
          <FontAwesomeIcon icon={faLink} /> {t("分享链接")}
        </button>
        <button className="btn" onClick={() => onEditTags(entry)}>
          <FontAwesomeIcon icon={faTags} /> {t("标签")}
        </button>
        <button className="btn" onClick={() => onEditGrants(entry)}>
          <FontAwesomeIcon icon={faUserShield} /> {t("细粒度授权")}
        </button>
      </div>
    </div>
  );
}
