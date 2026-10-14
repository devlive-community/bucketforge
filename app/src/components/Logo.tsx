export function Logo({ size = 28 }: { size?: number }) {
  return (
    <img
      src="/logo.png"
      alt="BucketForge"
      width={size}
      height={size}
      draggable={false}
      style={{ display: "block", objectFit: "contain" }}
    />
  );
}
