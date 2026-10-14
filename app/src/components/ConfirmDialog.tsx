import { useI18n } from "../i18n";

interface Props {
  title: string;
  message: string;
  danger?: boolean;
  confirmLabel?: string;
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmDialog({
  title,
  message,
  danger,
  confirmLabel,
  onConfirm,
  onCancel,
}: Props) {
  const { t } = useI18n();
  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <div className="modal modal--sm" onClick={(e) => e.stopPropagation()}>
        <div className="modal__header">
          <h3>{title}</h3>
        </div>
        <div className="modal__body">
          <p className="confirm__message">{message}</p>
        </div>
        <div className="modal__footer">
          <button className="btn" onClick={onCancel}>
            {t("取消")}
          </button>
          <button
            className={`btn ${danger ? "btn--danger" : "btn--primary"}`}
            onClick={onConfirm}
            autoFocus
          >
            {confirmLabel ?? t("确定")}
          </button>
        </div>
      </div>
    </div>
  );
}
