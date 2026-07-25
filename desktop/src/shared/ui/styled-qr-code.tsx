import { useId, useMemo, type ReactNode, type SVGProps } from "react";
import { create } from "qrcode";

const CELL_SPACING_RATIO = 0.2;
const FINDER_PATTERN_SIZE = 7;
const QUIET_ZONE_SIZE = 4;
const MAX_CENTER_OBSCURED_RATIO = 0.1;
const CENTER_ICON_SIZE_RATIO = 0.8;

type StyledQrCodeProps = Omit<
  SVGProps<SVGSVGElement>,
  "children" | "height" | "title" | "width"
> & {
  backgroundColor?: string;
  centerImageSrc?: string;
  foregroundColor?: string;
  size?: number;
  title?: string;
  value: string;
};

function isFinderCell(row: number, column: number, matrixSize: number) {
  const isTop = row < FINDER_PATTERN_SIZE;
  const isLeft = column < FINDER_PATTERN_SIZE;
  const isRight = column >= matrixSize - FINDER_PATTERN_SIZE;
  const isBottom = row >= matrixSize - FINDER_PATTERN_SIZE;

  return (isTop && isLeft) || (isTop && isRight) || (isBottom && isLeft);
}

function getCenterAreaSize(matrixSize: number) {
  const maxArea = matrixSize * matrixSize * MAX_CENTER_OBSCURED_RATIO;
  const maxSide = Math.floor(Math.sqrt(maxArea));
  return maxSide % 2 === 0 ? maxSide - 1 : maxSide;
}

function FinderPattern({
  backgroundColor,
  foregroundColor,
  x,
  y,
}: {
  backgroundColor: string;
  foregroundColor: string;
  x: number;
  y: number;
}) {
  return (
    <g data-qr-finder-pattern="">
      <rect
        fill={foregroundColor}
        height={FINDER_PATTERN_SIZE}
        rx={2}
        width={FINDER_PATTERN_SIZE}
        x={x}
        y={y}
      />
      <rect
        fill={backgroundColor}
        height={FINDER_PATTERN_SIZE - 2}
        rx={0.8}
        width={FINDER_PATTERN_SIZE - 2}
        x={x + 1}
        y={y + 1}
      />
      <rect
        fill={foregroundColor}
        height={FINDER_PATTERN_SIZE - 4}
        rx={0.4}
        width={FINDER_PATTERN_SIZE - 4}
        x={x + 2}
        y={y + 2}
      />
    </g>
  );
}

export function StyledQrCode({
  backgroundColor = "#ffffff",
  centerImageSrc,
  foregroundColor = "#000000",
  size = 240,
  title = "QR code",
  value,
  ...svgProps
}: StyledQrCodeProps) {
  const clipId = `styled-qr-logo-${useId().replace(/[^a-zA-Z0-9_-]/g, "")}`;
  const matrix = useMemo(
    () => create(value, { errorCorrectionLevel: "M" }).modules,
    [value],
  );
  const viewBoxSize = matrix.size + QUIET_ZONE_SIZE * 2;
  const dataCellSize = 1 - CELL_SPACING_RATIO;
  const dataCellRadius = dataCellSize / 2;
  const centerAreaSize = centerImageSrc ? getCenterAreaSize(matrix.size) : 0;
  const centerAreaStart = (matrix.size - centerAreaSize) / 2;
  const centerAreaEnd = centerAreaStart + centerAreaSize;
  const centerImageSize = centerAreaSize * CENTER_ICON_SIZE_RATIO;
  const centerImageStart = (matrix.size - centerImageSize) / 2;
  const cells: ReactNode[] = [];

  for (let row = 0; row < matrix.size; row += 1) {
    for (let column = 0; column < matrix.size; column += 1) {
      const isCenterCell =
        centerImageSrc &&
        row >= centerAreaStart &&
        row < centerAreaEnd &&
        column >= centerAreaStart &&
        column < centerAreaEnd;

      if (
        !matrix.get(row, column) ||
        isFinderCell(row, column, matrix.size) ||
        isCenterCell
      ) {
        continue;
      }

      cells.push(
        <circle
          cx={column + 0.5}
          cy={row + 0.5}
          fill={foregroundColor}
          key={`${row}-${column}`}
          r={dataCellRadius}
        />,
      );
    }
  }

  return (
    <svg
      {...svgProps}
      aria-label={title}
      data-qr-matrix-size={matrix.size}
      height={size}
      role="img"
      viewBox={`${-QUIET_ZONE_SIZE} ${-QUIET_ZONE_SIZE} ${viewBoxSize} ${viewBoxSize}`}
      width={size}
      xmlns="http://www.w3.org/2000/svg"
    >
      <title>{title}</title>
      <rect
        fill={backgroundColor}
        height={viewBoxSize}
        width={viewBoxSize}
        x={-QUIET_ZONE_SIZE}
        y={-QUIET_ZONE_SIZE}
      />
      <g data-qr-data-cells="">{cells}</g>
      <FinderPattern
        backgroundColor={backgroundColor}
        foregroundColor={foregroundColor}
        x={0}
        y={0}
      />
      <FinderPattern
        backgroundColor={backgroundColor}
        foregroundColor={foregroundColor}
        x={matrix.size - FINDER_PATTERN_SIZE}
        y={0}
      />
      <FinderPattern
        backgroundColor={backgroundColor}
        foregroundColor={foregroundColor}
        x={0}
        y={matrix.size - FINDER_PATTERN_SIZE}
      />
      {centerImageSrc ? (
        <>
          <defs>
            <clipPath id={clipId}>
              <rect
                height={centerImageSize}
                rx={2}
                width={centerImageSize}
                x={centerImageStart}
                y={centerImageStart}
              />
            </clipPath>
          </defs>
          <rect
            fill={foregroundColor}
            height={centerImageSize}
            rx={2}
            width={centerImageSize}
            x={centerImageStart}
            y={centerImageStart}
          />
          <image
            clipPath={`url(#${clipId})`}
            height={centerImageSize}
            href={centerImageSrc}
            preserveAspectRatio="xMidYMid meet"
            width={centerImageSize}
            x={centerImageStart}
            y={centerImageStart}
          />
        </>
      ) : null}
    </svg>
  );
}
