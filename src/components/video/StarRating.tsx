/**
 * 星级评分组件（5 星制，内部值为 0-10，每星 = 2 分）。
 * - 只读/可点两种模式；点击某星 → onChange(星数×2)，点同一星取消评分（0）。
 * - 点击会 stopPropagation，避免触发卡片播放。
 */
interface Props {
  /** 当前评分（0-10） */
  rating: number;
  /** 星星像素大小 */
  size?: number;
  /** 变化回调（0-10）；缺省为只读 */
  onChange?: (rating: number) => void;
  /** 是否显示数值（如 4.5） */
  showValue?: boolean;
  disabled?: boolean;
}

function StarSvg({ size, filled }: { size: number; filled: boolean }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill={filled ? "currentColor" : "none"}
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinejoin="round"
    >
      <path d="M12 2.5l2.95 5.98 6.6.96-4.78 4.66 1.13 6.58L12 17.57l-5.9 3.1 1.13-6.58L2.45 9.44l6.6-.96z" />
    </svg>
  );
}

export default function StarRating({ rating, size = 14, onChange, showValue, disabled }: Props) {
  const readonly = !onChange || disabled;
  const stars = [1, 2, 3, 4, 5];
  return (
    <span className="sr-wrap" style={{ fontSize: size }} onClick={(e) => e.stopPropagation()}>
      {stars.map((i) => {
        const active = rating >= i * 2;
        const isCurrent = rating > 0 && Math.round(rating) === i * 2;
        return (
          <button
            key={i}
            type="button"
            className={`sr-star${active ? " active" : ""}${isCurrent ? " current" : ""}`}
            disabled={readonly}
            title={readonly ? undefined : `${i} 星`}
            onClick={() => {
              if (!onChange) return;
              onChange(rating === i * 2 ? 0 : i * 2);
            }}
          >
            <StarSvg size={size} filled={active} />
          </button>
        );
      })}
      {showValue && rating > 0 && <span className="sr-value">{(rating / 2).toFixed(1)}</span>}
    </span>
  );
}
