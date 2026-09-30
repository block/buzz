declare module "qrcode" {
  type ErrorCorrectionLevel = "L" | "M" | "Q" | "H";

  type BitMatrix = {
    get(row: number, column: number): number;
    size: number;
  };

  type QrCode = {
    modules: BitMatrix;
  };

  export function create(
    value: string,
    options?: { errorCorrectionLevel?: ErrorCorrectionLevel },
  ): QrCode;
}
