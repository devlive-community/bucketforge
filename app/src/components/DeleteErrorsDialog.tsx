import type { DeleteFailure } from "../types";
import { useI18n } from "../i18n";

interface Props {
  name: string;
  errors: DeleteFailure[];
  onClose: () => void;
}

/** 删除结束后的失败样例。摘要只保留前 5 条,完整失败数见传输行。 */
export function DeleteErrorsDialog({ name, errors, onClose }: Props) {
  const { t } = useI18n();
  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal__header">
          <h3>
            {t("删除失败项")} · {name}
          </h3>
        </div>
        <div className="modal__body">
          <p className="confirm__message">
            {t("最多显示前 5 条失败记录。")}
          </p>
          <ul className="delete-errors">
            {errors.map((failure, index) => (
              <li
                key={`${failure.path}:${index}`}
                className="delete-errors__item"
              >
                <span className="delete-errors__path">{failure.path}</span>
                <span className="delete-errors__error">{failure.error}</span>
              </li>
            ))}
          </ul>
        </div>
        <div className="modal__footer">
          <button className="btn" onClick={onClose}>
            {t("关闭")}
          </button>
        </div>
      </div>
    </div>
  );
}
