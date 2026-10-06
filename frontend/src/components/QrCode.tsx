// SPDX-License-Identifier: AGPL-3.0-or-later
import qrcode from "qrcode-generator";
import { useT } from "../i18n";

/**
 * A join code rendered as a QR.
 *
 * The encoder is `qrcode-generator` (MIT, no dependencies) rather than
 * hand-rolled: a first attempt at implementing ISO 18004 here produced codes
 * that no scanner could read, which is the worst possible failure for a thing
 * whose only job is to be scanned. It runs entirely in the viewer's own
 * browser and sends nothing anywhere.
 *
 * Error correction level M — enough to survive a phone camera at an angle
 * without inflating the code.
 */
export default function QrCode({ value, size = 168, className }: { value: string; size?: number; className?: string }) {
  const { t } = useT();
  const qr = qrcode(0, "M"); // 0 = pick the smallest version that fits
  qr.addData(value);
  qr.make();

  const modules = qr.getModuleCount();
  const quiet = 4;
  const total = modules + quiet * 2;

  const path: string[] = [];
  for (let row = 0; row < modules; row++) {
    for (let col = 0; col < modules; col++) {
      if (qr.isDark(row, col)) path.push(`M${col + quiet},${row + quiet}h1v1h-1z`);
    }
  }

  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${total} ${total}`}
      role="img"
      aria-label={t("common.qrCode.invite")}
      className={className}
      shapeRendering="crispEdges"
    >
      {/* Always black on white: a QR in theme colours is a QR that sometimes
          doesn't scan, and the quiet zone has to be light. */}
      <rect width={total} height={total} fill="#ffffff" />
      <path d={path.join("")} fill="#000000" />
    </svg>
  );
}
